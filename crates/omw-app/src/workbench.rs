use crate::editor::EditorDocument;
use gpui_kit::{Entity, Global};
use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    path::PathBuf,
    rc::Rc,
    sync::Arc,
};

#[derive(Clone)]
pub enum WorkbenchEvent {
    Open(PathBuf),
    Close(PathBuf),
    Task(crate::ui::tasks::TaskIntent),
    ShowTasks,
    ShowPi,
}
#[derive(Clone)]
pub struct Workbench {
    pub root: PathBuf,
    pub app_data: omw_local::AppDataPaths,
    pub documents: Rc<RefCell<HashMap<PathBuf, Entity<EditorDocument>>>>,
    pub panels: Rc<RefCell<HashMap<PathBuf, Entity<crate::ui::panes::MarkdownPanel>>>>,
    pub active: Rc<RefCell<Option<PathBuf>>>,
    pub events: Rc<RefCell<VecDeque<WorkbenchEvent>>>,
    pub persist_tx: std::sync::mpsc::Sender<PersistChange>,
    pub recent: Rc<RefCell<Vec<PathBuf>>>,
    pub pi_status: Rc<RefCell<String>>,
    pub tasks: Rc<RefCell<Vec<crate::ui::tasks::TaskSummary>>>,
    pub writes: omw_local::WriteQueue,
    pub runtime: Arc<tokio::runtime::Runtime>,
}
impl Global for Workbench {}
impl Workbench {
    pub fn new(root: PathBuf, app_data: omw_local::AppDataPaths) -> Result<Self, String> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| format!("Pi/preview runtime: {e}"))?;
        let (persist_tx, persist_rx) = std::sync::mpsc::channel::<PersistChange>();
        let persist_root = root.clone();
        let paths = app_data.clone();
        std::thread::Builder::new()
            .name("ohmywork-ui-state".into())
            .spawn(move || {
                let mut last_result = Ok(());
                while let Ok(change) = persist_rx.recv() {
                    if let PersistChange::Flush(reply) = change {
                        let _ = reply.send(last_result.clone());
                        continue;
                    }
                    let mut state = crate::state::load_ui_state(&paths, &persist_root);
                    match change {
                        PersistChange::Dock(dock) => state.dock = Some(dock),
                        PersistChange::Canvas(canvas) => {
                            state.canvas =
                                serde_json::to_value(crate::home::CanvasPersist::new(canvas)).ok()
                        }
                        PersistChange::Flush(_) => unreachable!(),
                    }
                    last_result = crate::state::save_ui_state(&paths, &persist_root, &state);
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            root,
            app_data,
            documents: Default::default(),
            panels: Default::default(),
            active: Default::default(),
            events: Default::default(),
            writes: omw_local::WriteQueue::new(2),
            runtime: Arc::new(runtime),
            persist_tx,
            recent: Default::default(),
            pi_status: Default::default(),
            tasks: Default::default(),
        })
    }
    pub fn persist(&self, change: PersistChange) {
        let _ = self.persist_tx.send(change);
    }
    pub fn emit(&self, event: WorkbenchEvent) {
        self.events.borrow_mut().push_back(event);
    }
}

pub enum PersistChange {
    Dock(crate::ui::panes::PersistedDockState),
    Canvas(crate::home::CanvasState),
    Flush(std::sync::mpsc::Sender<Result<(), String>>),
}
