//! Task 局部提交的重基与补丁逻辑（计划14）。
//!
//! 仅操作磁盘 baseline，生成只改目标字段的最小 B' 。
//! 不保存全文；无法安全定位时返回错误。
//! 配合 write_queue 的 task 提交请求使用。
//!
//! 锚点使用来源范围（checkbox 或字段 span）+ 结构身份；重复标题 ≠ 同一任务。

use crate::model::{ByteOffset, ByteRange, DocumentSession};
use crate::serialize::{TextPatch, apply_patches, fnv1a64};
use crate::task::{Task, TaskDateField, TaskItemInfo, parse_task, task_items};

/// 任务锚点：用于唯一定位任务（即使标题重复）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TaskAnchor {
    pub item_range: ByteRange,
    pub checkbox_range: Option<ByteRange>,
}

/// Task 字段更新值。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskFieldUpdate {
    Complete {
        checked: bool,
    },
    SetDate {
        which: TaskDateField,
        value: Option<String>,
    },
    SetPriority {
        value: Option<u8>,
    },
}

/// Task 提交错误。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskCommitError {
    CannotLocateUnique { reason: String },
    OnlyInWorking,
    BaselineMismatch,
    InvalidFieldSpan,
    PatchFailed(String),
    Other(String),
}

impl std::fmt::Display for TaskCommitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CannotLocateUnique { reason } => {
                write!(f, "cannot locate unique task: {}", reason)
            }
            Self::OnlyInWorking => write!(f, "task only exists in unsaved working copy"),
            Self::BaselineMismatch => write!(f, "baseline mismatch"),
            Self::InvalidFieldSpan => write!(f, "invalid field span"),
            Self::PatchFailed(s) => write!(f, "patch failed: {}", s),
            Self::Other(s) => write!(f, "task commit: {}", s),
        }
    }
}

impl std::error::Error for TaskCommitError {}

pub fn locate_task_by_anchor(
    base: &DocumentSession,
    anchor: &TaskAnchor,
) -> Result<TaskItemInfo, TaskCommitError> {
    let items = task_items(base.tree());
    for item in &items {
        let matches = match anchor.checkbox_range {
            Some(cb) => item.checkbox_range == Some(cb),
            None => base
                .tree()
                .node(item.node)
                .source
                .is_some_and(|src| src.start == anchor.item_range.start),
        };
        if matches {
            return Ok(*item);
        }
    }
    Err(TaskCommitError::CannotLocateUnique {
        reason: "no match".into(),
    })
}

pub fn apply_task_update_to_baseline(
    base: &DocumentSession,
    anchor: &TaskAnchor,
    update: &TaskFieldUpdate,
) -> Result<Vec<u8>, TaskCommitError> {
    let base_bytes = base.bytes();
    let patch = plan_task_patch_with_bytes(base, &base_bytes, anchor, update)?;
    apply_patches(&base_bytes, &[patch])
        .map_err(|e| TaskCommitError::PatchFailed(format!("{:?}", e)))
}

pub(crate) fn plan_task_patch(
    base: &DocumentSession,
    anchor: &TaskAnchor,
    update: &TaskFieldUpdate,
) -> Result<TextPatch, TaskCommitError> {
    let base_bytes = base.bytes();
    plan_task_patch_with_bytes(base, &base_bytes, anchor, update)
}

fn plan_task_patch_with_bytes(
    base: &DocumentSession,
    base_bytes: &[u8],
    anchor: &TaskAnchor,
    update: &TaskFieldUpdate,
) -> Result<TextPatch, TaskCommitError> {
    let item = locate_task_by_anchor(base, anchor)?;
    let task = parse_task(base, &item).ok_or(TaskCommitError::CannotLocateUnique {
        reason: "parse fail".into(),
    })?;
    if matches!(update, TaskFieldUpdate::SetPriority { value: Some(p) } if !(1..=9).contains(p)) {
        return Err(TaskCommitError::Other("priority must be 1..=9".into()));
    }
    if let TaskFieldUpdate::SetDate { value: Some(value), .. } = update
        && crate::task::valid_date(value).is_none()
    {
        return Err(TaskCommitError::Other("invalid task date".into()));
    }
    if !task.diagnostics.is_empty() {
        return Err(TaskCommitError::Other(task.diagnostics.join("; ")));
    }
    let item_source = base.source_slice(item.node).ok_or(TaskCommitError::InvalidFieldSpan)?;
    let first_line = item_source.text.split(['\r', '\n']).next().ok_or(TaskCommitError::InvalidFieldSpan)?;
    let insert_at = ByteOffset(item_source.range.start.0 + first_line.len() as u32);
    match update {
        TaskFieldUpdate::Complete { checked } => compute_checkbox_patch(&item, *checked, &base_bytes),
        TaskFieldUpdate::SetDate { which, value } => compute_date_patch(&task, *which, value.as_deref(), &base_bytes, insert_at),
        TaskFieldUpdate::SetPriority { value } => compute_priority_patch(&task, *value, &base_bytes, insert_at),
    }
}

