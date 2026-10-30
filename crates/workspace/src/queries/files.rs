use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, atomic::{AtomicBool, Ordering}},
};

use gpui_kit::{AppContext as _, Context};
use project::{
    FileOperationError, ScanEntry, ScanOptions, create_directory, create_markdown, move_entry,
    rename_entry, reveal_in_system, scan_directory_children,
};

#[derive(Clone, Debug)]
pub enum FileCommand {
    CreateDirectory { parent: PathBuf, name: String },
    CreateMarkdown { parent: PathBuf, name: String },
    Move { source: PathBuf, destination: PathBuf },
    Rename { source: PathBuf, name: String },
    Reveal { path: PathBuf },
}

#[derive(Debug)]
pub enum FileBrowserError {
    Operation(FileOperationError),
    Reveal(std::io::Error),
}

impl std::fmt::Display for FileBrowserError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Operation(error) => error.fmt(f),
            Self::Reveal(error) => write!(f, "在系统中显示失败：{error}"),
        }
    }
}
impl std::error::Error for FileBrowserError {}
impl From<FileOperationError> for FileBrowserError {
    fn from(value: FileOperationError) -> Self { Self::Operation(value) }
}

#[derive(Clone, Default)]
struct ScanCancel(Arc<AtomicBool>);
impl ScanCancel {
    fn cancel(&self) { self.0.store(true, Ordering::Release); }
    fn is_cancelled(&self) -> bool { self.0.load(Ordering::Acquire) }
}

pub struct FileBrowser {
    root: PathBuf,
    entries: BTreeMap<PathBuf, Vec<ScanEntry>>,
    status: String,
    generation: u64,
    scan_generation: u64,
    scans: BTreeMap<PathBuf, (u64, ScanCancel)>,
}

impl FileBrowser {
    pub fn new(root: PathBuf, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            root,
            entries: BTreeMap::new(),
            status: "文件树已就绪".into(),
            generation: 0,
            scan_generation: 0,
            scans: BTreeMap::new(),
        };
        this.load_directory(PathBuf::new(), cx);
        this
    }

    pub fn root(&self) -> &Path { &self.root }
    pub fn entries(&self, directory: &Path) -> Option<&[ScanEntry]> {
        self.entries.get(directory).map(Vec::as_slice)
    }
    pub fn status(&self) -> &str { &self.status }
    pub fn is_loading(&self, directory: &Path) -> bool { self.scans.contains_key(directory) }

    pub fn load_directory(&mut self, logical_path: PathBuf, cx: &mut Context<Self>) {
        if self.entries.contains_key(&logical_path) || self.scans.contains_key(&logical_path) { return; }
        self.start_scan(logical_path, cx);
    }

    pub fn refresh(&mut self, logical_path: PathBuf, cx: &mut Context<Self>) {
        if let Some((_, token)) = self.scans.remove(&logical_path) { token.cancel(); }
        self.entries.remove(&logical_path);
        self.start_scan(logical_path, cx);
    }

    pub fn apply(&mut self, command: FileCommand, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.status = "正在执行文件操作…".into();
        cx.notify();
        let root = self.root.clone();
        let task = cx.background_spawn(async move {
            let result: Result<(Option<PathBuf>, Vec<PathBuf>), FileBrowserError> = match command {
                FileCommand::CreateDirectory { parent, name } => {
                    let path = create_directory(&root, parent.join(name)).map_err(FileBrowserError::from)?;
                    Ok((Some(path), vec![parent]))
                }
                FileCommand::CreateMarkdown { parent, name } => {
                    let path = create_markdown(&root, parent.join(name)).map_err(FileBrowserError::from)?;
                    Ok((Some(path), vec![parent]))
                }
                FileCommand::Move { source, destination } => {
                    let source_parent = source.parent().unwrap_or_else(|| Path::new("")).to_path_buf();
                    let destination_parent = destination.parent().unwrap_or_else(|| Path::new("")).to_path_buf();
                    let path = move_entry(&root, source, destination).map_err(FileBrowserError::from)?;
                    let mut parents = vec![source_parent.clone()];
                    if source_parent != destination_parent { parents.push(destination_parent); }
                    Ok((Some(path), parents))
                }
                FileCommand::Rename { source, name } => {
                    let parent = source.parent().unwrap_or_else(|| Path::new("")).to_path_buf();
                    let path = rename_entry(&root, source, &name).map_err(FileBrowserError::from)?;
                    Ok((Some(path), vec![parent]))
                }
                FileCommand::Reveal { path } => {
                    reveal_in_system(&root.join(path)).map_err(FileBrowserError::Reveal)?;
                    Ok((None, Vec::new()))
                }
            };
            result
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                let latest_operation = this.generation == generation;
                match result {
                    Ok((path, refresh)) => {
                        if latest_operation { this.status = "文件树已就绪".into(); }
                        if path.is_some() {
                            for parent in refresh { this.refresh(parent, cx); }
                        } else {
                            cx.notify();
                        }
                    }
                    Err(error) => {
                        if latest_operation { this.status = error.to_string(); }
                        cx.notify();
                    }
                }
            });
        }).detach();
    }

    fn start_scan(&mut self, logical_path: PathBuf, cx: &mut Context<Self>) {
        self.scan_generation = self.scan_generation.wrapping_add(1);
        let generation = self.scan_generation;
        let token = ScanCancel::default();
        self.scans.insert(logical_path.clone(), (generation, token.clone()));
        let root = self.root.clone();
        let request_path = logical_path.clone();
        let worker_token = token.clone();
        let task = cx.background_spawn(async move {
            if worker_token.is_cancelled() { return None; }
            let result = scan_directory_children(root, request_path, ScanOptions::default());
            (!worker_token.is_cancelled()).then_some(result)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                let is_current = this.scans.get(&logical_path).is_some_and(|(current, current_token)| {
                    *current == generation && !current_token.is_cancelled()
                });
                if !is_current { return; }
                this.scans.remove(&logical_path);
                match result {
                    Some(Ok(report)) => {
                        this.entries.insert(logical_path, report.entries().to_vec());
                        this.status = report.diagnostics().last()
                            .map(|diagnostic| format!("{}：{}", diagnostic.path.display(), diagnostic.message))
                            .unwrap_or_else(|| "文件树已就绪".into());
                    }
                    Some(Err(error)) => {
                        this.entries.insert(logical_path, Vec::new());
                        this.status = error.to_string();
                    }
                    None => return,
                }
                cx.notify();
            });
        }).detach();
    }
}

