use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::{Path, PathBuf},
};

use gpui_kit::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, SharedString, Styled as _, Subscription, Window,
    component::{
        list::ListItem,
        tree::{Tree, TreeEvent, TreeItem, TreeState},
    },
    div, px,
};
use omw_local::{
    FileOperationError, ScanEntry, ScanOptions, create_directory, create_markdown, move_entry,
    rename_entry, reveal_in_system, scan_directory_children,
};

const ROOT_ID: &str = "";
const LOADING_SUFFIX: &str = "/.ohmywork-loading";

#[derive(Debug)]
pub enum FileTreeError {
    Operation(FileOperationError),
    Reveal(io::Error),
}

impl std::fmt::Display for FileTreeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Operation(error) => error.fmt(f),
            Self::Reveal(error) => write!(f, "在系统中显示失败：{error}"),
        }
    }
}

impl std::error::Error for FileTreeError {}

impl From<FileOperationError> for FileTreeError {
    fn from(value: FileOperationError) -> Self {
        Self::Operation(value)
    }
}

pub struct FileTreeView {
    root: PathBuf,
    tree: Entity<TreeState>,
    entries: BTreeMap<PathBuf, Vec<ScanEntry>>,
    expanded: BTreeSet<PathBuf>,
    loading: BTreeSet<PathBuf>,
    diagnostics: Vec<String>,
    generation: u64,
    _tree_subscription: Subscription,
}

impl FileTreeView {
    pub fn new(root: impl Into<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let root = root.into();
        let tree = cx.new(|cx| {
            TreeState::new(cx).items(vec![Self::root_item(
                &root,
                &BTreeMap::new(),
                &BTreeSet::from([PathBuf::new()]),
            )])
        });
        tree.update(cx, |tree, cx| tree.focus(window, cx));
        let subscription = cx.subscribe(&tree, |this, _tree, event, cx| {
            this.handle_tree_event(event, cx);
        });
        let mut view = Self {
            root,
            tree,
            entries: BTreeMap::new(),
            expanded: BTreeSet::from([PathBuf::new()]),
            loading: BTreeSet::new(),
            diagnostics: Vec::new(),
            generation: 0,
            _tree_subscription: subscription,
        };
        view.load_directory(PathBuf::new(), cx);
        view
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn tree_state(&self) -> Entity<TreeState> {
        self.tree.clone()
    }

    pub fn diagnostics(&self) -> &[String] {
        &self.diagnostics
    }

    pub fn selected_path(&self, cx: &gpui_kit::App) -> Option<PathBuf> {
        self.tree.read(cx).selected_item().and_then(|item| {
            if item.id.as_ref() == ROOT_ID || item.id.ends_with(LOADING_SUFFIX) {
                None
            } else {
                Some(PathBuf::from(item.id.as_ref()))
            }
        })
    }

    pub(crate) fn open_directory(&mut self, logical_path: &Path, cx: &mut Context<Self>) {
        for ancestor in logical_path.ancestors() {
            let ancestor = ancestor.to_path_buf();
            self.expanded.insert(ancestor.clone());
            self.load_directory(ancestor, cx);
        }
        self.rebuild_tree(cx);
        cx.notify();
    }

    pub fn create_directory(
        &mut self,
        parent: impl AsRef<Path>,
        name: &str,
        cx: &mut Context<Self>,
    ) -> Result<PathBuf, FileTreeError> {
        let relative = parent.as_ref().join(name);
        let path = create_directory(&self.root, &relative)?;
        self.refresh(parent.as_ref(), cx);
        Ok(path)
    }

    pub fn create_markdown(
        &mut self,
        parent: impl AsRef<Path>,
        name: &str,
        cx: &mut Context<Self>,
    ) -> Result<PathBuf, FileTreeError> {
        let relative = parent.as_ref().join(name);
        let path = create_markdown(&self.root, &relative)?;
        self.refresh(parent.as_ref(), cx);
        Ok(path)
    }

    pub fn move_item(
        &mut self,
        source: impl AsRef<Path>,
        destination: impl AsRef<Path>,
        cx: &mut Context<Self>,
    ) -> Result<PathBuf, FileTreeError> {
        let source = source.as_ref().to_path_buf();
        let destination = destination.as_ref().to_path_buf();
        let result = move_entry(&self.root, &source, &destination)?;
        self.refresh_parent(&source, cx);
        self.refresh_parent(&destination, cx);
        Ok(result)
    }

    pub fn rename_item(
        &mut self,
        source: impl AsRef<Path>,
        new_name: &str,
        cx: &mut Context<Self>,
    ) -> Result<PathBuf, FileTreeError> {
        let source = source.as_ref().to_path_buf();
        let result = rename_entry(&self.root, &source, new_name)?;
        self.refresh_parent(&source, cx);
        Ok(result)
    }

    pub fn reveal_selected(&self, cx: &gpui_kit::App) -> Result<(), FileTreeError> {
        let Some(path) = self.selected_path(cx) else {
            return Ok(());
        };
        reveal_in_system(&self.root.join(path)).map_err(FileTreeError::Reveal)
    }

    fn handle_tree_event(&mut self, event: &TreeEvent, cx: &mut Context<Self>) {
        let path = match event {
            TreeEvent::Expanded(id) | TreeEvent::Collapsed(id) => PathBuf::from(id.as_ref()),
        };
        match event {
            TreeEvent::Expanded(_) => {
                self.expanded.insert(path.clone());
                self.load_directory(path, cx);
            }
            TreeEvent::Collapsed(_) => {
                self.expanded.remove(&path);
            }
        }
    }

    fn load_directory(&mut self, logical_path: PathBuf, cx: &mut Context<Self>) {
        if self.entries.contains_key(&logical_path) || !self.loading.insert(logical_path.clone()) {
            return;
        }
        let generation = self.generation;
        let root = self.root.clone();
        let request_path = logical_path.clone();
        let task = cx.background_spawn(async move {
            scan_directory_children(root, request_path, ScanOptions::default())
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading.remove(&logical_path);
                match result {
                    Ok(report) => {
                        this.entries
                            .insert(logical_path.clone(), report.entries().to_vec());
                        this.diagnostics
                            .extend(report.diagnostics().iter().map(|diagnostic| {
                                format!("{}：{}", diagnostic.path.display(), diagnostic.message)
                            }));
                    }
                    Err(error) => {
                        this.entries.insert(logical_path.clone(), Vec::new());
                        this.diagnostics.push(error.to_string());
                    }
                }
                this.rebuild_tree(cx);
            });
        })
        .detach();
    }

    fn refresh(&mut self, logical_path: &Path, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        self.entries.remove(logical_path);
        self.loading.remove(logical_path);
        self.load_directory(logical_path.to_path_buf(), cx);
    }

    fn refresh_parent(&mut self, logical_path: &Path, cx: &mut Context<Self>) {
        self.refresh(logical_path.parent().unwrap_or_else(|| Path::new("")), cx);
    }

    fn rebuild_tree(&mut self, cx: &mut Context<Self>) {
        let item = Self::root_item(&self.root, &self.entries, &self.expanded);
        self.tree
            .update(cx, |tree, cx| tree.set_items(vec![item], cx));
    }

    fn root_item(
        root: &Path,
        entries: &BTreeMap<PathBuf, Vec<ScanEntry>>,
        expanded: &BTreeSet<PathBuf>,
    ) -> TreeItem {
        let label = root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.display().to_string());
        let children = entries
            .get(Path::new(""))
            .map(|children| {
                children
                    .iter()
                    .map(|entry| Self::tree_item(entry, entries, expanded))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| {
                vec![
                    TreeItem::new(format!("{ROOT_ID}{LOADING_SUFFIX}"), "正在加载…").disabled(true),
                ]
            });
        TreeItem::new(ROOT_ID, label)
            .children(children)
            .expanded(expanded.contains(Path::new("")))
    }

    fn tree_item(
        entry: &ScanEntry,
        entries: &BTreeMap<PathBuf, Vec<ScanEntry>>,
        expanded: &BTreeSet<PathBuf>,
    ) -> TreeItem {
        let path = entry.logical_path().to_path_buf();
        let mut item = TreeItem::new(
            entry.id(),
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string()),
        );
        if entry.is_directory() {
            let children = entries
                .get(&path)
                .map(|children| {
                    children
                        .iter()
                        .map(|entry| Self::tree_item(entry, entries, expanded))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_else(|| {
                    vec![
                        TreeItem::new(format!("{}{LOADING_SUFFIX}", entry.id()), "正在加载…")
                            .disabled(true),
                    ]
                });
            item = item.children(children).expanded(expanded.contains(&path));
        }
        item
    }
}

