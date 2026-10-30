use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{Arc, atomic::{AtomicBool, Ordering}},
};

use editor::documents::{DocumentEvent, DocumentStore};
pub use editor::documents::TaskIntent;
use gpui_kit::{AppContext as _, Context, Entity, Subscription};

#[derive(Clone)]
pub struct TaskSummary {
    pub title: String,
    pub checked: bool,
    pub intent: TaskIntent,
}

#[derive(Clone)]
pub struct TaskRow {
    pub path: PathBuf,
    pub version: project::DiskVersion,
    pub anchor: editor::TaskAnchor,
    pub task: editor::task::Task,
    pub in_archive: bool,
}

#[derive(Clone, Default)]
struct ReloadCancel(Arc<AtomicBool>);
impl ReloadCancel {
    fn cancel(&self) { self.0.store(true, Ordering::Release); }
    fn is_cancelled(&self) -> bool { self.0.load(Ordering::Acquire) }
}

pub struct TasksModel {
    root: PathBuf,
    documents: Entity<DocumentStore>,
    rows: Vec<TaskRow>,
    summaries: Vec<TaskSummary>,
    include_archive: bool,
    status: String,
    generation: u64,
    cancel: Option<ReloadCancel>,
    _document_subscription: Subscription,
}

impl TasksModel {
    pub fn new(root: PathBuf, documents: Entity<DocumentStore>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.subscribe(&documents, |this, _, event: &DocumentEvent, cx| {
            match event {
                DocumentEvent::TaskCommitted(_) | DocumentEvent::Changed(_) => this.reload(cx),
                DocumentEvent::Failed { message, .. } => {
                    this.status = message.clone();
                    cx.notify();
                }
                _ => {}
            }
        });
        let mut this = Self {
            root,
            documents,
            rows: Vec::new(),
            summaries: Vec::new(),
            include_archive: false,
            status: String::new(),
            generation: 0,
            cancel: None,
            _document_subscription: subscription,
        };
        this.reload(cx);
        this
    }

    pub fn rows(&self) -> &[TaskRow] { &self.rows }
    pub fn summaries(&self) -> &[TaskSummary] { &self.summaries }
    pub fn include_archive(&self) -> bool { self.include_archive }
    pub fn status(&self) -> &str { &self.status }

    pub fn set_include_archive(&mut self, include_archive: bool, cx: &mut Context<Self>) {
        if self.include_archive == include_archive { return; }
        self.include_archive = include_archive;
        self.reload(cx);
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        if let Some(cancel) = self.cancel.take() { cancel.cancel(); }
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let token = ReloadCancel::default();
        self.cancel = Some(token.clone());
        let root = self.root.clone();
        let include_archive = self.include_archive;
        self.status = "正在读取任务…".into();
        cx.notify();
        let worker_token = token.clone();

        let task = cx.background_spawn(async move {
            let report = project::scan_workspace(&root, project::ScanOptions::default())
                .map_err(|error| Some(error.to_string()))?;
            if worker_token.is_cancelled() { return Err(None); }
            let mut rows = Vec::new();
            let mut seen = HashSet::new();
            for entry in report.entries() {
                if worker_token.is_cancelled() { return Err(None); }
                if !entry.is_file()
                    || entry.is_excluded()
                    || (!include_archive && is_archive(entry.logical_path()))
                    || entry.target_path().extension().and_then(|s| s.to_str()).is_none_or(|s| !s.eq_ignore_ascii_case("md"))
                    || !seen.insert(entry.target_path().to_path_buf())
                {
                    continue;
                }
                let snapshot = project::read_snapshot(entry.target_path()).map_err(|error| Some(error.to_string()))?;
                if worker_token.is_cancelled() { return Err(None); }
                let document = editor::parse(&snapshot.bytes).map_err(|error| Some(error.to_string()))?;
                for item in editor::task::task_items(document.tree()) {
                    if worker_token.is_cancelled() { return Err(None); }
                    if let (Some(task), Some(range)) = (
                        editor::task::parse_task(&document, &item),
                        document.tree().node(item.node).source,
                    ) {
                        rows.push(TaskRow {
                            path: entry.target_path().to_path_buf(),
                            version: snapshot.version.clone(),
                            anchor: editor::TaskAnchor { item_range: range, checkbox_range: item.checkbox_range },
                            task,
                            in_archive: is_archive(entry.logical_path()),
                        });
                    }
                }
            }
            rows.sort_by(|a, b| {
                a.task.item.checked.cmp(&b.task.item.checked)
                    .then_with(|| b.task.priority.as_ref().map(|p| p.0).cmp(&a.task.priority.as_ref().map(|p| p.0)))
                    .then_with(|| a.task.due.as_ref().map(|d| d.0.as_str()).unwrap_or("9999-12-31")
                        .cmp(b.task.due.as_ref().map(|d| d.0.as_str()).unwrap_or("9999-12-31")))
                    .then_with(|| a.path.cmp(&b.path))
                    .then_with(|| a.anchor.item_range.start.cmp(&b.anchor.item_range.start))
            });
            Ok::<_, Option<String>>(rows)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation || token.is_cancelled() { return; }
                match result {
                    Ok(rows) => {
                        this.summaries = rows.iter()
                            .filter(|row| !row.in_archive && !row.task.item.checked)
                            .map(|row| TaskSummary {
                                title: row.task.title.clone(),
                                checked: row.task.item.checked,
                                intent: TaskIntent {
                                    path: row.path.clone(),
                                    version: row.version.clone(),
                                    anchor: row.anchor,
                                    update: editor::TaskFieldUpdate::Complete { checked: true },
                                },
                            })
                            .collect();
                        this.status = format!("{} 个任务", rows.len());
                        this.rows = rows;
                    }
                    Err(None) => return,
                    Err(Some(error)) => this.status = error,
                }
                cx.notify();
            });
        }).detach();
    }
    pub fn commit_task(&mut self, intent: TaskIntent, cx: &mut Context<Self>) -> Result<(), editor::PageError> {
        self.documents.update(cx, |store, cx| store.commit_task(intent, cx))
    }

}

fn is_archive(path: &std::path::Path) -> bool {
    path.components().next().is_some_and(|component| component.as_os_str() == "Archive")
}

impl Drop for TasksModel {
    fn drop(&mut self) {
        if let Some(cancel) = &self.cancel { cancel.cancel(); }
    }
}
