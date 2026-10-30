use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    time::Duration,
};

use gpui_kit::{App, AppContext as _, Context, Entity, EventEmitter, Subscription};
use project::{DiskSnapshot, WriteQueue};

use crate::documents::SourceRope;

use super::{PageDocument, PageError, persistence, task_commit::{self, TaskIntent}};
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PageOpenState {
    NotOpened,
    Loading,
    Ready,
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DocumentEvent {
    Opened(PathBuf),
    Saved(PathBuf),
    TaskCommitted(PathBuf),
    Changed(PathBuf),
    Released(PathBuf),
    Failed { path: PathBuf, message: String },
}

enum DocumentStatus {
    Loading,
    Ready { page: Entity<PageDocument>, _subscription: Subscription },
    Failed(String),
}

struct DocumentEntry {
    generation: u64,
    lifetime: u64,
    status: DocumentStatus,
    operation: u64,
    pending_content: Option<([u8; 32], u64)>,
    save_revision: Option<u64>,
    last_external_error: Option<String>,
}
enum ExternalRead {
    Snapshot(DiskSnapshot, SourceRope),
    Missing,
    Failed(String),
}

pub struct DocumentStore {
    queue: WriteQueue,
    documents: HashMap<PathBuf, DocumentEntry>,
    aliases: HashMap<PathBuf, PathBuf>,
    pending_tasks: HashSet<PathBuf>,
    next_generation: u64,
}

impl DocumentStore {
    pub fn new(queue: WriteQueue, cx: &mut Context<Self>) -> Self {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(200)).await;
                let targets = match this.update(cx, |store, cx| store.external_targets(cx)) {
                    Ok(targets) => targets,
                    Err(_) => break,
                };
                if targets.is_empty() {
                    continue;
                }
                let reads = cx.background_spawn(async move {
                    targets
                        .into_iter()
                        .map(|(path, generation, lifetime, operation, revision)| {
                            let snapshot = match project::read_snapshot(&path) {
                                Ok(snapshot) => match crate::documents::decode_source(&snapshot.bytes) {
                                    Ok(source) => ExternalRead::Snapshot(snapshot, source),
                                    Err(error) => ExternalRead::Failed(error.to_string()),
                                },
                                Err(project::AtomicWriteError::Io { source, .. })
                                    if source.kind() == std::io::ErrorKind::NotFound =>
                                {
                                    ExternalRead::Missing
                                }
                                Err(error) => ExternalRead::Failed(error.to_string()),
                            };
                            (path, generation, lifetime, operation, revision, snapshot)
                        })
                        .collect::<Vec<_>>()
                });
                let results = reads.await;
                let _ = this.update(cx, |store, cx| store.apply_external_reads(results, cx));
            }
        }).detach();
        Self {
            queue,
            documents: HashMap::new(),
            aliases: HashMap::new(),
            pending_tasks: HashSet::new(),
            next_generation: 1,
        }
    }

    pub fn open(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let normalized = project::normalize_document_path(path.clone());
        self.aliases.insert(path, normalized.clone());
        self.aliases.insert(normalized.clone(), normalized.clone());
        if let Some(entry) = self.documents.get_mut(&normalized) {
            if !matches!(&entry.status, DocumentStatus::Failed(_)) {
                return;
            }
            entry.generation = self.next_generation;
            entry.lifetime = self.next_generation;
            self.next_generation = self.next_generation.wrapping_add(1).max(1);
            entry.status = DocumentStatus::Loading;
            entry.pending_content = None;
            entry.save_revision = None;
            entry.last_external_error = None;
        } else {
            let generation = self.next_generation;
            self.next_generation = self.next_generation.wrapping_add(1).max(1);
            self.documents.insert(normalized.clone(), DocumentEntry {
                generation,
                lifetime: generation,
                status: DocumentStatus::Loading,
                operation: 0,
                pending_content: None,
                save_revision: None,
                last_external_error: None,
            });
        }
        let generation = self.documents[&normalized].generation;
        cx.notify();
        let path_for_read = normalized.clone();
        let read = cx.background_spawn(async move {
            let snapshot = persistence::read(path_for_read.clone())?;
            let source = crate::documents::decode_source(&snapshot.bytes).map_err(|error| error.to_string())?;
            Ok::<_, String>((snapshot, source))
        });
        cx.spawn(async move |this, cx| {
            let result = read.await;
            let _ = this.update(cx, |store, cx| store.finish_open(normalized, generation, result, cx));
        }).detach();
    }

    pub fn get(&self, path: &Path) -> Option<Entity<PageDocument>> {
        let normalized = self.aliases.get(path).cloned().unwrap_or_else(|| project::normalize_document_path(path));
        match &self.documents.get(&normalized)?.status {
            DocumentStatus::Ready { page, .. } => Some(page.clone()),
            _ => None,
        }
    }

    pub fn has_composition(&self, cx: &App) -> bool {
        self.documents.values().any(|entry| match &entry.status {
            DocumentStatus::Ready { page, .. } => page.read(cx).has_composition(),
            DocumentStatus::Loading | DocumentStatus::Failed(_) => false,
        })
    }

    pub fn open_state(&self, path: &Path) -> PageOpenState {
        let normalized = self.aliases.get(path).cloned().unwrap_or_else(|| project::normalize_document_path(path));
        match self.documents.get(&normalized).map(|entry| &entry.status) {
            None => PageOpenState::NotOpened,
            Some(DocumentStatus::Loading) => PageOpenState::Loading,
            Some(DocumentStatus::Ready { .. }) => PageOpenState::Ready,
            Some(DocumentStatus::Failed(error)) => PageOpenState::Failed(error.clone()),
        }
    }

    pub fn save(&mut self, path: PathBuf, cx: &mut Context<Self>) -> Result<(), PageError> {
        let normalized = self.aliases.get(&path).cloned().unwrap_or_else(|| project::normalize_document_path(path.clone()));
        if self.pending_tasks.contains(&normalized) { return Err(PageError::State("请等待当前保存完成".into())); }
        let entry = self.documents.get_mut(&normalized).ok_or(PageError::NotReady)?;
        let page = match &entry.status { DocumentStatus::Ready { page, .. } => page.clone(), _ => return Err(PageError::NotReady) };
        let token = entry.operation.wrapping_add(1).max(1);
        let lifetime = entry.lifetime;
        let queue = self.queue.clone();
        let path = normalized.clone();
        let captured = page.update(cx, |page, cx| {
            if page.has_composition() { return Err(PageError::State("请先确认或取消输入法组合文本".into())); }
            if page.is_busy() { return Err(PageError::Busy); }
            if page.save_state.is_saving() { return Ok(None); }
            let page_path = page.path_buf().ok_or_else(|| PageError::State("文档没有可保存的路径".into()))?;
            let revision = page.revision();
            let baseline = page.save_state.baseline().cloned();
            let rope = page.source_text().clone();
            if !page.save_state_mut().begin_save(revision) { return Err(PageError::Busy); }
            cx.notify();
            Ok(Some((page_path, revision, baseline, rope)))
        }).map_err(|error| PageError::State(error.to_string()))?;
        let Some((page_path, revision, baseline, rope)) = captured else { return Ok(()); };
        entry.operation = token;
        entry.pending_content = None;
        entry.save_revision = Some(revision);
        let bytes_task = cx.background_spawn(async move {
            let bytes = persistence::rope_bytes(&rope);
            let hash = *blake3::hash(&bytes).as_bytes();
            let len = bytes.len() as u64;
            (bytes, hash, len)
        });
        cx.spawn(async move |this, cx| {
            let (bytes, hash, len) = bytes_task.await;
            let current = this.update(cx, |store, _| {
                let Some(entry) = store.documents.get_mut(&path) else { return false };
                if entry.lifetime != lifetime || entry.operation != token { return false; }
                entry.pending_content = Some((hash, len));
                true
            }).unwrap_or(false);
            if !current { return; }
            let result = match persistence::submit_save(&queue, page_path, baseline, revision, bytes) {
                Ok(receiver) => match cx.background_spawn(async move { receiver.recv() }).await {
                    Ok(result) => result.map_err(|error| error.to_string()),
                    Err(error) => Err(error.to_string()),
                },
                Err(error) => Err(error.to_string()),
            };
            let _ = this.update(cx, |store, cx| store.finish_save(path, lifetime, token, result, cx));
        }).detach();
        Ok(())
    }

    pub fn commit_task(&mut self, intent: TaskIntent, cx: &mut Context<Self>) -> Result<(), PageError> {
        let path = self.aliases.get(&intent.path).cloned().unwrap_or_else(|| project::normalize_document_path(intent.path.clone()));
        if self.pending_tasks.contains(&path) { return Err(PageError::State("请等待当前保存完成".into())); }
        let page = match self.documents.get(&path).map(|entry| &entry.status) {
            Some(DocumentStatus::Ready { page, .. }) => Some(page.clone()),
            _ => None,
        };
        let prepared = task_commit::prepare_task(page.as_ref(), cx)?;
        self.pending_tasks.insert(path.clone());
        let queue = self.queue.clone();
        let operation = task_commit::commit_task(queue, path.clone(), intent, prepared);
        let task = cx.background_spawn(operation);
        let entry_lifetime = self.documents.get(&path).map(|entry| entry.lifetime).unwrap_or(0);
        cx.spawn(async move |this, cx| {
            let result = task.await.map_err(|error| error.to_string());
            let _ = this.update(cx, |store, cx| {
                store.pending_tasks.remove(&path);
                let reopen = store.documents.get(&path).is_some_and(|entry| matches!(&entry.status, DocumentStatus::Loading));
                let matches_lifetime = store.documents.get(&path).map(|entry| entry.lifetime).unwrap_or(0) == entry_lifetime;
                if reopen {
                    if let Some(entry) = store.documents.get_mut(&path) {
                        entry.status = DocumentStatus::Failed("task completed while page was opening".into());
                    }
                }
                if !matches_lifetime {
                    match result {
                        Ok(_) => cx.emit(DocumentEvent::TaskCommitted(path.clone())),
                        Err(message) => cx.emit(DocumentEvent::Failed { path: path.clone(), message }),
                    }
                    if reopen { store.open(path.clone(), cx); }
                    return;
                }
                if let Some(page) = store.get(&path) {
                    match task_commit::finish_task(&page, result, cx) {
                        Ok(()) => cx.emit(DocumentEvent::TaskCommitted(path.clone())),
                        Err(message) => cx.emit(DocumentEvent::Failed { path: path.clone(), message }),
                    }
                } else {
                    match result {
                        Ok(_) => cx.emit(DocumentEvent::TaskCommitted(path.clone())),
                        Err(message) => cx.emit(DocumentEvent::Failed { path: path.clone(), message }),
                    }
                }
                if reopen { store.open(path.clone(), cx); }
            });
        }).detach();
        Ok(())
    }

    pub fn first_dirty(&self, cx: &App) -> Option<PathBuf> {
        self.documents.iter().find_map(|(path, entry)| match &entry.status {
            DocumentStatus::Ready { page, .. } if page.read(cx).is_dirty() => Some(path.clone()),
            _ => None,
        })
    }

    pub fn has_pending_work(&self, cx: &App) -> bool {
        !self.pending_tasks.is_empty() || self.documents.values().any(|entry| match &entry.status {
            DocumentStatus::Ready { page, .. } => {
                page.read(cx).is_busy()
                    || matches!(page.read(cx).save_status(), crate::save_state::SaveStatus::Saving { .. })
            }
            DocumentStatus::Loading => true,
            _ => false,
        })
    }

    pub fn release(&mut self, path: PathBuf, discard: bool, cx: &mut Context<Self>) -> Result<(), PageError> {
        let normalized = self.aliases.get(&path).cloned().unwrap_or_else(|| project::normalize_document_path(path.clone()));
        if self.pending_tasks.contains(&normalized) { return Err(PageError::Busy); }
        let Some(entry) = self.documents.get(&normalized) else { return Ok(()); };
        if let DocumentStatus::Ready { page, .. } = &entry.status {
            if page.read(cx).is_busy()
                || matches!(page.read(cx).save_status(), crate::save_state::SaveStatus::Saving { .. })
            {
                return Err(PageError::Busy);
            }
            if page.read(cx).is_dirty() && !discard { return Err(PageError::State("document has unsaved changes".into())); }
        }
        self.documents.remove(&normalized);
        let aliases = self.aliases.iter().filter_map(|(alias, target)| {
            (target == &normalized).then_some(alias.clone())
        }).collect::<Vec<_>>();
        for alias in aliases { self.aliases.remove(&alias); }
        cx.emit(DocumentEvent::Released(normalized));
        Ok(())
    }

    fn finish_open(&mut self, path: PathBuf, generation: u64, result: Result<(DiskSnapshot, SourceRope), String>, cx: &mut Context<Self>) {
        if self.pending_tasks.contains(&path) { return; }
        let Some(entry) = self.documents.get_mut(&path) else { return; };
        if entry.generation != generation { return; }
        match result {
            Ok((snapshot, source)) => {
                let page = cx.new(|_| PageDocument::from_source_snapshot(source, path.clone(), &snapshot));
                let changed_path = path.clone();
                let subscription = cx.observe(&page, move |_store, _, cx| {
                    cx.emit(DocumentEvent::Changed(changed_path.clone()));
                    cx.notify();
                });
                entry.status = DocumentStatus::Ready { page, _subscription: subscription };
                cx.emit(DocumentEvent::Opened(path));
            }
            Err(message) => {
                entry.status = DocumentStatus::Failed(message.clone());
                cx.emit(DocumentEvent::Failed { path, message });
            }
        }
        cx.notify();
    }

    fn finish_save(&mut self, path: PathBuf, lifetime: u64, token: u64, result: Result<project::CommitAck, String>, cx: &mut Context<Self>) {
        let Some(entry) = self.documents.get_mut(&path) else { return; };
        if entry.lifetime != lifetime || entry.operation != token { return; }
        let page = match &entry.status { DocumentStatus::Ready { page, .. } => page.clone(), _ => return };
        let revision = entry.save_revision.take();
        entry.pending_content = None;
        let is_success = result.is_ok();
        let message = result.as_ref().err().cloned();
        let _ = page.update(cx, |page, cx| {
            match result {
                Ok(ack) => page.save_state_mut().apply_commit(ack),
                Err(message) => {
                    let request_revision = revision.unwrap_or_else(|| page.revision());
                    page.save_state_mut().apply_failure(request_revision, message);
                }
            }
            cx.notify();
        });
        if is_success {
            entry.last_external_error = None;
            cx.emit(DocumentEvent::Saved(path));
        } else if let Some(message) = message {
            cx.emit(DocumentEvent::Failed { path, message });
        }
    }

    fn external_targets(&self, cx: &App) -> Vec<(PathBuf, u64, u64, u64, u64)> {
        self.documents.iter().filter_map(|(path, entry)| {
            let DocumentStatus::Ready { page, .. } = &entry.status else { return None };
            let state = page.read(cx);
            if self.pending_tasks.contains(path) || state.is_busy() || state.save_state.is_saving() {
                return None;
            }
            Some((path.clone(), entry.generation, entry.lifetime, entry.operation, state.revision()))
        }).collect()
    }

    fn apply_external_reads(
        &mut self,
        results: Vec<(PathBuf, u64, u64, u64, u64, ExternalRead)>,
        cx: &mut Context<Self>,
    ) {
        for (path, generation, lifetime, operation, revision, result) in results {
            let Some(entry) = self.documents.get(&path) else { continue };
            if entry.generation != generation || entry.lifetime != lifetime || entry.operation != operation { continue; }
            let pending_content = entry.pending_content;
            let Some(DocumentStatus::Ready { page, .. }) = self.documents.get(&path).map(|entry| &entry.status) else { continue };
            let page = page.clone();
            let (baseline, current_revision, task_pending, saving) = {
                let state = page.read(cx);
                (
                    state.save_state.baseline().cloned(),
                    state.revision(),
                    state.is_busy(),
                    state.save_state.is_saving(),
                )
            };
            if task_pending || saving || current_revision != revision { continue; }
            match result {
                ExternalRead::Snapshot(snapshot, source) => {
                    if let Some(entry) = self.documents.get_mut(&path) {
                        entry.last_external_error = None;
                    }
                    if baseline.as_ref().is_some_and(|base| base.same_content(&snapshot.version))
                        || pending_content.is_some_and(|(hash, len)| hash == snapshot.version.blake3 && len == snapshot.version.len)
                    {
                        let _ = page.update(cx, |page, _| page.refresh_external_baseline(&snapshot));
                        continue;
                    }
                    match page.update(cx, |page, cx| {
                        page.replace_external_source(&snapshot, source);
                        cx.notify();
                        Ok::<_, String>(())
                    }) {
                        Ok(()) => {}
                        Err(error) => cx.emit(DocumentEvent::Failed { path, message: error }),
                    }
                }
                ExternalRead::Missing => {
                    let message = "文档在磁盘上已不存在".to_string();
                    let Some(entry) = self.documents.get_mut(&path) else { continue };
                    if entry.last_external_error.as_deref() == Some(&message) { continue; }
                    entry.last_external_error = Some(message.clone());
                    if !page.read(cx).is_dirty() {
                        let _ = page.update(cx, |page, cx| {
                            page.mark_external_missing();
                            cx.notify();
                        });
                    }
                    cx.emit(DocumentEvent::Failed { path, message });
                }
                ExternalRead::Failed(message) => {
                    let Some(entry) = self.documents.get_mut(&path) else { continue };
                    if entry.last_external_error.as_deref() == Some(&message) { continue; }
                    entry.last_external_error = Some(message.clone());
                    if !page.read(cx).is_dirty() {
                        let _ = page.update(cx, |page, cx| {
                            page.mark_external_read_error(message.clone());
                            cx.notify();
                        });
                    }
                    cx.emit(DocumentEvent::Failed { path, message });
                }
            }
        }
    }
}

impl EventEmitter<DocumentEvent> for DocumentStore {}
