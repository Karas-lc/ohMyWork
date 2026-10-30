use std::{path::PathBuf, sync::Arc};

use editor::documents::DocumentStore;
use gpui_kit::{App, AppContext as _, Entity, Global};

use crate::{
    closing::CloseCoordinator,
    homepage::Homepage,
    pi::PiSession,
    preview::PreviewSession,
    queries::{files::FileBrowser, search::SearchModel, tasks::TasksModel},
    session::WorkspaceSession,
};

#[derive(Clone)]
pub(crate) struct WorkspaceContext {
    workspace: Entity<WorkspaceSession>,
    documents: Entity<DocumentStore>,
    files: Entity<FileBrowser>,
    search: Entity<SearchModel>,
    tasks: Entity<TasksModel>,
    homepage: Entity<Homepage>,
    pi: Entity<PiSession>,
    closing: Entity<CloseCoordinator>,
    runtime: Arc<tokio::runtime::Runtime>,
    preview_worker: PathBuf,
}

impl Global for WorkspaceContext {}

impl WorkspaceContext {
    pub(crate) fn new(
        workspace: Entity<WorkspaceSession>,
        documents: Entity<DocumentStore>,
        files: Entity<FileBrowser>,
        search: Entity<SearchModel>,
        tasks: Entity<TasksModel>,
        homepage: Entity<Homepage>,
        pi: Entity<PiSession>,
        closing: Entity<CloseCoordinator>,
        runtime: Arc<tokio::runtime::Runtime>,
        preview_worker: PathBuf,
    ) -> Self {
        Self {
            workspace,
            documents,
            files,
            search,
            tasks,
            homepage,
            pi,
            closing,
            runtime,
            preview_worker,
        }
    }

    pub(crate) fn workspace(&self) -> Entity<WorkspaceSession> { self.workspace.clone() }
    pub(crate) fn documents(&self) -> Entity<DocumentStore> { self.documents.clone() }
    pub(crate) fn files(&self) -> Entity<FileBrowser> { self.files.clone() }
    pub(crate) fn search(&self) -> Entity<SearchModel> { self.search.clone() }
    pub(crate) fn tasks(&self) -> Entity<TasksModel> { self.tasks.clone() }
    pub(crate) fn homepage(&self) -> Entity<Homepage> { self.homepage.clone() }
    pub(crate) fn pi(&self) -> Entity<PiSession> { self.pi.clone() }
    pub(crate) fn closing(&self) -> Entity<CloseCoordinator> { self.closing.clone() }

    pub(crate) fn new_preview(&self, path: PathBuf, cx: &mut App) -> Entity<PreviewSession> {
        cx.new(|cx| {
            PreviewSession::new(
                path,
                self.preview_worker.clone(),
                self.runtime.clone(),
                cx,
            )
        })
    }
}