impl Drop for FileBrowser {
    fn drop(&mut self) {
        for (_, token) in self.scans.values() { token.cancel(); }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::TestAppContext;
    use std::fs;

    #[gpui_kit::test]
    async fn file_operations_update_the_real_tree_and_disk(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let root = std::env::temp_dir().join(format!("ohmywork-file-browser-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let browser = cx.update(|cx| cx.new(|cx| FileBrowser::new(root.clone(), cx)));
        cx.run_until_parked();

        cx.update(|cx| browser.update(cx, |browser, cx| {
            browser.apply(FileCommand::CreateDirectory { parent: PathBuf::new(), name: "sub".into() }, cx);
        }));
        cx.run_until_parked();
        assert!(root.join("sub").is_dir());
        assert!(cx.update(|cx| browser.read(cx).entries(Path::new("")).unwrap().iter().any(|entry| entry.logical_path() == Path::new("sub"))));

        cx.update(|cx| browser.update(cx, |browser, cx| {
            browser.apply(FileCommand::CreateMarkdown { parent: PathBuf::from("sub"), name: "note.md".into() }, cx);
        }));
        cx.run_until_parked();
        assert_eq!(fs::read_to_string(root.join("sub/note.md")).unwrap(), "# ");

        cx.update(|cx| browser.update(cx, |browser, cx| {
            browser.apply(FileCommand::Rename { source: PathBuf::from("sub/note.md"), name: "renamed.md".into() }, cx);
        }));
        cx.run_until_parked();
        assert!(!root.join("sub/note.md").exists());
        assert!(root.join("sub/renamed.md").is_file());
        assert!(cx.update(|cx| browser.read(cx).entries(Path::new("sub")).unwrap().iter().any(|entry| entry.logical_path() == Path::new("sub/renamed.md"))));

        cx.update(|cx| browser.update(cx, |browser, cx| {
            browser.apply(FileCommand::Move { source: PathBuf::from("sub/renamed.md"), destination: PathBuf::from("moved.md") }, cx);
        }));
        cx.run_until_parked();
        assert!(!root.join("sub/renamed.md").exists());
        assert_eq!(fs::read_to_string(root.join("moved.md")).unwrap(), "# ");
        assert!(cx.update(|cx| browser.read(cx).entries(Path::new("")).unwrap().iter().any(|entry| entry.logical_path() == Path::new("moved.md"))));

        let _ = fs::remove_dir_all(root);
    }
}
