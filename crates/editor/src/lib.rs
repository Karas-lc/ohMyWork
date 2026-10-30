//! Lossless Markdown model and interactive editing surface.

#![forbid(unsafe_code)]

pub mod edit;
pub mod history;
pub mod model;
pub mod parse;
pub mod rebase;
pub mod serialize;
pub mod source_map;
pub mod task;

pub mod documents;
mod input;
mod toolbar;
mod view;
pub mod save_state;

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
pub use rebase::{
    TaskAnchor, TaskCommitError, TaskFieldUpdate, apply_task_update_to_baseline,
    locate_task_by_anchor,
};
pub use task::TaskDateField;
pub use source_map::SourceMap;
pub use input::{
    InputError, Utf16Range, VisibleProjection, break_visible_line, commit_text,
    delete_visible_range, utf16_range_to_visual,
};
pub use documents::SourceRope;
pub use documents::SourceSelection;
pub use documents::{DocumentEvent, DocumentStore, PageCommand, PageDocument, PageEditOutcome, PageError, PageOpenState, PageSnapshot, TaskIntent};
pub use toolbar::EditorToolbarCommand;
pub use view::EditorSurface;
