//! 编辑历史：撤销/重做记录与栈。
//!
//! 撤销/重做不依赖行号或文本快照，而是每个编辑的**语义记录**：
//! - `EditRecord` 携带语义操作、选区 before/after 与步骤 ID；
//! - 正/逆向步骤均为**绝对写入**集合：文本增量 + 节点状态 + 父节点子列表；
//! - 节点引用 NodeId（arena 下标），结构移动保持 ID，删除节点为墓碑节点。
//!
//! 文本增量是**方向中立**的：`{ start, end, old_bytes, new_bytes }` 描述
//! "编辑前坐标 `[start,end)` 的内容被替换为 `new_bytes`"。正向应用
//! （重做）删除 `[start,end)` 并插入新的；逆向应用（撤销）删除
//! `[start, start+len(new_bytes))` 并插回旧的。每个步骤只含**一个**
//! 文本增量（该编辑的净文本效果，坐标均为编辑前坐标），因此两个方向的
//! 应用都是坐标安全的。
use crate::edit::{SemanticOp, VisualSelection};
use crate::model::{ByteOffset, ByteRange, NodeFlags, NodeId, NodeKind};

/// 文本增量：编辑前坐标上的半开字节区间 `[start, end)` 及其前后字节。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextDelta {
    pub start: ByteOffset,
    pub end: ByteOffset,
    /// 编辑前 `[start, end)` 的字节（撤销插回）。
    pub old_bytes: String,
    /// 编辑后的替换字节（重做再写入）。
    pub new_bytes: String,
}

/// 节点状态的绝对写入：应用时直接覆盖该节点字段。`id` 指向 arena
/// 中的稳定槽位（结构移动保持 ID；墓碑节点 `deleted=true`）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeState {
    pub id: NodeId,
    pub kind: NodeKind,
    pub source: Option<ByteRange>,
    pub flags: NodeFlags,
    pub children: Vec<NodeId>,
    pub deleted: bool,
}

/// 一步编辑的完整前后状态。正/逆向各为一组绝对写入，应用顺序：
/// 文本增量 → 节点状态 → 父节点子列表 → 重算间隙。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Step {
    pub text: Vec<TextDelta>,
    pub nodes: Vec<NodeState>,
    pub parents: Vec<(NodeId, Vec<NodeId>)>,
}

/// 撤销/重做记录：语义操作 + 选区 + 正向/逆向步骤。
#[derive(Clone, Debug)]
pub struct EditRecord {
    pub op: SemanticOp,
    pub selection_before: VisualSelection,
    pub selection_after: VisualSelection,
    pub forward: Step,
    pub inverse: Step,
}

/// 撤销/重做栈。`cursor` 指向"当前状态"在历史中的位置：
/// `entries[..cursor]` 为已应用（可撤销），`entries[cursor..]` 为已撤销
/// （可重做）。新记录提交时截断重做尾巴。
#[derive(Clone, Debug, Default)]
pub struct History {
    entries: Vec<EditRecord>,
    cursor: usize,
}

impl History {
    pub fn new() -> History {
        History {
            entries: Vec::new(),
            cursor: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 当前状态在历史中的位置（即可撤销深度）。
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn undo_depth(&self) -> usize {
        self.cursor
    }

    pub fn redo_depth(&self) -> usize {
        self.entries.len().saturating_sub(self.cursor)
    }

    pub fn peek_undo(&self) -> Option<&EditRecord> {
        self.cursor.checked_sub(1).map(|i| &self.entries[i])
    }

    pub fn peek_redo(&self) -> Option<&EditRecord> {
        self.entries.get(self.cursor)
    }

    /// 提交一条记录：截断 redo 尾巴。
    pub fn push(&mut self, record: EditRecord) {
        self.entries.truncate(self.cursor);
        self.entries.push(record);
        self.cursor = self.entries.len();
    }

    /// 弹出并返回最顶层的可撤销记录（cursor 前移一位）。
    pub(crate) fn pop_undo(&mut self) -> Option<EditRecord> {
        if self.cursor == 0 {
            return None;
        }
        self.cursor -= 1;
        Some(self.entries[self.cursor].clone())
    }

    /// 弹出并返回最顶层的可重做记录（cursor 后移一位）。
    pub(crate) fn pop_redo(&mut self) -> Option<EditRecord> {
        if self.cursor >= self.entries.len() {
            return None;
        }
        let rec = self.entries[self.cursor].clone();
        self.cursor += 1;
        Some(rec)
    }
}
