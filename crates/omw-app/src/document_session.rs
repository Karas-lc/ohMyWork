use std::{fmt, path::PathBuf, sync::mpsc::Receiver};

use omw_local::{
    AtomicWriteError, CommitAck, DiskSnapshot, DiskVersion, DocumentKey, QueueError, WriteQueue,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveStatus {
    Clean,
    Dirty,
    Saving { request_revision: u64 },
    Failed { revision: u64, message: String },
    ExternalChanged,
    Missing,
    ExternalReadError { message: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingCommit {
    request_revision: u64,
    content_hash: Option<[u8; 32]>,
    content_len: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentSaveState {
    baseline: Option<DiskVersion>,
    revision: u64,
    status: SaveStatus,
    in_flight: Option<PendingCommit>,
}

impl DocumentSaveState {
    pub fn new(baseline: Option<DiskVersion>) -> Self {
        Self {
            baseline,
            revision: 0,
            status: SaveStatus::Clean,
            in_flight: None,
        }
    }

    pub fn from_snapshot(snapshot: &DiskSnapshot) -> Self {
        Self::new(Some(snapshot.version.clone()))
    }

    pub fn baseline(&self) -> Option<&DiskVersion> {
        self.baseline.as_ref()
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn status(&self) -> &SaveStatus {
        &self.status
    }

    pub fn is_dirty(&self) -> bool {
        matches!(
            self.status,
            SaveStatus::Dirty | SaveStatus::Saving { .. } | SaveStatus::Failed { .. }
        )
    }

    pub fn mark_edited(&mut self, revision: u64) {
        self.revision = self.revision.max(revision);
        self.status = SaveStatus::Dirty;
    }

    pub fn begin_save(&mut self, revision: u64) -> bool {
        self.begin_save_with_content(revision, None)
    }

    pub fn begin_save_for_content(&mut self, revision: u64, content: &[u8]) -> bool {
        self.begin_save_with_content(revision, Some(content))
    }

    fn begin_save_with_content(&mut self, revision: u64, content: Option<&[u8]>) -> bool {
        if revision != self.revision
            || !matches!(self.status, SaveStatus::Dirty | SaveStatus::Failed { .. })
        {
            return false;
        }
        self.status = SaveStatus::Saving {
            request_revision: revision,
        };
        self.in_flight = Some(PendingCommit {
            request_revision: revision,
            content_hash: content.map(|bytes| *blake3::hash(bytes).as_bytes()),
            content_len: content.map(|bytes| bytes.len() as u64),
        });
        true
    }

    pub fn apply_commit(&mut self, ack: CommitAck) {
        let Some(pending) = self.in_flight.take() else {
            return;
        };
        if pending.request_revision != ack.request_revision {
            self.in_flight = Some(pending);
            return;
        }
        self.baseline = Some(ack.new_version);
        self.status = if self.revision == ack.request_revision {
            SaveStatus::Clean
        } else {
            SaveStatus::Dirty
        };
    }

    pub fn apply_failure(&mut self, request_revision: u64, message: String) {
        let Some(pending) = self.in_flight.take() else {
            return;
        };
        if pending.request_revision != request_revision {
            self.in_flight = Some(pending);
            return;
        }
        if self.revision == request_revision {
            self.status = SaveStatus::Failed {
                revision: request_revision,
                message,
            };
        } else {
            self.status = SaveStatus::Dirty;
        }
    }

    pub fn apply_external_change(&mut self) {
        self.in_flight = None;
        self.status = SaveStatus::ExternalChanged;
    }

    fn pending_matches(&self, snapshot: &DiskSnapshot) -> bool {
        self.in_flight.as_ref().is_some_and(|pending| {
            pending
                .content_hash
                .zip(pending.content_len)
                .is_some_and(|(hash, len)| {
                    hash == snapshot.version.blake3 && len == snapshot.version.len
                })
        })
    }

    fn refresh_baseline(&mut self, snapshot: &DiskSnapshot) {
        self.baseline = Some(snapshot.version.clone());
    }

    pub fn reload(&mut self, snapshot: &DiskSnapshot) {
        self.refresh_baseline(snapshot);
        self.in_flight = None;
        self.revision = self.revision.wrapping_add(1);
        self.status = SaveStatus::Clean;
    }
    pub fn accept_task_commit(&mut self, version: DiskVersion, revision: u64, dirty: bool) {
        self.baseline = Some(version);
        self.revision = revision;
        self.in_flight = None;
        self.status = if dirty {
            SaveStatus::Dirty
        } else {
            SaveStatus::Clean
        };
    }

    fn mark_missing(&mut self) {
        if matches!(self.status, SaveStatus::Missing) {
            return;
        }
        self.in_flight = None;
        self.revision = self.revision.wrapping_add(1);
        self.status = SaveStatus::Missing;
    }

    fn mark_read_error(&mut self, message: String) {
        self.in_flight = None;
        self.status = SaveStatus::ExternalReadError { message };
    }

    pub fn discard(&mut self) {
        self.in_flight = None;
        self.revision = self.revision.wrapping_add(1);
        self.status = SaveStatus::Clean;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExternalEvent {
    Snapshot(DiskSnapshot),
    Deleted,
    ReadError(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExternalEventOutcome {
    BaselineRefreshed,
    SelfCommitObserved,
    Reloaded(DiskSnapshot),
    Missing,
    ReadError(String),
}

pub fn reduce_external_event(
    state: &mut DocumentSaveState,
    event: ExternalEvent,
) -> ExternalEventOutcome {
    match event {
        ExternalEvent::Snapshot(snapshot) => {
            if state
                .baseline()
                .is_some_and(|baseline| baseline.same_content(&snapshot.version))
            {
                state.refresh_baseline(&snapshot);
                ExternalEventOutcome::BaselineRefreshed
            } else if state.pending_matches(&snapshot) {
                state.refresh_baseline(&snapshot);
                ExternalEventOutcome::SelfCommitObserved
            } else {
                state.reload(&snapshot);
                ExternalEventOutcome::Reloaded(snapshot)
            }
        }
        ExternalEvent::Deleted => {
            state.mark_missing();
            ExternalEventOutcome::Missing
        }
        ExternalEvent::ReadError(message) => {
            state.mark_read_error(message.clone());
            ExternalEventOutcome::ReadError(message)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseDecision {
    Save,
    Discard,
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseAction {
    Close,
    RequestSave,
    KeepOpen,
}

pub fn close_action(state: &DocumentSaveState, decision: CloseDecision) -> CloseAction {
    match decision {
        CloseDecision::Cancel => CloseAction::KeepOpen,
        CloseDecision::Discard => CloseAction::Close,
        CloseDecision::Save if matches!(state.status, SaveStatus::Clean) => CloseAction::Close,
        CloseDecision::Save if matches!(state.status, SaveStatus::Saving { .. }) => {
            CloseAction::KeepOpen
        }
        CloseDecision::Save => CloseAction::RequestSave,
    }
}

#[derive(Debug)]
pub enum SaveRequestError {
    NoPath,
    DocumentBusy,
    DocumentLockPoisoned,
    Queue(QueueError),
}

impl fmt::Display for SaveRequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoPath => formatter.write_str("文档没有可保存的路径"),
            Self::DocumentBusy => formatter.write_str("文档正在被编辑，稍后重试保存"),
            Self::DocumentLockPoisoned => formatter.write_str("文档状态锁已失效"),
            Self::Queue(error) => write!(formatter, "后台保存队列不可用：{error:?}"),
        }
    }
}

impl std::error::Error for SaveRequestError {}

pub type SaveResult = Result<CommitAck, AtomicWriteError>;

pub fn submit_save(
    queue: &WriteQueue,
    state: &mut DocumentSaveState,
    path: Option<&PathBuf>,
    content: Vec<u8>,
) -> Result<Receiver<SaveResult>, SaveRequestError> {
    let path = path.ok_or(SaveRequestError::NoPath)?;
    let request_revision = state.revision();
    if !state.begin_save_for_content(request_revision, &content) {
        return Err(SaveRequestError::Queue(QueueError::Stopped));
    }
    queue
        .submit_save(
            DocumentKey::new(path.clone()),
            path.clone(),
            state.baseline().cloned(),
            content,
            request_revision,
        )
        .map_err(|error| {
            state.apply_failure(request_revision, format!("{error:?}"));
            SaveRequestError::Queue(error)
        })
}
/// Task 局部字段提交（计划14）：使用已 patch 的最小 B' 。
/// 外部变更检测、revision 仍由 state 处理。
pub fn submit_task_save(
    queue: &WriteQueue,
    state: &mut DocumentSaveState,
    path: Option<&PathBuf>,
    patched_bytes: Vec<u8>,
) -> Result<Receiver<SaveResult>, SaveRequestError> {
    let path = path.ok_or(SaveRequestError::NoPath)?;
    let request_revision = state.revision();
    if !state.begin_save_for_content(request_revision, &patched_bytes) {
        return Err(SaveRequestError::Queue(QueueError::Stopped));
    }
    queue
        .submit_task_save(
            DocumentKey::new(path.clone()),
            path.clone(),
            state.baseline().cloned(),
            patched_bytes,
            request_revision,
        )
        .map_err(|error| {
            state.apply_failure(request_revision, format!("{error:?}"));
            SaveRequestError::Queue(error)
        })
}

pub fn apply_save_result(state: &mut DocumentSaveState, result: SaveResult) {
    match result {
        Ok(ack) => state.apply_commit(ack),
        Err(error) => {
            let Some(revision) = state
                .in_flight
                .as_ref()
                .map(|pending| pending.request_revision)
            else {
                return;
            };
            state.apply_failure(revision, error.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn temp_file() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("ohmywork-save-state-{nonce}.md"));
        fs::write(&path, b"baseline").expect("write baseline");
        path
    }

    #[test]
    fn commit_ack_cleans_only_when_no_newer_edit_exists() {
        let path = temp_file();
        let baseline = omw_local::read_snapshot(&path).expect("baseline");
        let mut state = DocumentSaveState::from_snapshot(&baseline);
        state.mark_edited(1);
        assert!(state.begin_save(1));
        state.mark_edited(2);
        let ack = omw_local::atomic_write(&path, Some(&baseline.version), b"saved-revision-one", 1)
            .expect("commit");
        state.apply_commit(ack);
        assert_eq!(state.status(), &SaveStatus::Dirty);
        assert_eq!(state.baseline().expect("new baseline").len, 18);
        let _ = fs::remove_file(path);
    }
    #[test]
    fn self_write_echo_keeps_later_input_dirty() {
        let path = temp_file();
        let baseline = omw_local::read_snapshot(&path).expect("baseline");
        let mut state = DocumentSaveState::from_snapshot(&baseline);
        state.mark_edited(1);
        assert!(state.begin_save_for_content(1, b"saved"));
        let ack =
            omw_local::atomic_write(&path, Some(&baseline.version), b"saved", 1).expect("commit");
        let echoed = omw_local::read_snapshot(&path).expect("echoed snapshot");
        assert_eq!(
            reduce_external_event(&mut state, ExternalEvent::Snapshot(echoed)),
            ExternalEventOutcome::SelfCommitObserved
        );
        state.mark_edited(2);
        state.apply_commit(ack);
        assert_eq!(state.status(), &SaveStatus::Dirty);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn close_decisions_do_not_store_a_second_dialog_state() {
        let mut state = DocumentSaveState::new(None);
        assert_eq!(
            close_action(&state, CloseDecision::Cancel),
            CloseAction::KeepOpen
        );
        assert_eq!(
            close_action(&state, CloseDecision::Discard),
            CloseAction::Close
        );
        state.mark_edited(1);
        assert_eq!(
            close_action(&state, CloseDecision::Save),
            CloseAction::RequestSave
        );
        assert_eq!(
            close_action(&state, CloseDecision::Cancel),
            CloseAction::KeepOpen
        );
    }
}
