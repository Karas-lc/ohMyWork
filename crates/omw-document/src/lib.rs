//! 无损 Markdown 文档模型（V1 文档引擎的 parse 层）。
//!
//! 职责：只读解析 Markdown 源为带字节范围的 owned tree，并保证无编辑时的
//! 逐字节往返一致。不实现编辑命令、GPUI、文件保存或索引。
//!
//! 契约要点（详见 `docs/adr/adr-0008-visual-editing-save-policy.md`）：
//! - 源文本是事实来源；所有范围均为半开 `[start, end)` 字节范围。
//! - 每个源字节都有归属：节点范围、节点间隙（gaps）或 Document 尾随字节。
//! - 原始 HTML / 未知节点建 Raw 类型节点，保留原文，不改内部。
//! - 本 crate 不依赖 Comrak Arena 的借用；Arena 在 parse 返回前释放。

#![forbid(unsafe_code)]

pub mod edit;
pub mod history;
pub mod model;
pub mod parse;
pub mod rebase;
pub mod serialize;
pub mod source_map;
pub mod task;

pub use edit::{
    BlockKind, EditError, EditSession, MarkKind, SemanticOp, VisualPos, VisualRange,
    VisualSelection,
};
pub use history::{EditRecord, History, Step, TextDelta};
pub use model::{
    ByteOffset, ByteRange, DocumentSession, ListDelimiter, LosslessTree, NodeFlags, NodeId,
    NodeKind, RawDetail, SourceEncoding, SourceSlice, TreeNode,
};
pub use parse::{ParseError, parse};
pub use serialize::{
    PatchError, PatchPlan, PlanError, TextPatch, apply_patches, escape_text, fnv1a64,
    plan_source_patches, tree_semantic_projection, verify_patches,
};
pub use rebase::{TaskAnchor, TaskCommitError, TaskFieldUpdate, apply_task_update_to_baseline, locate_task_by_anchor};
pub use task::TaskDateField;
pub use source_map::SourceMap;
