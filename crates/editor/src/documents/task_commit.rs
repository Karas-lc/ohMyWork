use std::path::PathBuf;

use gpui_kit::{Context, Entity};
use project::{CommitAck, DiskVersion, WriteQueue};

use crate::{DocumentSession, TaskAnchor, TaskFieldUpdate, TaskCommitError, locate_task_by_anchor, parse, rebase::plan_task_patch, serialize::apply_patches};

use super::{DocumentStore, PageDocument, PageError, SourceSession, persistence};

#[derive(Clone, Debug)]
pub struct TaskIntent {
    pub path: PathBuf,
    pub version: DiskVersion,
    pub anchor: TaskAnchor,
    pub update: TaskFieldUpdate,
}

pub(super) struct PreparedTask {
    pub revision: u64,
    pub baseline: Option<DiskVersion>,
    pub working: Option<SourceSession>,
}

pub(super) struct TaskCommitResult {
    pub ack: CommitAck,
    pub merged: Option<SourceSession>,
    pub merged_dirty: bool,
}

pub(super) fn prepare_task(
    page: Option<&Entity<PageDocument>>,
    cx: &mut Context<DocumentStore>,
) -> Result<PreparedTask, PageError> {
    let Some(page) = page else {
        return Ok(PreparedTask { revision: 0, baseline: None, working: None });
    };
    page.update(cx, |page, cx| {
        if page.has_composition() { return Err(PageError::State("请先确认或取消输入法组合文本".into())); }
        if page.is_busy() || page.save_state.is_saving() { return Err(PageError::State("请等待当前保存完成".into())); }
        page.set_task_pending(true);
        cx.notify();
        Ok(PreparedTask {
            revision: page.revision(),
            baseline: page.save_state.baseline().cloned(),
            working: Some(page.source.clone()),
        })
    }).map_err(|error| PageError::State(error.to_string()))
}

pub(super) fn finish_task(
    page: &Entity<PageDocument>,
    result: Result<TaskCommitResult, String>,
    cx: &mut Context<DocumentStore>,
) -> Result<(), String> {
    page.update(cx, |page, cx| {
        page.set_task_pending(false);
        match result {
            Ok(TaskCommitResult { ack, merged: Some(merged), merged_dirty }) => {
                page.source = merged;
                page.accept_task_commit(ack, merged_dirty);
                cx.notify();
                Ok(())
            }
            Ok(TaskCommitResult { .. }) => { cx.notify(); Ok(()) }
            Err(message) => { cx.notify(); Err(message) }
        }
    }).map_err(|error| error.to_string())
}

pub(super) async fn commit_task(
    queue: WriteQueue,
    path: PathBuf,
    intent: TaskIntent,
    prepared: PreparedTask,
) -> Result<TaskCommitResult, String> {
    let snapshot = persistence::read(path.clone())?;
    if !snapshot.version.same_content(&intent.version)
        || (prepared.working.is_some()
            && prepared.baseline.as_ref().is_none_or(|baseline| !snapshot.version.same_content(baseline)))
    {
        return Err("任务来源已变化；请刷新后重试".into());
    }
    let parsed: DocumentSession = parse(&snapshot.bytes).map_err(|error| error.to_string())?;
    let item = locate_task_by_anchor(&parsed, &intent.anchor).map_err(|error| error.to_string())?;
    let task_range = parsed.tree().node(item.node).source.ok_or_else(|| TaskCommitError::InvalidFieldSpan.to_string())?;
    let task_range = task_range.start.0 as usize..task_range.end.0 as usize;
    let patch = plan_task_patch(&parsed, &intent.anchor, &intent.update).map_err(|error| error.to_string())?;
    let bytes = apply_patches(&snapshot.bytes, std::slice::from_ref(&patch)).map_err(|error| error.to_string())?;
    if bytes.len() > u32::MAX as usize { return Err("任务修改后文档超过 u32 字节范围".into()); }
    let merged = prepared.working.as_ref().map(|working| {
        working.with_committed_task_patch(&snapshot.bytes, task_range, &patch).map_err(|error| error.to_string())
    }).transpose()?;
    let merged_dirty = if let Some(working) = &merged {
        let working_bytes = persistence::rope_bytes(working.source());
        working_bytes.len() as u64 != bytes.len() as u64 || blake3::hash(&working_bytes) != blake3::hash(&bytes)
    } else { false };
    let receiver = persistence::submit_task_save(
        &queue,
        path,
        snapshot.version,
        prepared.revision,
        bytes,
    ).map_err(|error| error.to_string())?;
    let ack = receiver.recv().map_err(|error| error.to_string())?.map_err(|error| error.to_string())?;
    Ok(TaskCommitResult { ack, merged, merged_dirty })
}
