use std::{path::{Path, PathBuf}, sync::Arc};

use gpui_kit::{AppContext as _, Context, EventEmitter};
use project::{AppDataPaths, IndexStore, WorkspaceState};

use crate::indexing;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkspaceEvent {
    OpenPage(PathBuf),
    OpenPreview(PathBuf),
    RevealDirectory(PathBuf),
    ShowTasks,
    ShowPi,
    Ready,
    Failed(String),
}

pub struct WorkspaceSession {
    app_data: Option<AppDataPaths>,
    state: Option<WorkspaceState>,
    index: Option<Arc<IndexStore>>,
    active: Option<PathBuf>,
    recent: Vec<PathBuf>,
    ready: bool,
    generation: u64,
    busy: bool,
}

impl WorkspaceSession {
    pub fn new(app_data: Option<AppDataPaths>, _: &mut Context<Self>) -> Self {
        Self { app_data, state: None, index: None, active: None, recent: Vec::new(), ready: false, generation: 0, busy: false }
    }

    pub fn restore(&mut self, cx: &mut Context<Self>) {
        if self.busy || self.ready { return; }
        let Some(paths) = self.app_data.clone() else { return };
        self.busy = true;
        let generation = self.bump_generation();
        let task = cx.background_spawn(async move { WorkspaceState::load(paths) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation { return; }
                match result {
                    Ok(Some(state)) => this.accept_state(state, cx),
                    Ok(None) => this.busy = false,
                    Err(error) => { this.busy = false; cx.emit(WorkspaceEvent::Failed(error.to_string())); }
                }
                cx.notify();
            });
        }).detach();
    }

    pub fn initialize(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        if self.ready || self.busy { return; }
        let Some(paths) = self.app_data.clone() else {
            cx.emit(WorkspaceEvent::Failed("缺少 %LOCALAPPDATA%，无法保存设置。".into()));
            cx.notify();
            return;
        };
        self.busy = true;
        let generation = self.bump_generation();
        let task = cx.background_spawn(async move { WorkspaceState::initialize(root, paths) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation { return; }
                match result {
                    Ok(state) => this.accept_state(state, cx),
                    Err(error) => { this.busy = false; cx.emit(WorkspaceEvent::Failed(error.to_string())); }
                }
                cx.notify();
            });
        }).detach();
    }

    fn accept_state(&mut self, state: WorkspaceState, cx: &mut Context<Self>) {
        let root = state.root().to_path_buf();
        let app_data = state.app_data().clone();
        let index = cx.background_spawn(async move { IndexStore::open_app_data(&app_data) });
        let generation = self.generation;
        cx.spawn(async move |this, cx| {
            let result = index.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation { return; }
                match result {
                    Ok(index) => {
                        this.state = Some(state);
                        let index = Arc::new(index);
                        this.index = Some(index.clone());
                        let task = cx.background_spawn(async move { indexing::index_workspace(&index, root, project::ScanOptions::default()) });
                        cx.spawn(async move |this, cx| {
                            let result = task.await;
                            let _ = this.update(cx, |this, cx| {
                                if this.generation != generation { return; }
                                match result {
                                    Ok(_) => {
                                        this.ready = true;
                                        cx.emit(WorkspaceEvent::Ready);
                                    }
                                    Err(error) => cx.emit(WorkspaceEvent::Failed(error.to_string())),
                                }
                                this.busy = false;
                            });
                        }).detach();
                    }
                    Err(error) => { this.busy = false; cx.emit(WorkspaceEvent::Failed(error.to_string())); }
                }
                cx.notify();
            });
        }).detach();
    }

    pub fn request_open(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if !self.ready || path.as_os_str().is_empty() { return; }
        let generation = self.generation;
        let root = self.root().map(Path::to_path_buf);
        let task = cx.background_spawn(async move {
            let path = if path.is_absolute() {
                project::normalize_document_path(path)
            } else if let Some(root) = &root {
                project::normalize_document_path(root.join(path))
            } else {
                project::normalize_document_path(path)
            };
            let is_directory = path.is_dir();
            if is_directory {
                if let Some(root) = root {
                    let root = project::normalize_document_path(root);
                    if !path.starts_with(&root) { return Err("目录不在当前 Workspace 内".to_owned()); }
                }
                return Ok(Some(WorkspaceEvent::RevealDirectory(path)));
            }
            let ext = path.extension().and_then(|value| value.to_str()).unwrap_or_default().to_ascii_lowercase();
            if matches!(ext.as_str(), "md" | "markdown") { Ok(Some(WorkspaceEvent::OpenPage(path))) }
            else if matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "pdf") { Ok(Some(WorkspaceEvent::OpenPreview(path))) }
            else {
                std::process::Command::new("explorer.exe").arg(path).spawn().map(|_| None).map_err(|error| error.to_string())
            }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation { return; }
                match result {
                    Ok(Some(event)) => {
                        if let WorkspaceEvent::OpenPage(path) | WorkspaceEvent::OpenPreview(path) = &event {
                            this.activate(path.clone(), cx);
                        }
                        cx.emit(event);
                    }
                    Ok(None) => (),
                    Err(error) => cx.emit(WorkspaceEvent::Failed(error)),
                }
                cx.notify();
            });
        }).detach();
    }

    pub fn activate(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.active = Some(path.clone());
        self.recent.retain(|item| item != &path);
        self.recent.insert(0, path);
        self.recent.truncate(20);
        cx.notify();
    }
    pub fn show_tasks(&mut self, cx: &mut Context<Self>) { cx.emit(WorkspaceEvent::ShowTasks); cx.notify(); }
    pub fn show_pi(&mut self, cx: &mut Context<Self>) { cx.emit(WorkspaceEvent::ShowPi); cx.notify(); }
    pub fn root(&self) -> Option<&Path> { self.state.as_ref().map(WorkspaceState::root) }
    pub fn app_data(&self) -> Option<&AppDataPaths> { self.app_data.as_ref() }
    pub fn active(&self) -> Option<&Path> { self.active.as_deref() }
    pub fn recent(&self) -> &[PathBuf] { &self.recent }
    pub fn ready(&self) -> bool { self.ready }
    pub fn index_handle(&self) -> Option<Arc<IndexStore>> { self.index.clone() }

    fn bump_generation(&mut self) -> u64 { self.generation = self.generation.wrapping_add(1); self.generation }
}
impl EventEmitter<WorkspaceEvent> for WorkspaceSession {}
