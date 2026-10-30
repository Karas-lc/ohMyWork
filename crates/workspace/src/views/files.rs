use std::{collections::BTreeSet, path::{Path, PathBuf}};

use gpui_kit::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, SharedString, Styled as _, Subscription, Window,
    component::{button::Button, input::{Input, InputState}, list::ListItem, tree::{Tree, TreeEvent, TreeItem, TreeState}},
    div, px,
};
use project::ScanEntry;

use crate::queries::files::{FileBrowser, FileCommand};

const ROOT_ID: &str = "";
const LOADING_SUFFIX: &str = "/.ohmywork-loading";

pub struct FileTreeView {
    browser: Entity<FileBrowser>,
    session: Entity<crate::session::WorkspaceSession>,
    name: Entity<InputState>,
    tree: Entity<TreeState>,
    expanded: BTreeSet<PathBuf>,
    _tree_subscription: Subscription,
    _browser_subscription: Subscription,
}

impl FileTreeView {
    pub fn new(browser: Entity<FileBrowser>, session: Entity<crate::session::WorkspaceSession>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let root = browser.read(cx).root().to_path_buf();
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("文件名 / 目标相对路径"));
        let tree = cx.new(|cx| TreeState::new(cx).items(vec![TreeItem::new(ROOT_ID, root_label(&root)).children(vec![
            TreeItem::new(format!("{ROOT_ID}{LOADING_SUFFIX}"), "正在加载…").disabled(true),
        ]).expanded(true)]));
        tree.update(cx, |tree, cx| tree.focus(window, cx));
        let tree_subscription = cx.subscribe(&tree, |this, _tree, event, cx| this.handle_tree_event(event, cx));
        let browser_subscription = cx.observe(&browser, |this, _, cx| this.rebuild_tree(cx));
        let mut this = Self {
            browser,
            session,
            name,
            tree,
            expanded: BTreeSet::from([PathBuf::new()]),
            _tree_subscription: tree_subscription,
            _browser_subscription: browser_subscription,
        };
        this.rebuild_tree(cx);
        this
    }


    pub fn selected_path(&self, cx: &gpui_kit::App) -> Option<PathBuf> {
        self.tree.read(cx).selected_item().and_then(|item| {
            if item.id.as_ref() == ROOT_ID || item.id.ends_with(LOADING_SUFFIX) { None }
            else { Some(PathBuf::from(item.id.as_ref())) }
        })
    }

    pub(crate) fn open_directory(&mut self, logical_path: &Path, cx: &mut Context<Self>) {
        for ancestor in logical_path.ancestors() {
            let ancestor = ancestor.to_path_buf();
            self.expanded.insert(ancestor.clone());
            self.browser.update(cx, |browser, cx| browser.load_directory(ancestor, cx));
        }
        self.rebuild_tree(cx);
        cx.notify();
    }

    pub fn apply(&self, command: FileCommand, cx: &mut Context<Self>) {
        self.browser.update(cx, |browser, cx| browser.apply(command, cx));
    }

    pub fn reveal_selected(&self, cx: &mut Context<Self>) {
        if let Some(path) = self.selected_path(cx) {
            self.apply(FileCommand::Reveal { path }, cx);
        }
    }
    fn directory_target(&self, cx: &gpui_kit::App) -> PathBuf {
        let Some(selected) = self.selected_path(cx) else { return PathBuf::new(); };
        let parent = selected.parent().unwrap_or_else(|| Path::new(""));
        let is_directory = self.browser.read(cx).entries(parent)
            .and_then(|entries| entries.iter().find(|entry| entry.logical_path() == selected))
            .is_some_and(ScanEntry::is_directory);
        if is_directory { selected } else { parent.to_path_buf() }
    }

    fn create_directory(&self, cx: &mut Context<Self>) {
        let name = self.name.read(cx).value().to_string();
        if !name.trim().is_empty() {
            let parent = self.directory_target(cx);
            self.apply(FileCommand::CreateDirectory { parent, name }, cx);
        }
    }

    fn create_markdown(&self, cx: &mut Context<Self>) {
        let name = self.name.read(cx).value().to_string();
        if !name.trim().is_empty() {
            let parent = self.directory_target(cx);
            self.apply(FileCommand::CreateMarkdown { parent, name }, cx);
        }
    }

    fn rename_selected(&self, cx: &mut Context<Self>) {
        let name = self.name.read(cx).value().to_string();
        if let Some(source) = self.selected_path(cx).filter(|_| !name.trim().is_empty()) {
            self.apply(FileCommand::Rename { source, name }, cx);
        }
    }

    fn move_selected(&self, cx: &mut Context<Self>) {
        let destination = PathBuf::from(self.name.read(cx).value().to_string());
        if let Some(source) = self.selected_path(cx).filter(|_| !destination.as_os_str().is_empty()) {
            self.apply(FileCommand::Move { source, destination }, cx);
        }
    }

    fn refresh_selected(&self, cx: &mut Context<Self>) {
        let directory = self.directory_target(cx);
        self.browser.update(cx, |browser, cx| browser.refresh(directory, cx));
    }

    fn handle_tree_event(&mut self, event: &TreeEvent, cx: &mut Context<Self>) {
        let path = match event { TreeEvent::Expanded(id) | TreeEvent::Collapsed(id) => PathBuf::from(id.as_ref()) };
        match event {
            TreeEvent::Expanded(_) => {
                self.expanded.insert(path.clone());
                self.browser.update(cx, |browser, cx| browser.load_directory(path, cx));
            }
            TreeEvent::Collapsed(_) => { self.expanded.remove(&path); }
        }
        self.rebuild_tree(cx);
    }

    fn rebuild_tree(&mut self, cx: &mut Context<Self>) {
        let browser = self.browser.read(cx);
        let root = browser.root().to_path_buf();
        let item = root_item(&root, &browser, &self.expanded);
        self.tree.update(cx, |tree, cx| tree.set_items(vec![item], cx));
    }
}

