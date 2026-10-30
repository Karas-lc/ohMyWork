use std::{path::PathBuf, sync::mpsc::Receiver};

use project::{AtomicWriteError, CommitAck, DiskSnapshot, QueueError, WriteQueue};

use super::PageError;

pub(super) fn read(path: PathBuf) -> Result<DiskSnapshot, String> {
    project::read_snapshot(path).map_err(|error| error.to_string())
}

pub(super) fn submit_save(
    queue: &WriteQueue,
    path: PathBuf,
    baseline: Option<project::DiskVersion>,
    revision: u64,
    bytes: Vec<u8>,
) -> Result<Receiver<Result<CommitAck, AtomicWriteError>>, PageError> {
    queue
        .submit_save(
            project::DocumentKey::new(path.clone()),
            path,
            baseline,
            bytes,
            revision,
        )
        .map_err(|error: QueueError| PageError::State(format!("后台保存队列不可用：{error:?}")))
}

pub(super) fn submit_task_save(
    queue: &WriteQueue,
    path: PathBuf,
    baseline: project::DiskVersion,
    revision: u64,
    bytes: Vec<u8>,
) -> Result<Receiver<Result<CommitAck, AtomicWriteError>>, PageError> {
    queue
        .submit_task_save(
            project::DocumentKey::new(path.clone()),
            path,
            Some(baseline),
            bytes,
            revision,
        )
        .map_err(|error| PageError::State(format!("后台保存队列不可用：{error:?}")))
}


pub(super) fn rope_bytes(rope: &super::SourceRope) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(rope.len());
    for chunk in rope.chunks() {
        bytes.extend_from_slice(chunk.as_bytes());
    }
    bytes
}
