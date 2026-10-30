use std::{cell::RefCell, collections::HashSet, fmt, path::{Path, PathBuf}, rc::Rc};

use gpui_kit::{Context, EntityId};
use project::{CommitAck, DiskSnapshot};

use crate::DocumentSession;
use super::{PageSnapshot, SourceRope, SourceSelection, source::{SourceSession, valid_selection}};

#[derive(Clone, Debug)]
pub enum PageCommand {
    ReplaceSource { revision: u64, text: SourceRope, selection_before: SourceSelection, selection_after: SourceSelection },
    Undo,
    Redo,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageEditOutcome { pub revision: u64, pub selection: Option<SourceSelection> }

#[derive(Debug)]
pub enum PageError { NotReady, Busy, StaleRevision, InvalidRange, State(String) }
impl fmt::Display for PageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { match self {
        Self::NotReady => f.write_str("document is not ready"), Self::Busy => f.write_str("document is busy"),
        Self::StaleRevision => f.write_str("document changed; refresh and retry"), Self::InvalidRange => f.write_str("invalid text range"),
        Self::State(message) => f.write_str(message),
    }}
}
impl std::error::Error for PageError {}

pub struct PageDocument {
    pub(super) source: SourceSession,
    pub(super) save_state: crate::save_state::DocumentSaveState,
    path: Option<PathBuf>,
    snapshot: RefCell<Option<Rc<PageSnapshot>>>,
    pub(super) task_pending: bool,
    composing: HashSet<EntityId>,
    external_epoch: u64,
}

impl PageDocument {
    pub fn from_document(document: &DocumentSession) -> Self {
        let text = document.text().to_string();
        Self::from_source(SourceRope::from_str(&text), None, crate::save_state::DocumentSaveState::new(None))
    }
    pub fn from_source_snapshot(source: SourceRope, path: PathBuf, snapshot: &DiskSnapshot) -> Self {
        Self::from_source(source, Some(path), crate::save_state::DocumentSaveState::from_snapshot(snapshot))
    }
    fn from_source(source: SourceRope, path: Option<PathBuf>, save_state: crate::save_state::DocumentSaveState) -> Self {
        Self { source: SourceSession::new(source), save_state, path, snapshot: RefCell::new(None), task_pending: false, composing: HashSet::new(), external_epoch: 0 }
    }
    pub fn path(&self) -> Option<&Path> { self.path.as_deref() }
    pub fn snapshot(&self) -> Result<Rc<PageSnapshot>, PageError> {
        let revision = self.save_state.revision();
        if let Some(snapshot) = self.snapshot.borrow().as_ref() {
            if snapshot.revision() == revision { return Ok(snapshot.clone()); }
        }
        let snapshot = Rc::new(PageSnapshot::new(self.source.source().clone(), revision, self.external_epoch));
        *self.snapshot.borrow_mut() = Some(snapshot.clone());
        Ok(snapshot)
    }
    pub fn revision(&self) -> u64 { self.save_state.revision() }
    pub fn is_dirty(&self) -> bool { self.save_state.is_dirty() }
    pub fn is_busy(&self) -> bool { self.task_pending }
    pub fn save_status(&self) -> &crate::save_state::SaveStatus { self.save_state.status() }
    pub fn bytes(&self) -> Result<Vec<u8>, PageError> { Ok(super::persistence::rope_bytes(self.source.source())) }
    pub fn has_composition(&self) -> bool { !self.composing.is_empty() }
    pub fn set_composing(&mut self, surface: EntityId, composing: bool) -> bool {
        if composing { self.composing.insert(surface) } else { self.composing.remove(&surface) }
    }
    pub fn apply(&mut self, command: PageCommand, cx: &mut Context<Self>) -> Result<PageEditOutcome, PageError> {
        if self.task_pending { return Err(PageError::Busy); }
        let selection = match command {
            PageCommand::ReplaceSource { revision, text, selection_before, selection_after } => {
                if revision != self.save_state.revision() { return Err(PageError::StaleRevision); }
                if text.len() > u32::MAX as usize { return Err(PageError::State(crate::ParseError::DocumentTooLarge { size: text.len() }.to_string())); }
                if !valid_selection(&text, selection_after) || !valid_selection(self.source.source(), selection_before) { return Err(PageError::InvalidRange); }
                self.source.replace(text, selection_before, selection_after);
                self.mark_edited();
                cx.notify();
                Some(selection_after)
            }
            PageCommand::Undo => {
                let Some(selection) = self.source.undo() else { return Ok(PageEditOutcome { revision: self.revision(), selection: None }); };
                self.mark_edited(); cx.notify(); Some(selection)
            }
            PageCommand::Redo => {
                let Some(selection) = self.source.redo() else { return Ok(PageEditOutcome { revision: self.revision(), selection: None }); };
                self.mark_edited(); cx.notify(); Some(selection)
            }
        };
        Ok(PageEditOutcome { revision: self.save_state.revision(), selection })
    }
    fn mark_edited(&mut self) { self.save_state.mark_edited(); *self.snapshot.borrow_mut() = None; }
    pub(super) fn set_task_pending(&mut self, value: bool) { self.task_pending = value; }
    pub(super) fn save_state_mut(&mut self) -> &mut crate::save_state::DocumentSaveState { &mut self.save_state }
    pub(super) fn accept_task_commit(&mut self, ack: CommitAck, merged_dirty: bool) {
        let revision = self.save_state.mark_edited();
        self.save_state.accept_task_commit(ack.new_version, revision, merged_dirty);
        *self.snapshot.borrow_mut() = None;
    }
    pub(super) fn replace_external_source(&mut self, snapshot: &DiskSnapshot, source: SourceRope) {
        self.source.reset(source);
        self.composing.clear();
        self.external_epoch = self.external_epoch.wrapping_add(1);
        *self.snapshot.borrow_mut() = None;
        self.save_state.reload(snapshot);
    }
    pub(crate) fn source_text(&self) -> &SourceRope { self.source.source() }
    pub(super) fn path_buf(&self) -> Option<PathBuf> { self.path.clone() }
    pub(super) fn refresh_external_baseline(&mut self, snapshot: &DiskSnapshot) { self.save_state.refresh_baseline(snapshot); }
    pub(super) fn mark_external_missing(&mut self) { self.save_state.mark_missing(); }
    pub(super) fn mark_external_read_error(&mut self, message: String) { self.save_state.mark_read_error(message); }
}