impl Render for FileTreeView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let status = self
            .diagnostics
            .last()
            .cloned()
            .unwrap_or_else(|| "文件树已就绪".to_owned());
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .id("file-tree-status")
                    .text_sm()
                    .px_2()
                    .py_1()
                    .child(SharedString::from(status)),
            )
            .child(Tree::new(
                &self.tree,
                |ix, entry, _selected, _window, _cx| {
                    ListItem::new(("file-tree-item", ix))
                        .pl(px(12. + entry.depth() as f32 * 16.))
                        .on_click({
                            let path = entry.item().id.to_string();
                            move |_, _, cx| {
                                if let Some(service) =
                                    cx.try_global::<crate::workbench::Workbench>()
                                {
                                    service.emit(crate::workbench::WorkbenchEvent::Open(
                                        service.root.join(&path),
                                    ));
                                }
                            }
                        })
                        .child(entry.item().label.clone())
                },
            ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{TestAppContext, WindowOptions};

    #[gpui_kit::test]
    fn file_tree_uses_tree_state_keyboard_contract(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let root = std::env::temp_dir().join(format!("ohmywork-tree-ui-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("Atlas")).expect("create root");
        std::fs::write(root.join("Atlas/note.md"), "note").expect("write note");
        let (handle, view) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| FileTreeView::new(root.clone(), window, cx))
            })
            .expect("open file tree")
        });
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            view.update(cx, |view, cx| {
                assert!(view.tree.read(cx).selected_item().is_none());
                view.tree
                    .update(cx, |tree, cx| tree.set_selected_index(Some(0), cx));
            });
            window.press("right", cx);
            window.press("down", cx);
        })
        .expect("navigate file tree");
        cx.update(|cx| {
            assert!(view.read(cx).tree.read(cx).selected_item().is_some());
        });
        let _ = std::fs::remove_dir_all(root);
    }
}