fn compute_checkbox_patch(
    item: &TaskItemInfo,
    checked: bool,
    base: &[u8],
) -> Result<TextPatch, TaskCommitError> {
    let range = item
        .checkbox_range
        .ok_or(TaskCommitError::InvalidFieldSpan)?;
    let start = range.start.0 as usize;
    let end = range.end.0 as usize;
    if end > base.len() {
        return Err(TaskCommitError::InvalidFieldSpan);
    }
    let old = &base[start..end];
    let exp = fnv1a64(old);
    let new_c = if checked { 'x' } else { ' ' };
    Ok(TextPatch {
        range,
        expected_hash: exp,
        replacement: new_c.to_string(),
    })
}

fn compute_date_patch(
    task: &Task,
    which: TaskDateField,
    value: Option<&str>,
    base: &[u8],
    insert_at: ByteOffset,
) -> Result<TextPatch, TaskCommitError> {
    let current = match which {
        TaskDateField::Start => &task.start,
        TaskDateField::Due => &task.due,
    };
    let fname = match which {
        TaskDateField::Start => "start",
        TaskDateField::Due => "due",
    };
    if let Some(val) = value {
        if let Some((_, r)) = current {
            let new_text = format!("[{}:: {}]", fname, val);
            let s = r.start.0 as usize;
            let e = r.end.0 as usize;
            let old = &base[s..e];
            Ok(TextPatch {
                range: *r,
                expected_hash: fnv1a64(old),
                replacement: new_text,
            })
        } else {
            let ins = insert_at;
            let new_text = format!(" [{}:: {}]", fname, val);
            Ok(TextPatch {
                range: ByteRange {
                    start: ins,
                    end: ins,
                },
                expected_hash: fnv1a64(b""),
                replacement: new_text,
            })
        }
    } else if let Some((_, r)) = current {
        let s = r.start.0 as usize;
        let e = r.end.0 as usize;
        let old = &base[s..e];
        Ok(TextPatch {
            range: *r,
            expected_hash: fnv1a64(old),
            replacement: String::new(),
        })
    } else {
        Ok(TextPatch {
            range: ByteRange { start: insert_at, end: insert_at },
            expected_hash: fnv1a64(b""),
            replacement: String::new(),
        })
    }
}

fn compute_priority_patch(
    task: &Task,
    value: Option<u8>,
    base: &[u8],
    insert_at: ByteOffset,
) -> Result<TextPatch, TaskCommitError> {
    if let Some(p) = value {
        if let Some((_, r)) = &task.priority {
            let new_text = format!("[priority:: {}]", p);
            let s = r.start.0 as usize;
            let e = r.end.0 as usize;
            let old = &base[s..e];
            Ok(TextPatch {
                range: *r,
                expected_hash: fnv1a64(old),
                replacement: new_text,
            })
        } else {
            let ins = insert_at;
            let new_text = format!(" [priority:: {}]", p);
            Ok(TextPatch {
                range: ByteRange {
                    start: ins,
                    end: ins,
                },
                expected_hash: fnv1a64(b""),
                replacement: new_text,
            })
        }
    } else if let Some((_, r)) = &task.priority {
        let s = r.start.0 as usize;
        let e = r.end.0 as usize;
        let old = &base[s..e];
        Ok(TextPatch {
            range: *r,
            expected_hash: fnv1a64(old),
            replacement: String::new(),
        })
    } else {
        Ok(TextPatch {
            range: ByteRange { start: insert_at, end: insert_at },
            expected_hash: fnv1a64(b""),
            replacement: String::new(),
        })
    }
}