impl Render for FileTreeView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let status = self.browser.read(cx).status().to_owned();
        let session = self.session.clone();
        let root = self.browser.read(cx).root().to_path_buf();
        div().size_full().flex().flex_col()
            .child(div().flex().flex_wrap().gap_1().p_2()
                .child(Input::new(&self.name).id("file-operation-name").aria_label("文件名或目标相对路径"))
                .child(Button::new("file-create-directory").label("新建目录").on_click(cx.listener(|this, _, _, cx| this.create_directory(cx))))
                .child(Button::new("file-create-markdown").label("新建 Markdown").on_click(cx.listener(|this, _, _, cx| this.create_markdown(cx))))
                .child(Button::new("file-rename").label("重命名").on_click(cx.listener(|this, _, _, cx| this.rename_selected(cx))))
                .child(Button::new("file-move").label("移动").on_click(cx.listener(|this, _, _, cx| this.move_selected(cx))))
                .child(Button::new("file-reveal").label("在系统中显示").on_click(cx.listener(|this, _, _, cx| this.reveal_selected(cx))))
                .child(Button::new("file-refresh").label("刷新").on_click(cx.listener(|this, _, _, cx| this.refresh_selected(cx)))))
            .child(div().id("file-tree-status").text_sm().px_2().py_1().child(SharedString::from(status)))
            .child(Tree::new(&self.tree, move |_ix, entry, _selected, _window, _cx| {
                let path = root.join(entry.item().id.as_ref());
                ListItem::new(entry.item().id.clone())
                    .pl(px(12. + entry.depth() as f32 * 16.))
                    .on_click({
                        let session = session.clone();
                        let path = path.clone();
                        move |_, _, cx| session.update(cx, |session, cx| session.request_open(path.clone(), cx))
                    })
                    .child(entry.item().label.clone())
            }))
    }
}

fn root_label(root: &Path) -> String {
    root.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| root.display().to_string())
}

fn root_item(root: &Path, browser: &FileBrowser, expanded: &BTreeSet<PathBuf>) -> TreeItem {
    let children = match browser.entries(Path::new("")) {
        Some(entries) => entries.iter().map(|entry| tree_item(entry, browser, expanded)).collect(),
        None if browser.is_loading(Path::new("")) => vec![TreeItem::new(format!("{ROOT_ID}{LOADING_SUFFIX}"), "正在加载…").disabled(true)],
        None => Vec::new(),
    };
    TreeItem::new(ROOT_ID, root_label(root)).children(children).expanded(expanded.contains(Path::new("")))
}

fn tree_item(entry: &ScanEntry, browser: &FileBrowser, expanded: &BTreeSet<PathBuf>) -> TreeItem {
    let path = entry.logical_path().to_path_buf();
    let mut item = TreeItem::new(entry.id(), path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string()));
    if entry.is_directory() {
        let children = match browser.entries(&path) {
            Some(entries) => entries.iter().map(|entry| tree_item(entry, browser, expanded)).collect(),
            None if browser.is_loading(&path) => vec![TreeItem::new(format!("{}{LOADING_SUFFIX}", entry.id()), "正在加载…").disabled(true)],
            None => Vec::new(),
        };
        item = item.children(children).expanded(expanded.contains(&path));
    }
    item
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
            let browser = cx.new(|cx| FileBrowser::new(root.clone(), cx));
            let session = cx.new(|cx| crate::session::WorkspaceSession::new(None, cx));
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| FileTreeView::new(browser, session, window, cx))
            })
            .expect("open file tree")
        });
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            view.update(cx, |view, cx| {
                assert!(view.tree.read(cx).selected_item().is_none());
                view.tree.update(cx, |tree, cx| tree.set_selected_index(Some(0), cx));
            });
            window.press("right", cx);
            window.press("down", cx);
        })
        .expect("navigate file tree");
        cx.update(|cx| assert!(view.read(cx).tree.read(cx).selected_item().is_some()));
    }
}
