//! 语义编辑事务（V1 最小编辑集）。
//!
//! 工作模型：**一个 Rope 缓冲**持有文件等价字节（含标记），一棵可编辑树
//! 提供语义视图。核心不变式——**树与重解析语义一致**：把工作文本重新
//! 解析后得到的语义结构必须与工作树完全一致（`assert_semantics_consistent`）。
//!
//! 编辑操作（InsertText / DeleteText / ReplaceSelection / SplitBlock /
//! JoinBlocks / ToggleMark）只做规则内的事情，不接受任何会导致语义漂移的
//! 输入（Raw/代码/表格的内外、空白行间隙、空段落、跨标记选区等一律
//! 返回 typed `EditError`，**零状态变更**）。操作成功后提交一条
//! `EditRecord`（含方向无关的文本增量 + 节点/父节点绝对写入），
//! 供撤销/重做与保存规划使用。
//!
//! 普通输入在进入字节流前做一次转义（见 `serialize::escape_text`），
//! 保证键入/替换的内容只作字面文本，不产生结构性副作用。

use crate::history::{EditRecord, History, NodeState, Step, TextDelta};
use crate::model::{
    ByteOffset, ByteRange, DocumentSession, LosslessTree, NodeFlags, NodeId, NodeKind, RawDetail,
    SourceEncoding, TreeNode,
};
use crate::serialize::escape_text;
use ropey::Rope;
use std::borrow::Cow;
use std::fmt;
use unicode_segmentation::UnicodeSegmentation;

/// 视觉位置：工作文本中从 0 起计的字素（Grapheme Cluster）索引。
/// 显式区分于 UTF-8 字节偏移；转换 API 见 [`EditSession`]。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VisualPos(pub u32);

impl VisualPos {
    pub fn new(v: u32) -> VisualPos {
        VisualPos(v)
    }

    pub fn to_usize(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Debug for VisualPos {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "VisualPos({})", self.0)
    }
}

impl fmt::Display for VisualPos {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::ops::Add<u32> for VisualPos {
    type Output = VisualPos;
    fn add(self, rhs: u32) -> VisualPos {
        VisualPos(self.0.saturating_add(rhs))
    }
}

impl std::ops::Sub<u32> for VisualPos {
    type Output = VisualPos;
    fn sub(self, rhs: u32) -> VisualPos {
        VisualPos(self.0.saturating_sub(rhs))
    }
}

/// 视觉范围：两个视觉位置（调用方负责排序语义；本类型不做排序）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct VisualRange {
    pub start: VisualPos,
    pub end: VisualPos,
}

impl VisualRange {
    pub fn new(start: VisualPos, end: VisualPos) -> VisualRange {
        VisualRange { start, end }
    }

    pub fn sorted(self) -> VisualRange {
        if self.start <= self.end {
            self
        } else {
            VisualRange {
                start: self.end,
                end: self.start,
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
}

/// 视觉选区：锚点 + 焦点（选中范围 = 两者夹住的视觉区间）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct VisualSelection {
    pub anchor: VisualPos,
    pub head: VisualPos,
}

impl VisualSelection {
    /// 由锚点与焦点构造（范围取两者夹住的区间）。
    pub fn new(anchor: VisualPos, head: VisualPos) -> VisualSelection {
        VisualSelection { anchor, head }
    }

    pub fn collapsed(at: VisualPos) -> VisualSelection {
        VisualSelection {
            anchor: at,
            head: at,
        }
    }

    pub fn is_collapsed(&self) -> bool {
        self.anchor == self.head
    }

    pub fn range(&self) -> VisualRange {
        VisualRange::new(self.anchor.min(self.head), self.anchor.max(self.head))
    }
}
/// 可视化块类型。转换只改写目标块的源范围，块外的原始空白保持不变。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BlockKind {
    Paragraph,
    Heading { level: u8, setext: bool },
    Quote,
}

/// 标记类型。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MarkKind {
    Bold,
    Italic,
    Strikethrough,
}

/// 语义编辑操作（记录用；参数为用户意图的坐标/文本）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SemanticOp {
    InsertText {
        at: VisualPos,
        /// 转义前的原始输入。
        text: String,
    },
    DeleteText {
        range: VisualRange,
    },
    ReplaceSelection {
        range: VisualRange,
        /// 转义前的原始输入。
        text: String,
    },
    SplitBlock {
        at: VisualPos,
    },
    JoinBlocks {
        /// 被合并的块（与下一兄弟段落合并）。
        block: NodeId,
    },
    ToggleMark {
        mark: MarkKind,
        range: VisualRange,
    },
    SetBlockType {
        node: NodeId,
        kind: BlockKind,
    },
    ToggleTask {
        item: NodeId,
        checked: bool,
    },
    PasteMarkdown {
        range: VisualRange,
        text: String,
    },
    EditNode {
        node: NodeId,
        text: String,
    },
}

/// 编辑错误（typed）。所有失败都发生在**任何变更之前**：调用失败时
/// 会话状态与调用前完全一致。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditError {
    /// 位置越界 / 不是字素边界。
    PositionOutOfBounds,
    /// 目标不是语义文本叶节点（Raw/代码/表格结构/空白行间隙/空文档等）。
    NotInsideTextLeaf,
    /// 操作会触及不可编辑内容（Raw/代码等）或跨入其中。
    CrossesRawContent,
    /// 选区为空。
    EmptyRange,
    /// 输入文本含换行（\n/\r）：V1 只把输入当字面文本，段落级换行用 split_block。
    NewlineInInput,
    /// 操作后段落首行会以 ≥4 个连续空格开头：重解析时 comrak 会把它当作
    /// 缩进代码块（Raw 内容），与 V1 可视化编辑模型冲突，直接拒绝。
    IndentedCodeBlock,
    /// 操作会把整个块清空（该块将从重解析中消失）。
    EmptyBlockResult,
    /// 只支持对段落的拆分/合并。
    OnlyParagraphBlocks,
    /// 没有可合并的下一段落（跨父容器 / Raw 父容器 / 兄弟类型不符）。
    NoJoinTarget,
    /// 拆分会得到空的一半（位置在段首/段尾）。
    EmptyHalf,
    /// 拆分点/跨叶选区穿过行内标记或结构不支持。
    MarkShapeMismatch,
    /// 修改后文本大小超限（u32 字节偏移溢出）。
    Overflow,
    /// 内部不变量被破坏（不应对外暴露；出现即 bug）。
    InternalInvariant,
    /// 节点不存在、已删除或不是该操作需要的节点类型。
    InvalidNode,
    /// 输入不是合法 UTF-8 Markdown 或会生成不安全结构。
    InvalidMarkdown,
    /// 表格坐标或表格结构无效。
    InvalidTable,
    /// 空/合成表格单元不可作为编辑目标。
    SyntheticCell,
    /// Raw 节点只允许整体操作，不能编辑内部源文本。
    RawNode,
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EditError::PositionOutOfBounds => write!(f, "position out of bounds"),
            EditError::NotInsideTextLeaf => {
                write!(f, "target is not inside an editable text leaf")
            }
            EditError::CrossesRawContent => {
                write!(f, "edit crosses or touches non-editable (raw) content")
            }
            EditError::EmptyRange => write!(f, "empty range"),
            EditError::NewlineInInput => {
                write!(f, "input may not contain newlines (use split_block)")
            }
            EditError::IndentedCodeBlock => {
                write!(
                    f,
                    "leading whitespace would reparse as an indented code block"
                )
            }
            EditError::EmptyBlockResult => write!(f, "edit would empty the whole block"),
            EditError::OnlyParagraphBlocks => write!(f, "only paragraph blocks are supported"),
            EditError::NoJoinTarget => write!(f, "no joinable next paragraph"),
            EditError::EmptyHalf => write!(f, "split would produce an empty half"),
            EditError::MarkShapeMismatch => {
                write!(f, "selection does not match the target markup structure")
            }
            EditError::Overflow => write!(f, "text size limit exceeded"),
            EditError::InternalInvariant => write!(f, "internal invariant violated"),
            EditError::InvalidNode => write!(f, "invalid or deleted node"),
            EditError::InvalidMarkdown => write!(f, "markdown input is not safe to insert"),
            EditError::InvalidTable => write!(f, "invalid table coordinate or structure"),
            EditError::SyntheticCell => write!(f, "synthetic table cells are not editable"),
            EditError::RawNode => write!(f, "raw nodes can only be edited as a whole"),
        }
    }
}

impl std::error::Error for EditError {}

/// 禁止编辑的节点类型（Raw 家族 + 代码）：这些内容在编辑会话中只读。
fn kind_not_editable(kind: &NodeKind) -> bool {
    kind.is_raw() || matches!(kind, NodeKind::CodeBlock { .. } | NodeKind::Code { .. })
}

// ===== 可编辑树 =====

/// 树节点（arena 行）。`deleted` 标记墓碑节点：ID 保留、source/children
/// 清空，不再出现在任何遍历/锚定中。
#[derive(Clone, Debug)]
pub(crate) struct EditNode {
    pub(crate) kind: NodeKind,
    pub(crate) source: Option<ByteRange>,
    pub(crate) flags: NodeFlags,
    pub(crate) children: Vec<NodeId>,
    pub(crate) gaps: Vec<ByteRange>,
    pub(crate) deleted: bool,
}

/// owned 可编辑树：grow-only arena，NodeId 编辑期间稳定。
#[derive(Clone, Debug)]
pub(crate) struct EditableTree {
    nodes: Vec<EditNode>,
    root: NodeId,
}

impl EditableTree {
    pub(crate) fn from_tree(tree: &LosslessTree) -> EditableTree {
        let nodes = tree
            .nodes()
            .iter()
            .map(|n| EditNode {
                kind: n.kind.clone(),
                source: n.source,
                flags: n.flags,
                children: n.children.clone(),
                gaps: n.gaps.clone(),
                deleted: false,
            })
            .collect();
        EditableTree {
            nodes,
            root: tree.root(),
        }
    }

    pub(crate) fn root(&self) -> NodeId {
        self.root
    }

    pub(crate) fn len(&self) -> usize {
        self.nodes.len()
    }

    pub(crate) fn node(&self, id: NodeId) -> &EditNode {
        &self.nodes[id.0 as usize]
    }

    pub(crate) fn node_mut(&mut self, id: NodeId) -> &mut EditNode {
        &mut self.nodes[id.0 as usize]
    }

    /// 追加新节点（分配新 ID）。间隙由调用方后续设置或重算。
    pub(crate) fn push(
        &mut self,
        kind: NodeKind,
        source: Option<ByteRange>,
        flags: NodeFlags,
        children: Vec<NodeId>,
    ) -> NodeId {
        let id = NodeId(u32::try_from(self.nodes.len()).expect("arena overflow"));
        self.nodes.push(EditNode {
            kind,
            source,
            flags,
            children,
            gaps: Vec::new(),
            deleted: false,
        });
        id
    }

    /// 墓碑化：ID 保留，结构清空。
    pub(crate) fn tombstone(&mut self, id: NodeId) {
        let n = &mut self.nodes[id.0 as usize];
        n.source = None;
        n.children.clear();
        n.deleted = true;
    }

    /// 整棵子树递归墓碑化（孤儿预防）：墓碑化 id 及其全部后代。
    /// reconcile 重建段落行内时旧子树必须整体移除——若只墓碑化顶层，
    /// 旧子树的后代节点会以“存活但无父”的孤儿状态滞留 arena，携带陈旧
    /// 区间；recompute_gaps_all 遍历全部 id 时会在这些孤儿上误判区间倒置。
    pub(crate) fn bury_subtree(&mut self, id: NodeId) {
        let mut stack = vec![id];
        while let Some(cur) = stack.pop() {
            let kids = {
                let n = &mut self.nodes[cur.0 as usize];
                n.source = None;
                n.deleted = true;
                std::mem::take(&mut n.children)
            };
            stack.extend(kids);
        }
    }

    /// 从父节点子列表中摘除若干子节点（墓碑化后调用）。
    pub(crate) fn drop_children(&mut self, parent: NodeId, ids: &[NodeId]) {
        let n = &mut self.nodes[parent.0 as usize];
        n.children.retain(|c| !ids.contains(c));
    }

    /// 文档顺序的存活节点 DFS。
    pub(crate) fn alive_dfs(&self) -> Vec<NodeId> {
        let mut out = Vec::new();
        fn walk(tree: &EditableTree, id: NodeId, out: &mut Vec<NodeId>) {
            out.push(id);
            for &c in &tree.node(id).children {
                walk(tree, c, out);
            }
        }
        if !self.nodes.is_empty() {
            walk(self, self.root, &mut out);
        }
        out
    }

    /// arena 顺序的全部 ID（父先于子）。
    pub(crate) fn all_ids(&self) -> Vec<NodeId> {
        (0..self.nodes.len()).map(|i| NodeId(i as u32)).collect()
    }

    /// 父节点索引（arena 大小；根与墓碑无父）。
    pub(crate) fn parent_index(&self) -> Vec<Option<NodeId>> {
        let mut idx: Vec<Option<NodeId>> = vec![None; self.nodes.len()];
        for (i, n) in self.nodes.iter().enumerate() {
            if n.deleted {
                continue;
            }
            for &c in &n.children {
                idx[c.0 as usize] = Some(NodeId(i as u32));
            }
        }
        idx
    }

    /// 定位包含 `byte` 的叶节点（叶 = 无子节点）。
    /// 返回 `(叶, 直接父, 父子列表下标)`；字节落在间隙或无子可入时为 `None`。
    /// 规则：`byte == child.end` 优先落入**前一个**叶（"扩展上一叶"语义）。
    pub(crate) fn leaf_at(&self, byte: ByteOffset) -> Option<(NodeId, NodeId, usize)> {
        let mut cur = self.root;
        loop {
            let n = self.node(cur);
            if n.deleted {
                return None;
            }
            let range = n.source?;
            if !(range.start <= byte && byte <= range.end) {
                return None;
            }
            let mut hit: Option<(NodeId, usize)> = None;
            for (i, &c) in n.children.iter().enumerate() {
                let cc = self.node(c);
                if cc.deleted {
                    continue;
                }
                let Some(cr) = cc.source else { continue };
                if cr.start <= byte && byte <= cr.end {
                    hit = Some((c, i));
                    break;
                }
            }
            match hit {
                Some((c, i)) => {
                    if self.node(c).children.is_empty() {
                        return Some((c, cur, i));
                    }
                    cur = c;
                }
                None => return None,
            }
        }
    }

    /// 对 `[threshold, +∞)` 的节点与间隙范围做偏移；`threshold` 为插入/
    /// 删除点（净文本效果的坐标）。对跨过 threshold 的容器只延展其 end。
    /// 调用方必须保证替换区间内部的节点均已墓碑化或另行重锚。
    pub(crate) fn shift_from(
        &mut self,
        threshold: ByteOffset,
        delta: i64,
    ) -> Result<(), EditError> {
        let t = threshold.0 as i64;
        for i in 0..self.nodes.len() {
            let (mut src, mut gaps) = match self.nodes[i].deleted {
                true => continue,
                false => (self.nodes[i].source, self.nodes[i].gaps.clone()),
            };
            if let Some(r) = &mut src {
                self.shift_range(r, t, delta)?;
                if let Some(nr) = src {
                    self.nodes[i].source = Some(nr);
                }
            }
            for g in gaps.iter_mut() {
                self.shift_range(g, t, delta)?;
            }
            self.nodes[i].gaps = gaps;
        }
        Ok(())
    }

    fn shift_range(&self, r: &mut ByteRange, t: i64, delta: i64) -> Result<(), EditError> {
        let (a, b) = (r.start.0 as i64, r.end.0 as i64);
        if b <= t {
            return Ok(()); // 完全在 threshold 之前：不动
        }
        if a >= t {
            // 统一平移：所有从插入点开始的节点都右移/左移。
            // 叶/容器的“吸收”由调用方（insert_text 等）重设 range 负责，
            // 不得在这里吸收——否则恰从 t 开始的相邻兄弟（如 Strong）
            // 会被错误地留在原地，产生重叠区间。
            if a + delta < 0 || b + delta < 0 {
                return Err(EditError::Overflow);
            }
            r.start = ByteOffset((a + delta) as u32);
            r.end = ByteOffset((b + delta) as u32);
            return Ok(());
        }
        // a < t <= b：跨过 threshold，延展 end（容器包裹插入/删除）。
        if b + delta < a {
            return Err(EditError::Overflow);
        }
        r.end = ByteOffset((b + delta) as u32);
        Ok(())
    }

    /// 全部容器重算间隙（每次操作后调用，保证拼接不变量）。
    pub(crate) fn recompute_gaps_all(&mut self) {
        for i in 0..self.nodes.len() {
            self.recompute_gaps_for(NodeId(i as u32));
        }
    }

    fn recompute_gaps_for(&mut self, id: NodeId) {
        let n = &self.nodes[id.0 as usize];
        if n.deleted {
            return;
        }
        let Some(pr) = n.source else {
            self.nodes[id.0 as usize].gaps.clear();
            return;
        };
        let children = n.children.clone();
        let all_attributed = children
            .iter()
            .all(|&c| self.nodes[c.0 as usize].source.is_some());
        if !all_attributed {
            self.nodes[id.0 as usize].gaps.clear();
            return;
        }
        let mut gaps = Vec::with_capacity(children.len() + 1);
        let mut cursor = pr.start;
        for c in &children {
            let cr = self.nodes[c.0 as usize].source.unwrap();
            gaps.push(ByteRange::new(cursor, cr.start));
            cursor = cr.end;
        }
        gaps.push(ByteRange::new(cursor, pr.end));
        self.nodes[id.0 as usize].gaps = gaps;
    }

    /// 合并相邻的 Text 叶（续接文本必须保持单一 Text 节点，否则重解析
    /// 语义与工作树不一致）。合并方向：后叶并入前叶，后叶墓碑化。
    pub(crate) fn coalesce_all(&mut self) {
        for i in 0..self.nodes.len() {
            let id = NodeId(i as u32);
            loop {
                let sibs = self.node(id).children.clone();
                if self.node(id).deleted || sibs.len() < 2 {
                    break;
                }
                let mut merged = false;
                for w in 0..sibs.len() - 1 {
                    let (a, b) = (sibs[w], sibs[w + 1]);
                    let (na, nb) = (self.node(a), self.node(b));
                    if na.deleted
                        || nb.deleted
                        || na.kind != NodeKind::Text
                        || nb.kind != NodeKind::Text
                    {
                        continue;
                    }
                    let (ra, rb) = match (na.source, nb.source) {
                        (Some(x), Some(y)) => (x, y),
                        _ => continue,
                    };
                    if ra.end != rb.start {
                        continue;
                    }
                    let ar = ByteRange::new(ra.start, rb.end);
                    let node_a = self.node_mut(a);
                    node_a.source = Some(ar);
                    node_a.children.clear();
                    self.node_mut(id).children.remove(w + 1);
                    let nb_m = self.node_mut(b);
                    nb_m.source = None;
                    nb_m.children.clear();
                    nb_m.deleted = true;
                    merged = true;
                    break;
                }
                if !merged {
                    break;
                }
            }
        }
    }

    /// 校验树不变量（镜像 `LosslessTree::validate`；跳过墓碑）。
    pub(crate) fn validate(&self, text_len: usize) -> Result<(), Vec<String>> {
        let mut issues = Vec::new();
        for (idx, n) in self.nodes.iter().enumerate() {
            let id = NodeId(idx as u32);
            if n.deleted {
                continue;
            }
            let Some(range) = n.source else { continue };
            if range.end.to_usize() > text_len {
                issues.push(format!(
                    "node {id:?} range exceeds text: {}",
                    range.to_debug()
                ));
            }
            if range.start > range.end {
                issues.push(format!(
                    "node {id:?} has inverted range: {}",
                    range.to_debug()
                ));
            }
            let mut cursor = range.start;
            for &child in &n.children {
                let Some(cr) = self.nodes[child.0 as usize].source else {
                    issues.push(format!("node {id:?} has unattributed child {child:?}"));
                    continue;
                };
                if !range.contains_range(&cr) {
                    issues.push(format!(
                        "node {id:?} child {child:?} range {} exceeds parent {}",
                        cr.to_debug(),
                        range.to_debug()
                    ));
                }
                if cr.start < cursor {
                    issues.push(format!(
                        "node {id:?} children overlap/unsorted at {child:?}"
                    ));
                }
                cursor = cursor.max(cr.end);
            }
            let all_attributed = n
                .children
                .iter()
                .all(|c| self.nodes[c.0 as usize].source.is_some());
            if all_attributed && n.gaps.len() != n.children.len() + 1 {
                issues.push(format!(
                    "node {id:?} gaps.len() {} != children.len()+1 {}",
                    n.gaps.len(),
                    n.children.len() + 1
                ));
            }
        }
        if issues.is_empty() {
            Ok(())
        } else {
            Err(issues)
        }
    }
}

// ===== 编辑会话 =====

/// 记录一次操作的受影响节点（before/after 状态对）。
#[derive(Default)]
struct ChangeLog {
    text: Vec<TextDelta>,
    // 操作前后整个竞技场（全部节点，含墓碑）的状态快照：
    // shift_from/postop 会改动很多节点的范围（段落、后续兄弟、
    // 墓碑清空等），只记录局部节点在 undo 后会留下陈旧区间。
    before_nodes: Vec<NodeState>,
    after_nodes: Vec<NodeState>,
}

/// 编辑会话：工作文本 + 可编辑树 + 历史。
#[derive(Clone)]
pub struct EditSession {
    text: Rope,
    tree: EditableTree,
    encoding: SourceEncoding,
    history: History,
    /// fork 时基树的节点数：`id >= base_len` 即为本次会话新建的节点。
    base_len: usize,
}

impl EditSession {
    /// 底层可编辑树（序列化定位与诊断用，crate 内部）。
    pub(crate) fn tree(&self) -> &EditableTree {
        &self.tree
    }

    /// 从只读文档会话 fork 出编辑会话。节点 ID 与基树一一对应。
    pub fn from_document(doc: &DocumentSession) -> EditSession {
        EditSession {
            text: doc.text().clone(),
            tree: EditableTree::from_tree(doc.tree()),
            encoding: doc.encoding(),
            history: History::new(),
            base_len: doc.tree().len(),
        }
    }

    // ---- 只读视图 ----

    pub fn text(&self) -> &Rope {
        &self.text
    }

    pub fn len_bytes(&self) -> usize {
        self.text.len_bytes()
    }

    pub fn bytes(&self) -> Vec<u8> {
        self.text
            .chunks()
            .flat_map(|c| c.as_bytes().iter().copied())
            .collect()
    }

    pub fn encoding(&self) -> SourceEncoding {
        self.encoding
    }

    pub fn history(&self) -> &History {
        &self.history
    }

    /// Prepare a committed task-field overlay across every undo/redo state.
    /// The original session is untouched until the caller receives the disk ack.
    pub fn with_committed_task_field(
        &self,
        item: NodeId,
        update: &crate::TaskFieldUpdate,
    ) -> Result<Self, String> {
        fn overlay(
            session: &EditSession,
            item: NodeId,
            update: &crate::TaskFieldUpdate,
        ) -> Result<Vec<u8>, String> {
            let bytes = session.bytes();
            let Some(range) = session.source_range(item) else {
                return Ok(bytes);
            };
            let parsed = crate::parse(&bytes).map_err(|e| e.to_string())?;
            let task = crate::task::task_items(parsed.tree())
                .into_iter()
                .find(|task| {
                    parsed
                        .tree()
                        .node(task.node)
                        .source
                        .is_some_and(|r| r.start == range.start)
                })
                .ok_or("committed task no longer maps to its source")?;
            crate::apply_task_update_to_baseline(
                &parsed,
                &crate::TaskAnchor {
                    item_range: range,
                    checkbox_range: task.checkbox_range,
                },
                update,
            )
            .map_err(|e| e.to_string())
        }
        if !matches!(self.node_kind(item), Some(NodeKind::TaskItem { .. })) {
            return Err("task is deleted or exists only in unsaved content".into());
        }
        let cursor = self.history.cursor();
        let mut replay = self.clone();
        while replay.undo().map_err(|e| e.to_string())?.is_some() {}
        let initial = overlay(&replay, item, update)?;
        let parsed = crate::parse(&initial).map_err(|e| e.to_string())?;
        let mut rebased = Self::from_document(&parsed);
        while let Some(record) = replay.redo().map_err(|e| e.to_string())? {
            let next =
                String::from_utf8(overlay(&replay, item, update)?).map_err(|e| e.to_string())?;
            rebased
                .rebuild_from_text(
                    next,
                    record.op,
                    record.selection_before,
                    record.selection_after,
                )
                .map_err(|e| e.to_string())?;
        }
        while rebased.history.cursor() > cursor {
            rebased.undo().map_err(|e| e.to_string())?;
        }
        Ok(rebased)
    }

    /// 基树节点数（锚定与规划用）。
    pub fn base_len(&self) -> usize {
        self.base_len
    }

    /// 任意范围的字节切片（字符边界校验，越界返回 `None`）。
    pub fn byte_slice(&self, range: ByteRange) -> Option<Cow<'_, str>> {
        let r = range.to_byte_range();
        if r.end > self.text.len_bytes() || r.start > r.end {
            return None;
        }
        // ropey 的 try_byte_to_char 对非边界字节不报错（向下取整），
        // 必须“byte→char→byte”往返验证才能真正判断字符边界。
        let ok_boundary = |byte: usize| {
            self.text
                .try_byte_to_char(byte)
                .ok()
                .and_then(|c| self.text.try_char_to_byte(c).ok())
                == Some(byte)
        };
        if !ok_boundary(r.start) || !ok_boundary(r.end) {
            return None;
        }
        Some(Cow::from(self.text.byte_slice(r)))
    }

    /// 存活 Text 叶（文档顺序）：`(叶 id, 字节范围)`。编辑测试/调用方
    /// 定位选区用。
    pub fn text_leaves(&self) -> Vec<(NodeId, ByteRange)> {
        let mut out = Vec::new();
        for id in self.tree.alive_dfs() {
            let n = self.tree.node(id);
            if n.deleted || !n.children.is_empty() {
                continue;
            }
            if n.kind == NodeKind::Text
                && let Some(r) = n.source
            {
                out.push((id, r));
            }
        }
        out
    }

    /// 语义投影（测试/规划用）：文档顺序 `(NodeKind, 原文)`。
    pub fn semantic_projection(&self) -> Vec<(NodeKind, String)> {
        fn walk(tree: &EditableTree, rope: &Rope, id: NodeId, out: &mut Vec<(NodeKind, String)>) {
            let n = tree.node(id);
            // 与 serialize::tree_semantic_projection 对称：Text 节点在投影前
            // unescape 掉序列化转义，保证与重解析语义一致。
            let raw = n
                .source
                .map(|r| rope.byte_slice(r.to_byte_range()).to_string())
                .unwrap_or_default();
            let bytes = if n.kind == NodeKind::Text {
                crate::serialize::unescape_text(&raw)
            } else {
                raw
            };
            out.push((n.kind.clone(), bytes));
            for &c in &n.children {
                walk(tree, rope, c, out);
            }
        }
        let mut out = Vec::new();
        if self.tree.len() > 0 {
            walk(&self.tree, &self.text, self.tree.root(), &mut out);
        }
        out
    }

    /// 存活节点压缩视图（供渲染/诊断；ID 重新编号，与基树不再对应）。
    pub fn semantic_tree(&self) -> LosslessTree {
        let ids = self.tree.all_ids();
        let mut remap: Vec<Option<NodeId>> = vec![None; self.tree.len()];
        let mut nodes: Vec<TreeNode> = Vec::new();
        let mut new_root = None;
        for &id in &ids {
            let n = self.tree.node(id);
            if n.deleted {
                continue;
            }
            let new_id = NodeId(u32::try_from(nodes.len()).expect("arena overflow"));
            remap[id.0 as usize] = Some(new_id);
            if id == self.tree.root() {
                new_root = Some(new_id);
            }
            let children = n
                .children
                .iter()
                .filter_map(|&c| remap[c.0 as usize])
                .collect();
            nodes.push(TreeNode {
                kind: n.kind.clone(),
                source: n.source,
                flags: n.flags,
                children,
                gaps: n.gaps.clone(),
            });
        }
        let root = new_root.unwrap_or(NodeId(0));
        LosslessTree::new(nodes, root)
    }

    /// 校验整棵可编辑树不变量。
    pub fn validate(&self) -> Result<(), Vec<String>> {
        self.tree.validate(self.text.len_bytes())
    }

    // ---- 字素（视觉位置）↔ 字节转换 ----

    /// 全部字素簇的起始字节偏移（含结尾哨兵 = 文本长度）。
    fn grapheme_starts(&self) -> Vec<usize> {
        let s = self.text.to_string();
        let mut v = Vec::new();
        for (i, _) in s.grapheme_indices(true) {
            v.push(i);
        }
        v.push(s.len());
        v
    }

    /// 视觉长度（字素数）。
    pub fn visual_len(&self) -> u32 {
        u32::try_from(self.grapheme_starts().len().saturating_sub(1)).unwrap_or(u32::MAX)
    }

    /// 视觉位置 → 字节偏移。`None` 表示越界（或乱序位置）。
    pub fn visual_to_byte(&self, pos: VisualPos) -> Option<ByteOffset> {
        let starts = self.grapheme_starts();
        starts
            .get(pos.to_usize())
            .copied()
            .and_then(ByteOffset::from_usize)
    }

    /// 字节偏移 → 视觉位置。仅当字节在字素簇起点时返回 `Some`。
    pub fn byte_to_visual(&self, byte: ByteOffset) -> Option<VisualPos> {
        let b = byte.to_usize();
        if self.text.try_byte_to_char(b).is_err() {
            return None;
        }
        let starts = self.grapheme_starts();
        match starts.binary_search(&b) {
            Ok(i) => Some(VisualPos(u32::try_from(i).unwrap_or(u32::MAX))),
            Err(_) => None,
        }
    }

    fn char_of(&self, byte: ByteOffset) -> Result<usize, EditError> {
        self.text
            .try_byte_to_char(byte.to_usize())
            .map_err(|_| EditError::PositionOutOfBounds)
    }

    // ---- 编辑执行 ----

    /// 输入文本（转义后写作真文本）。
    pub fn insert_text(&mut self, at: VisualPos, text: &str) -> Result<EditRecord, EditError> {
        if text.contains('\n') || text.contains('\r') {
            return Err(EditError::NewlineInInput);
        }
        let byte = self
            .visual_to_byte(at)
            .ok_or(EditError::PositionOutOfBounds)?;
        let (leaf, parent, _idx) = self.editable_text_leaf(byte)?;
        let leaf_range = self.tree.node(leaf).source.unwrap();
        let escaped = escape_text(text);
        // 行首插入空格/制表：若累计缩进达到 4 → 重解析成缩进代码块（Raw）。
        if let Some((blk, pr)) = self.block_range_of(leaf)
            && byte == pr.start
            && !self.inside_list_or_quote(blk)
            && Self::leading_ws_width(&escaped) > 0
        {
            let outer = self.line_leading_spaces_before(pr.start);
            let inner = Self::leading_ws_width(&escaped);
            if outer + inner >= 4 {
                return Err(EditError::IndentedCodeBlock);
            }
        }
        let old = String::new();
        // 叶吸收插入字节（即使 byte == leaf 末尾也不整体平移：字节归该叶）。
        // 段首/段尾插入时段落块也要相应重锚定，否则 recompute_gaps 会看到
        // 叶与段落区间错位（段首均匀平移把段落甩开；段尾不扩展段落）。
        let block = self.block_range_of(leaf);

        let mut log = self.begin_log(&[leaf], &[parent]);
        // 文本：净增量 = 纯插入。
        self.apply_rope_insert(byte, &escaped)?;
        // 树：threshold 之后统一偏移；叶吸收插入字节。
        self.tree.shift_from(byte, escaped.len() as i64)?;
        self.set_range(
            leaf,
            ByteRange::new(
                leaf_range.start,
                byte_checked_add(leaf_range.end, escaped.len() as u32)?,
            ),
        )?;
        if let Some((blk, pr)) = block {
            if byte == pr.start {
                // 段首插入：uniform shift 会把段落右移，需重锚回原起点并扩展 end。
                let old_end = pr.end;
                self.set_range(
                    blk,
                    ByteRange::new(pr.start, byte_checked_add(pr.end, escaped.len() as u32)?),
                )?;
                self.extend_containers_ending_at(blk, old_end, escaped.len() as i64)?;
                // 起点与段落重合的祖先容器（如首段时的 Document）同样被统移，
                // 需要拉回起点（end 已增长）。
                self.reanchor_containers_starting_at(blk, pr.start, escaped.len() as u32)?;
            } else if byte == pr.end {
                // 段尾插入：新字节即段落内容，段落（及 end 相重的祖先）必须
                // 随叶一并增长。末尾叶 end == pr.end 时叶吸收已扩，段落同样要扩。
                let old_end = pr.end;
                self.set_range(
                    blk,
                    ByteRange::new(pr.start, byte_checked_add(pr.end, escaped.len() as u32)?),
                )?;
                self.extend_containers_ending_at(blk, old_end, escaped.len() as i64)?;
            }
        }
        log.text.push(TextDelta {
            start: byte,
            end: byte,
            old_bytes: old,
            new_bytes: escaped.clone(),
        });
        self.postop();

        let after = at + grapheme_count(&escaped);
        Ok(self.record(
            log,
            SemanticOp::InsertText {
                at,
                text: text.to_string(),
            },
            VisualSelection::collapsed(at),
            VisualSelection::collapsed(after),
        ))
    }

    pub fn delete_text(&mut self, range: VisualRange) -> Result<EditRecord, EditError> {
        let r = self.range_to_bytes(&range)?;
        if r.0 == r.1 {
            // 空选区：无操作记录（幂等）。undo/redo 必须把整片竞技场原样
            // 保存——若用空 Step（nodes=[]），apply_step 会把所有未被覆盖的
            // 节点当成“本次新建”全部框为墓碑，整棵树被清空。
            let snap = self.snapshot_all();
            let step = Step {
                text: Vec::new(),
                nodes: snap,
                parents: Vec::new(),
            };
            let sel = VisualSelection::collapsed(range.start.min(range.end));
            let rec = EditRecord {
                op: SemanticOp::DeleteText { range },
                selection_before: sel,
                selection_after: sel,
                forward: step.clone(),
                inverse: step,
            };
            self.history.push(rec.clone());
            return Ok(rec);
        }
        self.replace_span(r.0, r.1, "", true, "")
    }

    /// 跨块删除事务。它只允许删除可视化内容和结构间隙，遇到 Raw/HTML
    /// 节点会整体拒绝，避免选区把不可编辑源字节吞掉。
    pub fn delete_range(&mut self, range: VisualRange) -> Result<EditRecord, EditError> {
        let (start, end) = self.range_to_bytes(&range)?;
        if start == end {
            return Err(EditError::EmptyRange);
        }
        for id in self.tree.alive_dfs() {
            let node = self.tree.node(id);
            if node.kind.is_raw()
                && node
                    .source
                    .is_some_and(|source| source.start < end && start < source.end)
            {
                return Err(EditError::CrossesRawContent);
            }
        }
        let mut next = String::from_utf8(self.bytes()).map_err(|_| EditError::InvalidMarkdown)?;
        next.replace_range(start.0 as usize..end.0 as usize, "");
        self.rebuild_from_text(
            next,
            SemanticOp::DeleteText { range },
            VisualSelection::new(range.start, range.end),
            VisualSelection::collapsed(range.start),
        )
    }

    pub fn replace_selection(
        &mut self,
        range: VisualRange,
        text: &str,
    ) -> Result<EditRecord, EditError> {
        if text.contains('\n') || text.contains('\r') {
            return Err(EditError::NewlineInInput);
        }
        let r = self.range_to_bytes(&range)?;
        let escaped = escape_text(text);
        // 空文本 = 删除语义（含“清空整个块”防护）。
        self.replace_span(r.0, r.1, &escaped, escaped.is_empty(), text)
    }

    /// 段落拆分：在 `at` 处拆成两个段落（插入 `\n\n`）。
    pub fn split_block(&mut self, at: VisualPos) -> Result<EditRecord, EditError> {
        let byte = self
            .visual_to_byte(at)
            .ok_or(EditError::PositionOutOfBounds)?;
        let (leaf, parent, idx) = self.editable_text_leaf(byte)?;
        // 只支持段落，且叶必须直属段落（不穿过行内标记）。
        let (paragraph, prange) = self
            .block_range_of(leaf)
            .ok_or(EditError::OnlyParagraphBlocks)?;
        if self.tree.node(paragraph).kind != NodeKind::Paragraph {
            return Err(EditError::OnlyParagraphBlocks);
        }
        if parent != paragraph {
            return Err(EditError::MarkShapeMismatch);
        }
        if byte <= prange.start || byte >= prange.end {
            return Err(EditError::EmptyHalf);
        }
        let leaf_range = self.tree.node(leaf).source.unwrap();
        if byte <= leaf_range.start || byte >= leaf_range.end {
            return Err(EditError::EmptyHalf);
        }

        let para_children = self.tree.node(paragraph).children.clone();
        let parent_of_para = self.parent_of(paragraph);
        let para_idx = match parent_of_para {
            Some(p) => p,
            None => {
                return Err(EditError::NoJoinTarget); // 防御：段落必有父
            }
        };
        let siblings = self.tree.node(para_idx).children.clone();
        let pos_in_siblings = siblings
            .iter()
            .position(|&c| c == paragraph)
            .ok_or(EditError::InternalInvariant)?;

        let mut log = self.begin_log(&[leaf, paragraph], &[para_idx]);
        // 文本：净增量 = 插入 "\n\n"。
        self.apply_rope_insert(byte, "\n\n")?;
        self.tree.shift_from(byte, 2)?;
        // 树：第一半保持原 ID，第二半新建。
        self.set_range(leaf, ByteRange::new(leaf_range.start, byte))?;
        self.set_range(paragraph, ByteRange::new(prange.start, byte))?;
        let leaf2 = self.tree.push(
            NodeKind::Text,
            Some(ByteRange::new(
                byte_checked_add(byte, 2)?,
                byte_checked_add(leaf_range.end, 2)?,
            )),
            NodeFlags::NONE,
            Vec::new(),
        );
        self.log_created(&mut log, leaf2);
        let tail = &para_children[(idx + 1)..];
        let mut para2_children = Vec::with_capacity(tail.len() + 1);
        para2_children.push(leaf2);
        para2_children.extend_from_slice(tail);
        let para2 = self.tree.push(
            NodeKind::Paragraph,
            Some(ByteRange::new(
                byte_checked_add(byte, 2)?,
                byte_checked_add(prange.end, 2)?,
            )),
            NodeFlags::NONE,
            para2_children,
        );
        self.log_created(&mut log, para2);
        // 归属：原段保留 [..=idx]，新段持有 leaf2 + 尾部兄弟。
        self.tree.node_mut(paragraph).children.truncate(idx + 1);
        // 父容器插入 para2 紧跟 para。
        {
            let sibs = &mut self.tree.node_mut(para_idx).children;
            sibs.insert(pos_in_siblings + 1, para2);
        }
        log.text.push(TextDelta {
            start: byte,
            end: byte,
            old_bytes: String::new(),
            new_bytes: "\n\n".to_string(),
        });
        self.postop();

        let after = at + 2;
        Ok(self.record(
            log,
            SemanticOp::SplitBlock { at },
            VisualSelection::collapsed(at),
            VisualSelection::collapsed(after),
        ))
    }

    /// 合并：位置所在段落与其**下一兄弟段落**合并（插入 `\n` 软换行）。
    pub fn join_blocks(&mut self, at: VisualPos) -> Result<EditRecord, EditError> {
        let byte = self
            .visual_to_byte(at)
            .ok_or(EditError::PositionOutOfBounds)?;
        let (leaf, _, _) = self.editable_text_leaf(byte)?;
        let (para, prange) = self
            .block_range_of(leaf)
            .ok_or(EditError::OnlyParagraphBlocks)?;
        if self.tree.node(para).kind != NodeKind::Paragraph {
            return Err(EditError::OnlyParagraphBlocks);
        }
        // 下一兄弟必须是段落。
        let pidx = self.parent_of(para).ok_or(EditError::NoJoinTarget)?;
        let parent_kind = self.tree.node(pidx).kind.clone();
        if kind_not_editable(&parent_kind) {
            return Err(EditError::NoJoinTarget);
        }
        let sibs = self.tree.node(pidx).children.clone();
        let i = sibs
            .iter()
            .position(|&c| c == para)
            .ok_or(EditError::InternalInvariant)?;
        let next = *sibs.get(i + 1).ok_or(EditError::NoJoinTarget)?;
        if self.tree.node(next).kind != NodeKind::Paragraph {
            return Err(EditError::NoJoinTarget);
        }
        let qrange = self
            .tree
            .node(next)
            .source
            .ok_or(EditError::InternalInvariant)?;
        // 段落不包含结尾 \n（它在父节点间隙里），所以要替换的"分隔符"
        // 是 [prange.end, qrange.start)，不是 [prange.end, qrange.end)。
        let old: String = self
            .byte_slice(ByteRange::new(prange.end, qrange.start))
            .ok_or(EditError::InternalInvariant)?
            .into_owned();

        let mut log = self.begin_log(&[para, next], &[pidx]);
        // 文本：净增量 = 用 "\n" 替换分隔符 [prange.end, qrange.start)。
        self.apply_rope_replace(prange.end, qrange.start, "\n")?;
        // 树：
        //  1) threshold = qrange.start：Q 自身及其后一切偏移 delta。
        let delta = 1_i64 - i64::from(qrange.start.0 - prange.end.0);
        self.tree.shift_from(qrange.start, delta)?;
        //  2) Q 的子树保持原位（仅随 shift 平移），之后整体移入 P。
        let moved = self.tree.node(next).children.clone();
        //  3) para 延展到合并后的末尾，换行节点接手 "\n"。
        self.tree
            .node_mut(para)
            .children
            .extend(moved.iter().copied());
        //     若 P 尾部有 ≥2 空格，CommonMark 视作硬换行：
        //     LineBreak 源覆盖“最后 2 空格 + \n”（多余空格并入 gap）。
        let p_slice: String = self
            .byte_slice(prange)
            .map(|c| c.into_owned())
            .unwrap_or_default();
        let trail = p_slice.bytes().rev().take_while(|&c| c == b' ').count();
        let (break_kind, break_start) = if trail >= 2 {
            (NodeKind::LineBreak, prange.end.0.saturating_sub(2))
        } else {
            (NodeKind::SoftBreak, prange.end.0)
        };
        let sb = self.tree.push(
            break_kind,
            Some(ByteRange::new(
                ByteOffset(break_start),
                byte_checked_add(prange.end, 1)?,
            )),
            NodeFlags::NONE,
            Vec::new(),
        );
        self.log_created(&mut log, sb);
        // 顺序：换行节点位于被并入子节点之前。
        {
            let p = self.tree.node_mut(para);
            let insert_at = p.children.len() - moved.len();
            p.children.insert(insert_at, sb);
        }
        // 合并后段落 = [ps, pe) + "\n" + Q 内容 = [ps, pe + 1 + (qe - qs))。
        let para_end = ByteOffset(prange.end.0 + 1 + (qrange.end.0 - qrange.start.0));
        self.set_range(para, ByteRange::new(prange.start, para_end))?;
        self.tree.tombstone(next);
        self.tree.drop_children(pidx, &[next]);
        log.text.push(TextDelta {
            start: prange.end,
            end: qrange.start,
            old_bytes: old,
            new_bytes: "\n".to_string(),
        });
        self.postop();

        let after = self
            .byte_to_visual(prange.end)
            .ok_or(EditError::InternalInvariant)?;
        Ok(self.record(
            log,
            SemanticOp::JoinBlocks { block: para },
            VisualSelection::collapsed(at),
            VisualSelection::collapsed(after),
        ))
    }

    /// 选中范围加上/去掉加粗/斜体标记。
    pub fn toggle_mark(
        &mut self,
        mark: MarkKind,
        range: VisualRange,
    ) -> Result<EditRecord, EditError> {
        let r = self.range_to_bytes(&range)?;
        if r.0 == r.1 {
            return Err(EditError::EmptyRange);
        }
        let (b, e) = (r.0, r.1);
        let marker = match mark {
            MarkKind::Bold => "**",
            MarkKind::Italic => "*",
            MarkKind::Strikethrough => "~~",
        };
        let expected_kind = match mark {
            MarkKind::Bold => NodeKind::Strong,
            MarkKind::Italic => NodeKind::Emph,
            MarkKind::Strikethrough => NodeKind::Strikethrough,
        };
        let (leaf, leaf_parent, idx) = self.editable_text_leaf(b)?;
        let lr = self.tree.node(leaf).source.unwrap();
        if e > lr.end {
            // 选区必须落在单个 Text 叶内（跨叶/跨标记拒绝）。
            return Err(EditError::MarkShapeMismatch);
        }
        let parent_kind = self.tree.node(leaf_parent).kind.clone();

        // ---- 去除标记：selection 恰好是标记节点的唯一子内容 ----
        if parent_kind == expected_kind && self.tree.node(leaf_parent).children.len() == 1 {
            // 必须整个叶、且选择范围 = 整个内容。
            if b != lr.start || e != lr.end {
                return Err(EditError::MarkShapeMismatch);
            }
            let mark_id = leaf_parent;
            let mark_parent = self
                .parent_of(mark_id)
                .ok_or(EditError::MarkShapeMismatch)?;
            let mg = self.tree.node(mark_id).gaps.clone();
            if mg.len() < 2 {
                return Err(EditError::MarkShapeMismatch);
            }
            let open = mg[0];
            let close = mg[mg.len() - 1];
            let (open_ok, close_ok) = (self.bytes_eq(open, marker), self.bytes_eq(close, marker));
            if !open_ok || !close_ok {
                return Err(EditError::MarkShapeMismatch);
            }
            let content: String = self
                .byte_slice(ByteRange::new(lr.start, lr.end))
                .ok_or(EditError::InternalInvariant)?
                .into_owned();

            let mut log = self.begin_log(&[leaf, mark_id], &[mark_parent]);
            // 文本：净增量 = 用内容替换 [open.start, close.end)。
            let old: String = self
                .byte_slice(ByteRange::new(open.start, close.end))
                .ok_or(EditError::InternalInvariant)?
                .into_owned();
            self.apply_rope_replace(open.start, close.end, &content)?;
            // 树：先移除前标记（内容随之前移），再移除后标记。
            let open_len = i64::from(open.len());
            let close_len = i64::from(close.len());
            self.tree.shift_from(open.end, -open_len)?;
            let close_start = ByteOffset((close.start.0 as i64 - open_len) as u32);
            self.tree.shift_from(close_start, -close_len)?;
            self.tree.tombstone(mark_id);
            // 叶挂回原父（mark 的父）的同一位置。
            {
                let mparent = &mut self.tree.node_mut(mark_parent).children;
                let mi = mparent
                    .iter()
                    .position(|&c| c == mark_id)
                    .ok_or(EditError::InternalInvariant)?;
                mparent[mi] = leaf;
            }
            log.text.push(TextDelta {
                start: open.start,
                end: close.end,
                old_bytes: old,
                new_bytes: content.to_string(),
            });
            self.postop();
            let sel = VisualSelection::new(range.start.min(range.end), range.start.max(range.end));
            return Ok(self.record(log, SemanticOp::ToggleMark { mark, range }, sel, sel));
        }

        // ---- 添加标记：selection 在段落直属的单个 Text 叶内 ----
        if parent_kind != NodeKind::Paragraph {
            return Err(EditError::MarkShapeMismatch);
        }
        let open_new = marker;
        let close_new = marker;
        let content: String = self
            .byte_slice(ByteRange::new(b, e))
            .ok_or(EditError::InternalInvariant)?
            .into_owned();
        // 选中内容以空白开头/结尾时，comrak 不会把它当标记
        // （"**a **" 是普通文本）：拒绝，避免 tree≠reparse。
        if content.starts_with(' ') || content.ends_with(' ') {
            return Err(EditError::MarkShapeMismatch);
        }
        let has_pre = b > lr.start;
        let has_post = e < lr.end;
        // comrak 兼容性预检：拼出“前导字符 + 新标记 + 相邻后缀字符”的片段并重新解析，
        // 验证 comrak 恰好生成一个包裹 content 的 Emph/Strong。
        // 若失败（如转义字符紧贴标记、intraword 歧义），拒绝而不产生 tree≠reparse。
        {
            let mut snippet = String::new();
            if has_pre && let Some(c) = self.byte_slice(ByteRange::new(ByteOffset(b.0 - 1), b)) {
                snippet.push_str(c.chars().next().unwrap_or(' ').to_string().as_str());
            }
            snippet.push_str(marker);
            snippet.push_str(&content);
            snippet.push_str(marker);
            if has_post && let Some(c) = self.byte_slice(ByteRange::new(e, ByteOffset(e.0 + 1))) {
                snippet.push_str(c.chars().next().unwrap_or(' ').to_string().as_str());
            }
            let parse_res = crate::parse(snippet.as_bytes());
            match parse_res {
                Ok(reparsed) => {
                    let proj =
                        crate::serialize::tree_semantic_projection(reparsed.tree(), &snippet);
                    // 期望恰好一个 Emph/Strong，且它的直接文本子节点拼起来 == 未转义 content。
                    let mut emph_count = 0usize;
                    let mut in_emph = false;
                    let mut emph_text = String::new();
                    for (k, t) in &proj {
                        match k {
                            NodeKind::Emph | NodeKind::Strong => {
                                emph_count += 1;
                                if emph_count == 1 {
                                    in_emph = true;
                                }
                            }
                            NodeKind::Text if in_emph => emph_text.push_str(t.as_str()),
                            NodeKind::Text => {}
                            _ => {}
                        }
                    }
                    let unescaped = crate::serialize::unescape_text(&content);
                    if emph_count != 1 || emph_text != unescaped {
                        return Err(EditError::MarkShapeMismatch);
                    }
                }
                Err(_) => return Err(EditError::MarkShapeMismatch),
            }
        }
        let open_len = marker.len();
        let close_len = marker.len();

        // 段首包裹（b == 段落起点）时，两次 shift 会把段落及其余起点相重的
        // 祖先容器（如 Document/Item）整体右移，编辑后必须把起点钉回 b。
        let para_start_before = self.tree.node(leaf_parent).source.map(|r| r.start);
        let para_end_before = self.tree.node(leaf_parent).source.map(|r| r.end);

        let mut log = self.begin_log(&[leaf], &[leaf_parent]);
        // 文本：净增量 = 把 [b, e) 替换为 marker + content + marker。
        let new_bytes = format!("{open_new}{content}{close_new}");
        let old: String = self
            .byte_slice(ByteRange::new(b, e))
            .ok_or(EditError::InternalInvariant)?
            .into_owned();
        self.apply_rope_replace(b, e, &new_bytes)?;
        // 树：两次单点插入（open 在 b，close 在当前 e+len(open) 处）。
        self.tree.shift_from(b, open_len as i64)?;
        let close_at = ByteOffset(b.0 + open_len as u32 + (e.0 - b.0));
        self.tree.shift_from(close_at, close_len as i64)?;
        // 仅当选中尾部紧贴段落尾（e == lr.end == 段落原终点）时，close
        // 标记恰好落在段尾（shift_from 对 end==t 的节点不动），段尾才需要
        // 手动扩展：直接块（段落）以及 end 与该段终点重合的祖先容器
        // （Item/List/BlockQuote 等），否则容器落后于末子产生负间隙。
        // 若选区只到叶尾而未到段尾（e == lr.end < 段尾），shift2 的
        // a<t<=b 规则已自动扩展段落与祖先，此处再扩会多算 close_len。
        if !has_post && para_end_before == Some(lr.end) {
            let old_end = self.tree.node(leaf_parent).source.map(|r| r.end);
            if let Some(pr) = self.tree.node(leaf_parent).source {
                self.set_range(
                    leaf_parent,
                    ByteRange::new(pr.start, byte_checked_add(pr.end, close_len as u32)?),
                )?;
            }
            if let Some(prev_end) = old_end {
                self.extend_containers_ending_at(leaf_parent, prev_end, close_len as i64)?;
            }
        }
        // 段首包裹：把段落起点（及其余起点相重的祖先）钉回 b。
        // （end 已由 shift + !has_post 分支正确增长，只需拉回起点。）
        if para_start_before == Some(b) {
            let cur = self.tree.node(leaf_parent).source.unwrap();
            self.set_range(leaf_parent, ByteRange::new(b, cur.end))?;
            self.reanchor_containers_starting_at(leaf_parent, b, open_len as u32)?;
        }
        // 坐标组装：
        //   pre 叶（有前部时保留原 ID）：[lr.start, b)
        //   sel 叶（无前部时用原 ID）：[b+open, e+open)
        //   post 叶：[e+open+close, lr.end + open + close)
        let (pre_id, sel_id, post_id): (Option<NodeId>, NodeId, Option<NodeId>) = if has_pre {
            self.set_range(leaf, ByteRange::new(lr.start, b))?;
            let sel = self.tree.push(
                NodeKind::Text,
                Some(ByteRange::new(
                    ByteOffset(b.0 + open_len as u32),
                    ByteOffset(e.0 + open_len as u32),
                )),
                NodeFlags::NONE,
                Vec::new(),
            );
            self.log_created(&mut log, sel);
            let post = if has_post {
                let p = self.tree.push(
                    NodeKind::Text,
                    Some(ByteRange::new(
                        ByteOffset(e.0 + open_len as u32 + close_len as u32),
                        ByteOffset(lr.end.0 + open_len as u32 + close_len as u32),
                    )),
                    NodeFlags::NONE,
                    Vec::new(),
                );
                self.log_created(&mut log, p);
                Some(p)
            } else {
                None
            };
            (Some(leaf), sel, post)
        } else {
            // 无前部：原叶直接充当选中内容，坐标右移 open。
            self.set_range(
                leaf,
                ByteRange::new(
                    ByteOffset(b.0 + open_len as u32),
                    ByteOffset(e.0 + open_len as u32),
                ),
            )?;
            let post = if has_post {
                let p = self.tree.push(
                    NodeKind::Text,
                    Some(ByteRange::new(
                        ByteOffset(e.0 + open_len as u32 + close_len as u32),
                        ByteOffset(lr.end.0 + open_len as u32 + close_len as u32),
                    )),
                    NodeFlags::NONE,
                    Vec::new(),
                );
                self.log_created(&mut log, p);
                Some(p)
            } else {
                None
            };
            (None, leaf, post)
        };
        // 标记节点覆盖 [b, e + open + close)。
        let mark_node = self.tree.push(
            expected_kind.clone(),
            Some(ByteRange::new(
                b,
                ByteOffset(e.0 + open_len as u32 + close_len as u32),
            )),
            NodeFlags::NONE,
            vec![sel_id],
        );
        self.log_created(&mut log, mark_node);
        // 替换段落中的叶位置：pre?, mark, post?（都位于原下标处）。
        {
            let sibs = &mut self.tree.node_mut(leaf_parent).children;
            let mut replacement: Vec<NodeId> = Vec::new();
            if let Some(p) = pre_id {
                replacement.push(p);
            }
            replacement.push(mark_node);
            if let Some(p) = post_id {
                replacement.push(p);
            }
            sibs.splice(idx..=idx, replacement);
        }
        log.text.push(TextDelta {
            start: b,
            end: e,
            old_bytes: old,
            new_bytes,
        });
        self.postop();
        let sel = VisualSelection::new(range.start.min(range.end), range.start.max(range.end));
        Ok(self.record(log, SemanticOp::ToggleMark { mark, range }, sel, sel))
    }

    pub fn root_id(&self) -> NodeId {
        self.tree.root()
    }

    /// 存活节点的原始 `NodeId` 遍历。与 `semantic_tree()` 的压缩视图不同，
    /// 这些 ID 可继续传给结构事务。
    pub fn live_node_ids(&self) -> Vec<NodeId> {
        self.tree.alive_dfs()
    }
    /// 返回当前存活节点的类型。调用方不应把 `NodeId` 当作跨会话 ID。
    pub fn node_kind(&self, id: NodeId) -> Option<NodeKind> {
        let node = self.tree.nodes.get(id.0 as usize)?;
        (!node.deleted).then(|| node.kind.clone())
    }

    /// 返回当前存活节点的源范围。
    pub fn source_range(&self, id: NodeId) -> Option<ByteRange> {
        let node = self.tree.nodes.get(id.0 as usize)?;
        (!node.deleted).then_some(node.source).flatten()
    }

    /// 返回当前存活节点的子节点。
    pub fn children(&self, id: NodeId) -> Option<Vec<NodeId>> {
        let node = self.tree.nodes.get(id.0 as usize)?;
        (!node.deleted).then(|| node.children.clone())
    }

    /// 删除一个完整节点。Raw/HTML 的内部字节永远只会随整个节点删除。
    pub fn delete_node(&mut self, id: NodeId) -> Result<EditRecord, EditError> {
        let range = self.source_range(id).ok_or(EditError::InvalidNode)?;
        let old = self
            .byte_slice(range)
            .ok_or(EditError::InvalidNode)?
            .into_owned();
        let mut start = range.start.0 as usize;
        let mut end = range.end.0 as usize;
        let bytes = self.bytes();
        if end < bytes.len() && bytes[end] == b'\r' {
            end += 1;
        }
        if end < bytes.len() && bytes[end] == b'\n' {
            end += 1;
        } else if start > 0 && bytes[start - 1] == b'\n' {
            start -= 1;
        }
        let mut replacement = String::new();
        if start == range.start.0 as usize && end == range.end.0 as usize {
            replacement.push_str("");
        }
        let mut next = String::from_utf8(bytes).map_err(|_| EditError::InvalidMarkdown)?;
        next.replace_range(start..end, &replacement);
        let before = self.visual_len();
        self.rebuild_from_text(
            next,
            SemanticOp::EditNode {
                node: id,
                text: old,
            },
            VisualSelection::collapsed(VisualPos(0)),
            VisualSelection::collapsed(VisualPos(before)),
        )
    }

    /// 修改节点的结构化源片段。Raw/HTML 禁止走此入口；它们只能整体移动或删除。
    pub fn edit_node_text(&mut self, id: NodeId, text: &str) -> Result<EditRecord, EditError> {
        let kind = self.node_kind(id).ok_or(EditError::InvalidNode)?;
        if kind.is_raw()
            || matches!(kind, NodeKind::CodeBlock { .. } | NodeKind::Code { .. })
            || self.has_raw_descendant(id)
        {
            return Err(EditError::RawNode);
        }
        let range = self.source_range(id).ok_or(EditError::InvalidNode)?;
        let mut next = String::from_utf8(self.bytes()).map_err(|_| EditError::InvalidMarkdown)?;
        next.replace_range(range.start.0 as usize..range.end.0 as usize, text);
        self.rebuild_from_text(
            next,
            SemanticOp::EditNode {
                node: id,
                text: text.to_string(),
            },
            VisualSelection::collapsed(VisualPos(0)),
            VisualSelection::collapsed(VisualPos(0)),
        )
    }

    /// 普通粘贴之外的显式 Markdown 粘贴命令。这里不做文字转义，解析失败则
    /// 不改动会话；HTML 仍由解析器保留为 Raw，不会执行。
    pub fn paste_markdown(
        &mut self,
        range: VisualRange,
        markdown: &str,
    ) -> Result<EditRecord, EditError> {
        if markdown.contains('\0') {
            return Err(EditError::InvalidMarkdown);
        }
        let (start, end) = self.range_to_bytes(&range)?;
        let mut next = String::from_utf8(self.bytes()).map_err(|_| EditError::InvalidMarkdown)?;
        next.replace_range(start.0 as usize..end.0 as usize, markdown);
        self.rebuild_from_text(
            next,
            SemanticOp::PasteMarkdown {
                range,
                text: markdown.to_string(),
            },
            VisualSelection::collapsed(range.start),
            VisualSelection::collapsed(range.start),
        )
    }

    /// 用 CommonMark/GFM 规则把块转换为段落、标题或引用。
    pub fn set_block_type(
        &mut self,
        id: NodeId,
        target: BlockKind,
    ) -> Result<EditRecord, EditError> {
        let current = self.node_kind(id).ok_or(EditError::InvalidNode)?;
        if current.is_raw() || matches!(current, NodeKind::CodeBlock { .. }) {
            return Err(EditError::RawNode);
        }
        let range = self.source_range(id).ok_or(EditError::InvalidNode)?;
        let source = self
            .byte_slice(range)
            .ok_or(EditError::InvalidNode)?
            .into_owned();
        let body = block_body(&source, &current);
        let replacement = match target {
            BlockKind::Paragraph => body,
            BlockKind::Heading { level, setext } => {
                let level = level.clamp(1, 6) as usize;
                if setext {
                    format!(
                        "{body}\n{}",
                        if level <= 1 { "=" } else { "-" }.repeat(body.len().max(1))
                    )
                } else {
                    format!("{} {body}", "#".repeat(level))
                }
            }
            BlockKind::Quote => body
                .lines()
                .map(|line| format!("> {line}"))
                .collect::<Vec<_>>()
                .join("\n"),
        };
        self.replace_node_source(
            id,
            replacement,
            SemanticOp::SetBlockType {
                node: id,
                kind: target,
            },
        )
    }

    /// 勾选/取消一个 GFM task item，只替换 checkbox 符号。
    pub fn set_task_checked(
        &mut self,
        item: NodeId,
        checked: bool,
    ) -> Result<EditRecord, EditError> {
        let kind = self.node_kind(item).ok_or(EditError::InvalidNode)?;
        let NodeKind::TaskItem {
            symbol_source: Some(symbol),
            ..
        } = kind
        else {
            return Err(EditError::InvalidNode);
        };
        let mut next = String::from_utf8(self.bytes()).map_err(|_| EditError::InvalidMarkdown)?;
        let value = if checked { "x" } else { " " };
        next.replace_range(symbol.start.0 as usize..symbol.end.0 as usize, value);
        self.rebuild_from_text(
            next,
            SemanticOp::ToggleTask { item, checked },
            VisualSelection::collapsed(VisualPos(0)),
            VisualSelection::collapsed(VisualPos(0)),
        )
    }

    /// 编辑行内代码或 fenced code block 的可视文本；围栏长度至少比正文中
    /// 最长连续 fence 多一个，避免用户输入制造提前闭合。
    pub fn edit_code_content(
        &mut self,
        node: NodeId,
        content: &str,
    ) -> Result<EditRecord, EditError> {
        let kind = self.node_kind(node).ok_or(EditError::InvalidNode)?;
        let replacement = match kind {
            NodeKind::Code { .. } => {
                if content.contains('\n') || content.contains('\r') {
                    return Err(EditError::NewlineInInput);
                }
                let count = longest_run(content, '`');
                let fence = "`".repeat(count.max(1) + 1);
                format!("{fence}{content}{fence}")
            }
            NodeKind::CodeBlock {
                fenced,
                fence_char,
                info,
                ..
            } => {
                if !fenced {
                    content.to_string()
                } else {
                    let longest = longest_run(content, fence_char as char);
                    let fence = (fence_char as char).to_string().repeat(longest.max(3) + 1);
                    let suffix = if info.is_empty() {
                        String::new()
                    } else {
                        format!(" {info}")
                    };
                    format!("{fence}{suffix}\n{content}\n{fence}")
                }
            }
            _ => return Err(EditError::InvalidNode),
        };
        self.replace_node_source(
            node,
            replacement,
            SemanticOp::EditNode {
                node,
                text: content.to_string(),
            },
        )
    }

    /// 表格单元编辑。表格分隔符和其他行列不会被重新格式化。
    pub fn edit_table_cell(
        &mut self,
        table: NodeId,
        row: usize,
        column: usize,
        text: &str,
    ) -> Result<EditRecord, EditError> {
        let kind = self.node_kind(table).ok_or(EditError::InvalidNode)?;
        if !matches!(kind, NodeKind::Table { .. }) {
            return Err(EditError::InvalidTable);
        }
        let rows = self
            .children(table)
            .ok_or(EditError::InvalidTable)?
            .into_iter()
            .filter(|id| matches!(self.node_kind(*id), Some(NodeKind::TableRow { .. })))
            .collect::<Vec<_>>();
        let row_id = *rows.get(row).ok_or(EditError::InvalidTable)?;
        let cells = self
            .children(row_id)
            .ok_or(EditError::InvalidTable)?
            .into_iter()
            .filter(|id| matches!(self.node_kind(*id), Some(NodeKind::TableCell)))
            .collect::<Vec<_>>();
        let cell = *cells.get(column).ok_or(EditError::InvalidTable)?;
        let cell_kind = self.node_kind(cell).ok_or(EditError::InvalidTable)?;
        let source = self.source_range(cell).ok_or(EditError::SyntheticCell)?;
        if matches!(cell_kind, NodeKind::TableCell)
            && self
                .byte_slice(source)
                .map(|s| {
                    s.trim_matches(|c| c == '|' || c == ' ' || c == '\t')
                        .is_empty()
                })
                .unwrap_or(true)
        {
            return Err(EditError::SyntheticCell);
        }
        if text.contains('\n') || text.contains('\r') {
            return Err(EditError::NewlineInInput);
        }
        let escaped = escape_text(text);
        self.replace_node_source(
            cell,
            escaped,
            SemanticOp::EditNode {
                node: cell,
                text: text.to_string(),
            },
        )
    }

    /// 在列表 item 内拆分出紧邻的第二 item。`at` 使用编辑会话的视觉位置，
    /// 调用方应先把可见投影边界映射到会话位置。
    pub fn split_list_item(
        &mut self,
        item: NodeId,
        at: VisualPos,
    ) -> Result<EditRecord, EditError> {
        let kind = self.node_kind(item).ok_or(EditError::InvalidNode)?;
        if !matches!(kind, NodeKind::Item | NodeKind::TaskItem { .. }) {
            return Err(EditError::InvalidNode);
        }
        let range = self.source_range(item).ok_or(EditError::InvalidNode)?;
        let byte = self
            .visual_to_byte(at)
            .ok_or(EditError::PositionOutOfBounds)?;
        if byte <= range.start || byte >= range.end {
            return Err(EditError::EmptyHalf);
        }
        let source = self
            .byte_slice(range)
            .ok_or(EditError::InvalidNode)?
            .into_owned();
        let marker = source
            .trim_start()
            .split_once(char::is_whitespace)
            .map(|(marker, _)| marker)
            .ok_or(EditError::InvalidNode)?;
        self.replace_source_range(
            ByteRange::new(byte, byte),
            format!("\n{marker} "),
            SemanticOp::EditNode {
                node: item,
                text: "split-item".into(),
            },
        )
    }

    /// 在列表中插入一个新的 item。普通文本仍按字面写入；需要结构时使用
    /// `paste_markdown`，避免粘贴内容偷偷改变块结构。
    pub fn insert_list_item(
        &mut self,
        list: NodeId,
        index: usize,
        text: &str,
    ) -> Result<EditRecord, EditError> {
        let NodeKind::List {
            ordered,
            start,
            delimiter,
            bullet_char,
            ..
        } = self.node_kind(list).ok_or(EditError::InvalidNode)?
        else {
            return Err(EditError::InvalidNode);
        };
        if text.contains('\n') || text.contains('\r') {
            return Err(EditError::NewlineInInput);
        }
        let items = self
            .children(list)
            .ok_or(EditError::InvalidNode)?
            .into_iter()
            .filter(|id| {
                matches!(
                    self.node_kind(*id),
                    Some(NodeKind::Item | NodeKind::TaskItem { .. })
                )
            })
            .collect::<Vec<_>>();
        if index > items.len() {
            return Err(EditError::InvalidNode);
        }
        let marker = if ordered {
            let number = start.saturating_add(index);
            match delimiter {
                crate::model::ListDelimiter::Period => format!("{number}. "),
                crate::model::ListDelimiter::Paren => format!("{number}) "),
            }
        } else {
            format!("{} ", bullet_char.unwrap_or(b'-') as char)
        };
        let at = if let Some(item) = items.get(index) {
            self.source_range(*item)
                .ok_or(EditError::InvalidNode)?
                .start
        } else {
            self.source_range(list).ok_or(EditError::InvalidNode)?.end
        };
        let insertion = if at.0 > 0 && self.bytes().get(at.0 as usize - 1) == Some(&b'\n') {
            format!("{marker}{text}")
        } else {
            format!("\n{marker}{text}")
        };
        self.replace_source_range(
            ByteRange::new(at, at),
            insertion,
            SemanticOp::EditNode {
                node: list,
                text: text.to_string(),
            },
        )
    }

    pub fn indent_list_item(&mut self, item: NodeId) -> Result<EditRecord, EditError> {
        self.rewrite_item_indentation(item, 2)
    }

    pub fn outdent_list_item(&mut self, item: NodeId) -> Result<EditRecord, EditError> {
        self.rewrite_item_indentation(item, -2)
    }

    fn rewrite_item_indentation(
        &mut self,
        item: NodeId,
        amount: i32,
    ) -> Result<EditRecord, EditError> {
        let range = self.source_range(item).ok_or(EditError::InvalidNode)?;
        let bytes = self.bytes();
        let mut start = range.start.0 as usize;
        while start > 0 && bytes[start - 1] != b'\n' {
            start -= 1;
        }
        let source = self
            .byte_slice(ByteRange::new(ByteOffset(start as u32), range.end))
            .ok_or(EditError::InvalidNode)?
            .into_owned();
        let replacement = if amount > 0 {
            source
                .lines()
                .map(|line| format!("{}{}", " ".repeat(amount as usize), line))
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            source
                .lines()
                .map(|line| line.strip_prefix("  ").unwrap_or(line))
                .collect::<Vec<_>>()
                .join("\n")
        };
        self.replace_source_range(
            ByteRange::new(ByteOffset(start as u32), range.end),
            replacement,
            SemanticOp::EditNode {
                node: item,
                text: String::new(),
            },
        )
    }

    pub fn merge_list_items(
        &mut self,
        first: NodeId,
        second: NodeId,
    ) -> Result<EditRecord, EditError> {
        let first_range = self.source_range(first).ok_or(EditError::InvalidNode)?;
        let second_range = self.source_range(second).ok_or(EditError::InvalidNode)?;
        let parent = self.parent_of(first).ok_or(EditError::InvalidNode)?;
        if self.parent_of(second) != Some(parent) {
            return Err(EditError::InvalidNode);
        }
        let children = self.children(parent).ok_or(EditError::InvalidNode)?;
        let first_pos = children.iter().position(|id| *id == first);
        if first_pos.and_then(|i| children.get(i + 1)).copied() != Some(second) {
            return Err(EditError::InvalidNode);
        }
        let left = self
            .byte_slice(first_range)
            .ok_or(EditError::InvalidNode)?
            .into_owned();
        let right = self
            .byte_slice(second_range)
            .ok_or(EditError::InvalidNode)?
            .into_owned();
        let right_body = right
            .trim_start()
            .trim_start_matches(|c: char| c.is_ascii_digit())
            .trim_start_matches(['.', ')', '-', '*', '+'])
            .trim_start()
            .to_string();
        self.replace_source_range(
            ByteRange::new(first_range.start, second_range.end),
            format!("{left} {right_body}"),
            SemanticOp::EditNode {
                node: first,
                text: right_body,
            },
        )
    }

    pub fn move_node_after(
        &mut self,
        node: NodeId,
        after: NodeId,
    ) -> Result<EditRecord, EditError> {
        let parent = self.parent_of(node).ok_or(EditError::InvalidNode)?;
        if self.parent_of(after) != Some(parent) {
            return Err(EditError::InvalidNode);
        }
        let siblings = self.children(parent).ok_or(EditError::InvalidNode)?;
        let a = siblings
            .iter()
            .position(|id| *id == node)
            .ok_or(EditError::InvalidNode)?;
        let b = siblings
            .iter()
            .position(|id| *id == after)
            .ok_or(EditError::InvalidNode)?;
        if a.abs_diff(b) != 1 {
            return Err(EditError::InvalidNode);
        }
        let left = self.source_range(node).ok_or(EditError::InvalidNode)?;
        let right = self.source_range(after).ok_or(EditError::InvalidNode)?;
        let (first, second) = if left.start < right.start {
            (node, after)
        } else {
            (after, node)
        };
        let first_range = self.source_range(first).ok_or(EditError::InvalidNode)?;
        let second_range = self.source_range(second).ok_or(EditError::InvalidNode)?;
        let bytes = self.bytes();
        let first_text = self.byte_slice(first_range).ok_or(EditError::InvalidNode)?;
        let between = String::from_utf8(
            bytes[first_range.end.0 as usize..second_range.start.0 as usize].to_vec(),
        )
        .map_err(|_| EditError::InvalidMarkdown)?;
        let second_text = self
            .byte_slice(second_range)
            .ok_or(EditError::InvalidNode)?;
        self.replace_source_range(
            ByteRange::new(first_range.start, second_range.end),
            format!("{second_text}{between}{first_text}"),
            SemanticOp::EditNode {
                node,
                text: "move".into(),
            },
        )
    }

    pub fn insert_table_row(&mut self, table: NodeId, row: usize) -> Result<EditRecord, EditError> {
        let rows = self.table_rows(table)?;
        if row > rows.len() {
            return Err(EditError::InvalidTable);
        }
        let columns = rows
            .first()
            .and_then(|id| self.children(*id))
            .map(|cells| cells.len())
            .filter(|count| *count > 0)
            .ok_or(EditError::InvalidTable)?;
        let at = if let Some(id) = rows.get(row) {
            self.source_range(*id).ok_or(EditError::InvalidTable)?.start
        } else {
            self.source_range(table).ok_or(EditError::InvalidTable)?.end
        };
        let text = format!(
            "\n|{}|",
            (0..columns).map(|_| "  ").collect::<Vec<_>>().join("|")
        );
        self.replace_source_range(
            ByteRange::new(at, at),
            text,
            SemanticOp::EditNode {
                node: table,
                text: "insert-row".into(),
            },
        )
    }

    pub fn delete_table_row(&mut self, table: NodeId, row: usize) -> Result<EditRecord, EditError> {
        let rows = self.table_rows(table)?;
        let row_id = *rows.get(row).ok_or(EditError::InvalidTable)?;
        if row == 0 {
            return Err(EditError::InvalidTable);
        }
        self.delete_node(row_id)
    }

    pub fn insert_table_column(
        &mut self,
        table: NodeId,
        column: usize,
    ) -> Result<EditRecord, EditError> {
        let rows = self.table_rows(table)?;
        let columns = rows
            .first()
            .and_then(|id| self.children(*id))
            .map(|cells| cells.len())
            .ok_or(EditError::InvalidTable)?;
        if column > columns {
            return Err(EditError::InvalidTable);
        }
        let source = self.table_source(table)?;
        let next = rewrite_table_lines(&source, |line, index| {
            table_line_insert_column(line, column, index == 1)
        });
        self.replace_node_source(
            table,
            next,
            SemanticOp::EditNode {
                node: table,
                text: "insert-column".into(),
            },
        )
    }

    pub fn delete_table_column(
        &mut self,
        table: NodeId,
        column: usize,
    ) -> Result<EditRecord, EditError> {
        let rows = self.table_rows(table)?;
        let columns = rows
            .first()
            .and_then(|id| self.children(*id))
            .map(|cells| cells.len())
            .ok_or(EditError::InvalidTable)?;
        if column >= columns {
            return Err(EditError::InvalidTable);
        }
        let source = self.table_source(table)?;
        let next = rewrite_table_lines(&source, |line, _| table_line_delete_column(line, column));
        self.replace_node_source(
            table,
            next,
            SemanticOp::EditNode {
                node: table,
                text: "delete-column".into(),
            },
        )
    }

    pub fn set_link_target(&mut self, node: NodeId, target: &str) -> Result<EditRecord, EditError> {
        self.replace_link_target(node, target)
    }

    pub fn set_image_target(
        &mut self,
        node: NodeId,
        target: &str,
    ) -> Result<EditRecord, EditError> {
        self.replace_link_target(node, target)
    }

    fn replace_link_target(&mut self, node: NodeId, target: &str) -> Result<EditRecord, EditError> {
        if target.contains('\n') || target.contains('\r') {
            return Err(EditError::InvalidMarkdown);
        }
        if !matches!(
            self.node_kind(node),
            Some(NodeKind::Link { .. } | NodeKind::Image { .. })
        ) {
            return Err(EditError::InvalidNode);
        }
        let range = self.source_range(node).ok_or(EditError::InvalidNode)?;
        let source = self
            .byte_slice(range)
            .ok_or(EditError::InvalidNode)?
            .into_owned();
        let open = source.find("](").ok_or(EditError::InvalidNode)? + 2;
        let close = source.rfind(')').ok_or(EditError::InvalidNode)?;
        if open > close {
            return Err(EditError::InvalidNode);
        }
        let mut replacement = source.clone();
        let suffix = source[open..close]
            .split_once(char::is_whitespace)
            .map(|(_, title)| format!(" {title}"))
            .unwrap_or_default();
        replacement.replace_range(open..close, &format!("{target}{suffix}"));
        self.replace_node_source(
            node,
            replacement,
            SemanticOp::EditNode {
                node,
                text: target.to_string(),
            },
        )
    }

    fn table_rows(&self, table: NodeId) -> Result<Vec<NodeId>, EditError> {
        if !matches!(self.node_kind(table), Some(NodeKind::Table { .. })) {
            return Err(EditError::InvalidTable);
        }
        Ok(self
            .children(table)
            .ok_or(EditError::InvalidTable)?
            .into_iter()
            .filter(|id| matches!(self.node_kind(*id), Some(NodeKind::TableRow { .. })))
            .collect())
    }

    fn table_source(&self, table: NodeId) -> Result<String, EditError> {
        let range = self.source_range(table).ok_or(EditError::InvalidTable)?;
        self.byte_slice(range)
            .map(|s| s.into_owned())
            .ok_or(EditError::InvalidTable)
    }

    fn replace_source_range(
        &mut self,
        range: ByteRange,
        replacement: String,
        op: SemanticOp,
    ) -> Result<EditRecord, EditError> {
        let mut next = String::from_utf8(self.bytes()).map_err(|_| EditError::InvalidMarkdown)?;
        next.replace_range(range.start.0 as usize..range.end.0 as usize, &replacement);
        self.rebuild_from_text(
            next,
            op,
            VisualSelection::collapsed(VisualPos(0)),
            VisualSelection::collapsed(VisualPos(0)),
        )
    }

    fn replace_node_source(
        &mut self,
        id: NodeId,
        replacement: String,
        op: SemanticOp,
    ) -> Result<EditRecord, EditError> {
        let range = self.source_range(id).ok_or(EditError::InvalidNode)?;
        let mut next = String::from_utf8(self.bytes()).map_err(|_| EditError::InvalidMarkdown)?;
        next.replace_range(range.start.0 as usize..range.end.0 as usize, &replacement);
        self.rebuild_from_text(
            next,
            op,
            VisualSelection::collapsed(VisualPos(0)),
            VisualSelection::collapsed(VisualPos(0)),
        )
    }

    fn rebuild_from_text(
        &mut self,
        next: String,
        op: SemanticOp,
        selection_before: VisualSelection,
        selection_after: VisualSelection,
    ) -> Result<EditRecord, EditError> {
        let old = self.text.to_string();
        if old == next {
            let snapshot = self.snapshot_all();
            let step = Step {
                text: Vec::new(),
                nodes: snapshot,
                parents: Vec::new(),
            };
            let record = EditRecord {
                op,
                selection_before,
                selection_after,
                forward: step.clone(),
                inverse: step,
            };
            self.history.push(record.clone());
            return Ok(record);
        }
        let parsed = crate::parse(next.as_bytes()).map_err(|_| EditError::InvalidMarkdown)?;
        let mut log = self.begin_log(&[], &[]);
        let (prefix, old_end, new_end) = text_delta_bounds(&old, &next);
        let replacement = &next[prefix..new_end];
        self.apply_rope_replace(
            ByteOffset(prefix as u32),
            ByteOffset(old_end as u32),
            replacement,
        )?;
        self.rebuild_tree_preserving_ids(&parsed, &old, &next);
        log.text.push(TextDelta {
            start: ByteOffset(prefix as u32),
            end: ByteOffset(old_end as u32),
            old_bytes: old[prefix..old_end].to_string(),
            new_bytes: replacement.to_string(),
        });
        self.postop();
        Ok(self.record(log, op, selection_before, selection_after))
    }

    /// 撤销：应用最顶层记录的反向步骤。
    pub fn undo(&mut self) -> Result<Option<EditRecord>, EditError> {
        let Some(rec) = self.history.pop_undo() else {
            return Ok(None);
        };
        self.apply_step(&rec.inverse, false)?;
        self.postop();
        Ok(Some(rec))
    }

    /// 重做：应用最顶层记录的正向步骤。
    pub fn redo(&mut self) -> Result<Option<EditRecord>, EditError> {
        let Some(rec) = self.history.pop_redo() else {
            return Ok(None);
        };
        self.apply_step(&rec.forward, true)?;
        self.postop();
        Ok(Some(rec))
    }

    // ---- 内部辅助 ----

    fn rebuild_tree_preserving_ids(
        &mut self,
        parsed: &DocumentSession,
        old_text: &str,
        new_text: &str,
    ) {
        let old_tree = self.tree.clone();
        let old_len = old_tree.len();
        let new_tree = parsed.tree();
        let mut old_parent = vec![None; old_len];
        for (index, node) in old_tree.nodes.iter().enumerate() {
            if node.deleted {
                continue;
            }
            for &child in &node.children {
                old_parent[child.0 as usize] = Some(NodeId(index as u32));
            }
        }
        let mut new_parent = vec![None; new_tree.len()];
        for (index, node) in new_tree.nodes().iter().enumerate() {
            for &child in &node.children {
                new_parent[child.0 as usize] = Some(NodeId(index as u32));
            }
        }
        let mut mapping = vec![None; new_tree.len()];
        let mut used = vec![false; old_len];
        mapping[new_tree.root().0 as usize] = Some(old_tree.root());
        used[old_tree.root().0 as usize] = true;
        for new_id in new_tree.iter() {
            if new_id == new_tree.root() {
                continue;
            }
            let Some(new_parent_id) = new_parent[new_id.0 as usize] else {
                continue;
            };
            let Some(old_parent_id) = mapping[new_parent_id.0 as usize] else {
                continue;
            };
            let new_node = new_tree.node(new_id);
            let new_source = new_node
                .source
                .and_then(|range| new_text.get(range.to_byte_range()))
                .unwrap_or_default();
            let candidate = old_tree
                .node(old_parent_id)
                .children
                .iter()
                .copied()
                .find(|old_id| {
                    let index = old_id.0 as usize;
                    if used[index] {
                        return false;
                    }
                    let old_node = old_tree.node(*old_id);
                    if old_node.kind != new_node.kind {
                        return false;
                    }
                    let old_source = old_node
                        .source
                        .and_then(|range| old_text.get(range.to_byte_range()))
                        .unwrap_or_default();
                    old_source == new_source
                });
            if let Some(old_id) = candidate {
                mapping[new_id.0 as usize] = Some(old_id);
                used[old_id.0 as usize] = true;
            }
        }
        let mut next_id = old_len;
        for slot in &mut mapping {
            if slot.is_none() {
                *slot = Some(NodeId(u32::try_from(next_id).unwrap_or(u32::MAX)));
                next_id += 1;
            }
        }
        let mut nodes = old_tree.nodes;
        for node in &mut nodes {
            node.source = None;
            node.children.clear();
            node.gaps.clear();
            node.deleted = true;
        }
        while nodes.len() < next_id {
            nodes.push(EditNode {
                kind: NodeKind::Raw {
                    detail: RawDetail::ComrakRaw,
                },
                source: None,
                flags: NodeFlags::UNATTRIBUTED,
                children: Vec::new(),
                gaps: Vec::new(),
                deleted: true,
            });
        }
        for new_id in new_tree.iter() {
            let target = mapping[new_id.0 as usize].expect("mapping filled");
            let parsed_node = new_tree.node(new_id);
            let children = parsed_node
                .children
                .iter()
                .map(|child| mapping[child.0 as usize].expect("child mapping filled"))
                .collect();
            let node = &mut nodes[target.0 as usize];
            node.kind = parsed_node.kind.clone();
            node.source = parsed_node.source;
            node.flags = parsed_node.flags;
            node.children = children;
            node.deleted = false;
        }
        self.tree = EditableTree {
            nodes,
            root: mapping[new_tree.root().0 as usize].expect("root mapping filled"),
        };
        self.tree.recompute_gaps_all();
        self.encoding = parsed.encoding();
    }

    fn parent_of(&self, id: NodeId) -> Option<NodeId> {
        self.tree.parent_index()[id.0 as usize]
    }
    fn has_raw_descendant(&self, id: NodeId) -> bool {
        self.tree.node(id).kind.is_raw()
            || self
                .tree
                .node(id)
                .children
                .iter()
                .copied()
                .any(|child| self.has_raw_descendant(child))
    }

    /// 变更前捕获：从 `child` 向上收集 end 与 `prev_end` 重合的祖先容器
    /// （Item/List/BlockQuote 等）。必须在任何 shift/set_range 之前调用——
    /// 之后容器可能被平移“撞到” prev_end，造成误配。
    fn containers_ending_at(&self, child: NodeId, prev_end: ByteOffset) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut cur = self.parent_of(child);
        while let Some(id) = cur {
            let n = self.tree.node(id);
            if n.deleted {
                break;
            }
            match n.source {
                Some(src) if src.end == prev_end => out.push(id),
                _ => break,
            }
            cur = self.parent_of(id);
        }
        out
    }

    /// 向上扩展/收缩：end 与 `prev_end` 重合的祖先容器（Item/List/
    /// BlockQuote 等）同步延展 `delta`（可为负，删除时收缩），防止子块
    /// 尾随增减后容器落后于其末子或越过其他兄弟（recompute_gaps_for
    /// 会产生负间隙/重叠）。shift_range 对 end==t 的节点一律不动，故这类
    /// 重合必须在调用方显式钉回。
    fn extend_containers_ending_at(
        &mut self,
        child: NodeId,
        prev_end: ByteOffset,
        delta: i64,
    ) -> Result<(), EditError> {
        let mut cur = self.parent_of(child);
        while let Some(id) = cur {
            let n = self.tree.node(id);
            if n.deleted {
                break;
            }
            match n.source {
                Some(src) if src.end == prev_end => {
                    self.set_range(
                        id,
                        ByteRange::new(src.start, Self::byte_add_i64(prev_end, delta)?),
                    )?;
                }
                _ => break,
            }
            cur = self.parent_of(id);
        }
        Ok(())
    }

    /// 向上钉回：起点被 uniform shift 平移（旧起点 == `prev_start`，新起点
    /// == `prev_start + pull_back`）的祖先容器，把起点拉回 `prev_start`，
    /// 保持 end 不动（end 已由 shift 正确增长）。shift_range 对 a>=t 的
    /// 节点一律统移，段首插入/包裹必须在调用方把起点钉回，否则段落（乃至
    /// Document/Item 等起点相重的容器）会整体右移，产生负间隙。祖先起点
    /// 沿链非增（容器必含子节点），故首个不匹配即可安全停止。
    fn reanchor_containers_starting_at(
        &mut self,
        child: NodeId,
        prev_start: ByteOffset,
        pull_back: u32,
    ) -> Result<(), EditError> {
        let shifted = ByteOffset(prev_start.0 + pull_back);
        let mut cur = self.parent_of(child);
        while let Some(id) = cur {
            let n = self.tree.node(id);
            if n.deleted {
                break;
            }
            match n.source {
                Some(src) if src.start == shifted => {
                    let end = src.end;
                    self.set_range(id, ByteRange::new(prev_start, end))?;
                }
                _ => break,
            }
            cur = self.parent_of(id);
        }
        Ok(())
    }

    /// 叶所在"块文本容器"（段落或标题等包含文本的块）。
    fn block_range_of(&self, leaf: NodeId) -> Option<(NodeId, ByteRange)> {
        let mut cur = leaf;
        loop {
            let n = self.tree.node(cur);
            if matches!(n.kind, NodeKind::Paragraph | NodeKind::Heading { .. }) {
                return n.source.map(|r| (cur, r));
            }
            cur = self.parent_of(cur)?;
        }
    }

    /// 定位可编辑的 Text 叶（非 Raw/代码上下文）。
    fn editable_text_leaf(&self, byte: ByteOffset) -> Result<(NodeId, NodeId, usize), EditError> {
        let (leaf, parent, idx) = self
            .tree
            .leaf_at(byte)
            .ok_or(EditError::PositionOutOfBounds)?;
        let n = self.tree.node(leaf);
        if n.kind != NodeKind::Text {
            return Err(EditError::NotInsideTextLeaf);
        }
        // 祖先链（含叶自身）不得是 Raw/代码。
        let mut cur = leaf;
        loop {
            let k = self.tree.node(cur).kind.clone();
            if kind_not_editable(&k) {
                return Err(EditError::CrossesRawContent);
            }
            match self.parent_of(cur) {
                Some(p) => cur = p,
                None => break,
            }
        }
        Ok((leaf, parent, idx))
    }

    fn range_to_bytes(&self, range: &VisualRange) -> Result<(ByteOffset, ByteOffset), EditError> {
        let r = range.sorted();
        let (b, e) = (
            self.visual_to_byte(r.start)
                .ok_or(EditError::PositionOutOfBounds)?,
            self.visual_to_byte(r.end)
                .ok_or(EditError::PositionOutOfBounds)?,
        );
        Ok((b, e))
    }

    fn bytes_eq(&self, range: ByteRange, want: &str) -> bool {
        self.byte_slice(range).as_deref() == Some(want)
    }

    fn set_range(&mut self, id: NodeId, range: ByteRange) -> Result<(), EditError> {
        if range.start > range.end {
            return Err(EditError::InternalInvariant);
        }
        self.tree.node_mut(id).source = Some(range);
        Ok(())
    }

    /// 带符号字节偏移（delta 可为负），带回溢检查。
    fn byte_add_i64(bo: ByteOffset, d: i64) -> Result<ByteOffset, EditError> {
        let v = i64::from(bo.0).checked_add(d).ok_or(EditError::Overflow)?;
        if v < 0 || v > i64::from(u32::MAX) {
            return Err(EditError::Overflow);
        }
        Ok(ByteOffset(v as u32))
    }

    /// 从 `byte` 向前数行首空白宽度（' '=1、'\t'=4），到非空白/换行即停。
    /// 用于判断缩进是否会重解析成代码块（≥4）。
    fn line_leading_spaces_before(&self, byte: ByteOffset) -> u32 {
        let text = self.text().to_string();
        let bytes = text.as_bytes();
        let mut i = byte.0 as usize;
        let mut w = 0u32;
        while i > 0 {
            match bytes[i - 1] {
                b' ' => w += 1,
                b'\t' => w += 4,
                _ => break,
            }
            if w >= 4 {
                return w;
            }
            i -= 1;
        }
        w
    }

    /// 字符串开头的空白宽度：' '=1、'\t'=4，遇其它字符停止。
    fn leading_ws_width(s: &str) -> u32 {
        let mut w = 0u32;
        for ch in s.chars() {
            match ch {
                ' ' => w += 1,
                '\t' => w += 4,
                _ => break,
            }
        }
        w
    }

    fn apply_rope_insert(&mut self, byte: ByteOffset, s: &str) -> Result<(), EditError> {
        let cs = self.char_of(byte)?;
        self.text.try_insert(cs, s).map_err(|_| EditError::Overflow)
    }

    fn apply_rope_replace(
        &mut self,
        start: ByteOffset,
        end: ByteOffset,
        s: &str,
    ) -> Result<(), EditError> {
        let cs = self.char_of(start)?;
        let ce = self.char_of(end)?;
        if cs > ce {
            return Err(EditError::InternalInvariant);
        }
        self.text
            .try_remove(cs..ce)
            .map_err(|_| EditError::PositionOutOfBounds)?;
        self.text.try_insert(cs, s).map_err(|_| EditError::Overflow)
    }

    fn postop(&mut self) {
        self.tree.coalesce_all();
        self.normalize_paragraph_attribution();
        self.reconcile_paragraph_inlines();
        self.tree.recompute_gaps_all();
    }

    /// 把段落空白归属对齐 comrak 约定：前导 ≤3 空格归父级 gap
    /// （段落与首子一起右移），尾部空格归段落自身尾部 gap。
    /// 幂等：已归一化的段落首字符非空格、尾子不再覆盖尾空格。
    fn normalize_paragraph_attribution(&mut self) {
        let ids = self.tree.all_ids();
        for id in ids {
            let (kind, deleted) = {
                let n = self.tree.node(id);
                (n.kind.clone(), n.deleted)
            };
            if deleted || kind != NodeKind::Paragraph {
                continue;
            }
            let pr = match self.tree.node(id).source {
                Some(r) => r,
                None => continue,
            };
            let slice: String = match self.byte_slice(pr) {
                Some(c) => c.into_owned(),
                None => continue,
            };
            // (i) 前导空格 → 父级 gap。仅当首文本叶起点等于段首（无 margin）。
            let lead = slice.bytes().take_while(|&c| c == b' ').count();
            if (1..=3).contains(&lead)
                && let Some(&first) = self.tree.node(id).children.first()
                && let Some(fr) = self.tree.node(first).source
                && fr.start == pr.start
                && fr.end.0 >= pr.start.0 + lead as u32
            {
                let _ = self.set_range(
                    first,
                    ByteRange::new(ByteOffset(fr.start.0 + lead as u32), fr.end),
                );
                let _ = self.set_range(
                    id,
                    ByteRange::new(ByteOffset(pr.start.0 + lead as u32), pr.end),
                );
            }
            // (ii) 尾部空格 → 段落尾部 gap：收缩最后 Text 叶（若它覆盖到段尾）。
            if let Some(&last) = self.tree.node(id).children.last() {
                let (lk, lr, ld) = {
                    let n = self.tree.node(last);
                    (n.kind.clone(), n.source, n.deleted)
                };
                if !ld
                    && lk == NodeKind::Text
                    && let Some(lr) = lr
                    && lr.end == pr.end
                {
                    let lslice: String = match self.byte_slice(lr) {
                        Some(c) => c.into_owned(),
                        None => continue,
                    };
                    let trail = lslice.bytes().rev().take_while(|&c| c == b' ').count();
                    if trail > 0 {
                        let _ = self.set_range(
                            last,
                            ByteRange::new(lr.start, ByteOffset(lr.end.0 - trail as u32)),
                        );
                    }
                }
            }
        }
    }

    /// 对每个存活段落：把段内容当作独立文档重解析，若行内投影与工作树
    /// 不一致（删除清空了标记内容、或在字面分隔符之间插入了内容，使
    /// comrak 重新推导出/撤销 Emph/Strong/Code 等行内结构），则按重解析
    /// 结果重建段落行内子树。
    ///
    /// 等价不变式由构造保证：重建后的行内投影 == 重解析投影。
    /// 仅当段落含有标记类字符（`*` `` ` `` `_` `~`）时才检查，否则
    /// 段落文本对 comrak 必然是单个 Text 游程（+ SoftBreak），结构不会分歧。
    fn reconcile_paragraph_inlines(&mut self) {
        fn markish(s: &str) -> bool {
            s.bytes().any(|b| matches!(b, b'*' | b'`' | b'_' | b'~'))
        }
        fn walk_work(
            tree: &EditableTree,
            text: &str,
            id: NodeId,
            out: &mut Vec<(NodeKind, String)>,
        ) {
            let n = tree.node(id);
            let raw = n
                .source
                .map(|r| text[r.to_byte_range()].to_string())
                .unwrap_or_default();
            let bytes = if n.kind == NodeKind::Text {
                crate::serialize::unescape_text(&raw)
            } else {
                raw
            };
            out.push((n.kind.clone(), bytes));
            for &c in &n.children {
                walk_work(tree, text, c, out);
            }
        }
        fn walk_reparse(
            tree: &crate::model::LosslessTree,
            text: &str,
            id: crate::model::NodeId,
            out: &mut Vec<(NodeKind, String)>,
        ) {
            let n = tree.node(id);
            let raw = n
                .source
                .map(|r| text[r.to_byte_range()].to_string())
                .unwrap_or_default();
            let bytes = if n.kind == NodeKind::Text {
                crate::serialize::unescape_text(&raw)
            } else {
                raw
            };
            out.push((n.kind.clone(), bytes));
            for &c in &n.children {
                walk_reparse(tree, text, c, out);
            }
        }
        fn clone_subtree(
            tree: &mut EditableTree,
            src: &crate::model::LosslessTree,
            id: crate::model::NodeId,
            base: u32,
        ) -> NodeId {
            let n = src.node(id);
            let source = n
                .source
                .map(|r| ByteRange::new(ByteOffset(base + r.start.0), ByteOffset(base + r.end.0)));
            let kids: Vec<NodeId> = n
                .children
                .iter()
                .map(|&c| clone_subtree(tree, src, c, base))
                .collect();
            tree.push(n.kind.clone(), source, NodeFlags::NONE, kids)
        }

        let ids = self.tree.all_ids();
        for id in ids {
            let (kind, deleted) = {
                let n = self.tree.node(id);
                (n.kind.clone(), n.deleted)
            };
            if deleted || kind != NodeKind::Paragraph {
                continue;
            }
            let pr = match self.tree.node(id).source {
                Some(r) => r,
                None => continue,
            };
            let slice: String = match self.byte_slice(pr) {
                Some(c) => c.into_owned(),
                None => continue,
            };
            if slice.is_empty() || !markish(&slice) {
                continue;
            }
            let mut input = slice.clone();
            input.push('\n');
            let Ok(session) = crate::parse::parse(input.as_bytes()) else {
                continue;
            };
            let rebuilt = session.tree();
            let root_kids = rebuilt.node(rebuilt.root()).children.clone();
            if root_kids.len() != 1 {
                continue;
            }
            let rb = root_kids[0];
            let work_text = self.text().to_string();
            let rebuild_text = session.text().to_string();
            let mut mine = Vec::new();
            walk_work(&self.tree, &work_text, id, &mut mine);
            let mut theirs = Vec::new();
            walk_reparse(rebuilt, &rebuild_text, rb, &mut theirs);
            if mine == theirs {
                continue; // 结构已一致：免动，避免无谓的节点 ID 翻新。
            }
            // 分歧 → 重建。先把旧行内子树整体移除（含后代，防孤儿）。
            let old: Vec<NodeId> = self.tree.node(id).children.clone();
            for c in &old {
                self.tree.bury_subtree(*c);
            }
            let rkind = rebuilt.node(rb).kind.clone();
            if rkind != NodeKind::Paragraph {
                // 段落在编辑后变成块级元素（如分隔线 ThematicBreak、裸
                // "*" 重解析成的 List）：块级重解释。局部改 kind 会丢结构
                // （如 List 缺 Item），改为：墓碑化该段，把重解析根块整棵
                // 克隆到其父容器原位。投影按遍历序比较，后续被 comrak
                // 吸收进嵌套项的块在工作树中仍是同序兄弟，故逐项相等即可
                // 成立（探针证实被吸收内容的 Item source 不包含吸收内容）。
                let parent = match self.parent_of(id) {
                    Some(p) => p,
                    None => continue,
                };
                let clone = clone_subtree(&mut self.tree, rebuilt, rb, pr.start.0);
                let kids = self.tree.node_mut(parent).children.clone();
                let kids = {
                    let mut kids = kids;
                    match kids.iter().position(|&c| c == id) {
                        Some(i) => kids[i] = clone,
                        None => kids.push(clone),
                    }
                    kids
                };
                self.tree.node_mut(parent).children = kids;
                self.tree.tombstone(id);
                continue;
            }
            let kids: Vec<NodeId> = rebuilt
                .node(rb)
                .children
                .iter()
                .map(|&c| clone_subtree(&mut self.tree, rebuilt, c, pr.start.0))
                .collect();
            self.tree.node_mut(id).children = kids;
        }
    }

    /// 该节点是否位于列表/引用容器内（缩进规则不同，跳过代码块判定）。
    fn inside_list_or_quote(&self, mut id: NodeId) -> bool {
        while let Some(p) = self.parent_of(id) {
            let k = self.tree.node(p).kind.clone();
            if matches!(
                k,
                NodeKind::List { .. } | NodeKind::Item | NodeKind::BlockQuote
            ) {
                return true;
            }
            id = p;
        }
        false
    }

    fn begin_log(&mut self, _nodes: &[NodeId], _parents: &[NodeId]) -> ChangeLog {
        ChangeLog {
            text: Vec::new(),
            before_nodes: self.snapshot_all(),
            after_nodes: Vec::new(),
        }
    }

    fn snapshot_all(&self) -> Vec<NodeState> {
        (0..self.tree.len())
            .map(|i| self.node_state(NodeId(i as u32)))
            .collect()
    }

    fn log_created(&mut self, _log: &mut ChangeLog, _id: NodeId) {
        // 无需专门记录：before 快照已覆盖整个竞技场，新建节点
        // 在 before 中不存在，undo 时按“不在快照中”复位为墓碑。
    }

    fn node_state(&self, id: NodeId) -> NodeState {
        let n = self.tree.node(id);
        NodeState {
            id,
            kind: n.kind.clone(),
            source: n.source,
            flags: n.flags,
            children: n.children.clone(),
            deleted: n.deleted,
        }
    }

    fn end_log(&mut self, log: &mut ChangeLog) {
        log.after_nodes = self.snapshot_all();
    }

    #[allow(clippy::too_many_arguments)]
    fn record(
        &mut self,
        mut log: ChangeLog,
        op: SemanticOp,
        sel_before: VisualSelection,
        sel_after: VisualSelection,
    ) -> EditRecord {
        self.end_log(&mut log);
        let forward = Step {
            text: log.text.clone(),
            nodes: log.after_nodes.clone(),
            parents: Vec::new(),
        };
        let inverse = Step {
            text: log.text,
            nodes: log.before_nodes,
            parents: Vec::new(),
        };
        let rec = EditRecord {
            op,
            selection_before: sel_before,
            selection_after: sel_after,
            forward,
            inverse,
        };
        self.history.push(rec.clone());
        rec
    }

    /// 语义不变式：重解析工作文本的语义必须与工作树一致。
    pub fn assert_semantics_consistent(&self) -> Result<(), String> {
        let bytes = self.bytes();
        let session = crate::parse::parse(&bytes).map_err(|e| e.to_string())?;
        let reparsed =
            crate::serialize::tree_semantic_projection(session.tree(), &session.text().to_string());
        let mine = self.semantic_projection();
        if reparsed.len() != mine.len() {
            return Err(format!(
                "reparse projection length {} != working tree {}",
                reparsed.len(),
                mine.len()
            ));
        }
        for (i, (a, b)) in reparsed.iter().zip(mine.iter()).enumerate() {
            if a != b {
                return Err(format!(
                    "projection mismatch at {i}: reparse {:?} vs tree {:?}",
                    a, b
                ));
            }
        }
        Ok(())
    }

    /// 应用一个步骤。`forward=true` 应用 forward 步（重做），否则应用
    /// inverse 步（撤销）。文本增量方向中立。
    fn apply_step(&mut self, step: &Step, forward: bool) -> Result<(), EditError> {
        // 1）快照未覆盖的节点复位为墓碑：它们是本次操作新建的，
        //    undo 时必须从树里“消失”（ID 保留供 redo 复用）。
        let covered: std::collections::HashSet<u32> = step.nodes.iter().map(|n| n.id.0).collect();
        for i in 0..self.tree.len() {
            let id = NodeId(i as u32);
            if !covered.contains(&id.0) {
                let n = &mut self.tree.node_mut(id);
                n.source = None;
                n.children.clear();
                n.deleted = true;
            }
        }
        for d in &step.text {
            let cs = self.char_of(d.start)?;
            let (remove_to, insert): (usize, &str) = if forward {
                let ce = self.char_of(d.end)?;
                if cs > ce {
                    return Err(EditError::InternalInvariant);
                }
                (ce, d.new_bytes.as_str())
            } else {
                let n = d.new_bytes.chars().count();
                if cs + n > self.text.len_chars() {
                    return Err(EditError::PositionOutOfBounds);
                }
                (cs + n, d.old_bytes.as_str())
            };
            self.text
                .try_remove(cs..remove_to)
                .map_err(|_| EditError::PositionOutOfBounds)?;
            self.text
                .try_insert(cs, insert)
                .map_err(|_| EditError::Overflow)?;
        }
        for ns in &step.nodes {
            let n = &mut self.tree.node_mut(ns.id);
            n.kind = ns.kind.clone();
            n.source = ns.source;
            n.flags = ns.flags;
            n.children = ns.children.clone();
            n.deleted = ns.deleted;
        }
        for (pid, children) in &step.parents {
            self.tree.node_mut(*pid).children = children.clone();
        }
        Ok(())
    }
}

/// 替换 [b, e) 为 `rep`（删除/替换共用实现）。
/// 前置校验：单叶内或同一段落内跨 SoftBreak/LineBreak；
/// 不触碰 Raw；不会清空整个块（`rep` 为空时）。
/// `raw` 是转义前的用户输入（用于操作记录）。
impl EditSession {
    fn replace_span(
        &mut self,
        b: ByteOffset,
        e: ByteOffset,
        rep: &str,
        is_delete: bool,
        raw: &str,
    ) -> Result<EditRecord, EditError> {
        let (head, parent, idx) = self.editable_text_leaf(b)?;
        let hr = self.tree.node(head).source.unwrap();
        // 块容器（段落/标题）及其字节范围：多次复用，需在任何变更前捕获。
        let block = self.block_range_of(head);
        let brange = block.map(|(_, r)| r);
        // 与段落 end 重合的祖先容器必须在任何变更前捕获：shift_from 之后
        // 再判断会误伤“被平移恰好撞到 prev_end”的容器（如删除段首字符时
        // Document 从 144 缩到 143，恰等于旧段尾 143 → 被多缩 1 字节）。
        let end_cids: Vec<NodeId> = block
            .as_ref()
            .map(|(blk, pr)| self.containers_ending_at(*blk, pr.end))
            .unwrap_or_default();

        // 组装叶拓扑：head 起、跨叶时经过的 SoftBreak/LineBreak（middles）与
        // 被整体覆盖的 Text（consumed_texts）、以及尾部叶（tail，可选）。
        let mut middles: Vec<NodeId> = Vec::new();
        let mut consumed_texts: Vec<NodeId> = Vec::new();
        let mut tail: Option<(NodeId, ByteRange)> = None;
        if e > hr.end {
            // 跨叶只允许在段内直属叶之间（穿过行内标记 → 拒绝）。
            let (paragraph, _) = self
                .block_range_of(head)
                .ok_or(EditError::NotInsideTextLeaf)?;
            if parent != paragraph {
                return Err(EditError::MarkShapeMismatch);
            }
            let sibs = self.tree.node(parent).children.clone();
            let para_range = self.tree.node(paragraph).source.unwrap();
            if e > para_range.end {
                return Err(EditError::NotInsideTextLeaf);
            }
            let mut j = idx + 1;
            loop {
                let Some(&c) = sibs.get(j) else {
                    // 兄弟列表走完仍未到 e：越界。
                    return Err(EditError::NotInsideTextLeaf);
                };
                let cr = self.tree.node(c).source.unwrap();
                let k = self.tree.node(c).kind.clone();
                if cr.end <= e {
                    // 整体覆盖的叶：SoftBreak/LineBreak 进 middles，Text 进
                    // consumed_texts，其它类型（标记节点等）→ 拒绝。
                    match k {
                        NodeKind::SoftBreak | NodeKind::LineBreak => {
                            middles.push(c);
                            if cr.end == e {
                                break;
                            }
                        }
                        NodeKind::Text => {
                            consumed_texts.push(c);
                            if cr.end == e {
                                break;
                            }
                        }
                        _ => return Err(EditError::MarkShapeMismatch),
                    }
                } else if cr.start <= e {
                    // e 严格落在该叶内部：必须是 Text（尾部叶）。
                    if k != NodeKind::Text {
                        return Err(EditError::MarkShapeMismatch);
                    }
                    tail = Some((c, cr));
                    break;
                } else {
                    // e 早于该叶起点：前面的叶都不知道 e，越界。
                    return Err(EditError::NotInsideTextLeaf);
                }
                j += 1;
            }
        }

        // 计算替换后的块文本，用于两个前置校验：
        //  1) 结果块（含空白）为空 → 重解析会失去该块 → 拒绝。
        //  2) 结果块的“行首缩进”达到/超过 4 个空格 → 重解析变成缩进代码块
        //     （Raw）→ 拒绝。缩进 = 块起点前的行首空白 + 块内前导空白。
        let mut new_block_text: Option<String> = None;
        if let Some(pr) = brange {
            let slice: String = self
                .byte_slice(pr)
                .ok_or(EditError::InternalInvariant)?
                .into_owned();
            let ib = (b.0 - pr.start.0) as usize;
            let ie = (e.0 - pr.start.0) as usize;
            let mut assembled = String::with_capacity(slice.len() + rep.len() + 4);
            assembled.push_str(&slice[..ib]);
            assembled.push_str(rep);
            assembled.push_str(&slice[ie..]);
            if assembled.trim().is_empty() {
                return Err(EditError::EmptyBlockResult);
            }
            new_block_text = Some(assembled);
        }
        if let (Some((blk, pr)), Some(assembled)) = (block, new_block_text.as_deref()) {
            // 列表/引用容器内的缩进参照 marker，不构成顶层缩进代码块。
            if !self.inside_list_or_quote(blk) {
                // 行首缩进 = 块起点前的行首空白宽度 + 块内前导空白宽度。
                let outer = self.line_leading_spaces_before(pr.start);
                let inner = Self::leading_ws_width(assembled);
                if outer + inner >= 4 {
                    return Err(EditError::IndentedCodeBlock);
                }
            }
        }

        let old: String = self
            .byte_slice(ByteRange::new(b, e))
            .ok_or(EditError::InternalInvariant)?
            .into_owned();

        // 操作前选区（视觉坐标）：必须在文本变更前计算，
        // 因为删除后字节 e 可能已越界（文档变短）。
        let pre_sel = VisualSelection::new(
            self.byte_to_visual(b).ok_or(EditError::InternalInvariant)?,
            self.byte_to_visual(e).ok_or(EditError::InternalInvariant)?,
        );

        let mut affected: Vec<NodeId> = vec![head];
        affected.extend(middles.iter().copied());
        affected.extend(consumed_texts.iter().copied());
        if let Some((t, _)) = tail {
            affected.push(t);
        }
        let mut log = self.begin_log(&affected, &[parent]);

        // 文本净变更。
        self.apply_rope_replace(b, e, rep)?;
        // 树：
        let delta = rep.len() as i64 - i64::from(e.0 - b.0);
        //  1) 完全覆盖的叶 → 墓碑；从父子列表中摘除。
        for m in middles.iter().chain(consumed_texts.iter()) {
            self.tree.tombstone(*m);
        }
        self.tree.drop_children(parent, &middles);
        self.tree.drop_children(parent, &consumed_texts);
        //  2) threshold = e 的点偏移：e 之后的一切偏移 delta。删除区间内部
        //     （b..e）不允许有未处理的存活节点——head/tail 与墓碑叶已显式管理。
        self.tree.shift_from(e, delta)?;
        //  3) 归属叶手工重锚。
        match tail {
            Some((tail_id, tr)) => {
                if b > hr.start {
                    // head 存活：持有 [hr.start, b) + rep；tail 自 b+len(rep) 续。
                    let rep_end = Self::byte_add_i64(b, rep.len() as i64)?;
                    self.set_range(head, ByteRange::new(hr.start, rep_end))?;
                    self.set_range(
                        tail_id,
                        ByteRange::new(rep_end, Self::byte_add_i64(tr.end, delta)?),
                    )?;
                } else {
                    // head 完全覆盖：rep 前缀至 tail。
                    self.tree.tombstone(head);
                    self.tree.drop_children(parent, &[head]);
                    self.set_range(
                        tail_id,
                        ByteRange::new(b, Self::byte_add_i64(tr.end, delta)?),
                    )?;
                }
            }
            None => {
                if b > hr.start && e < hr.end {
                    // 中部替换：叶 = 前缀 + rep + 后缀，收缩为单区间 [ls, le+delta)。
                    self.set_range(
                        head,
                        ByteRange::new(hr.start, Self::byte_add_i64(hr.end, delta)?),
                    )?;
                } else if b > hr.start {
                    // 后缀替换：叶 = [hr.start, b) + rep。
                    self.set_range(
                        head,
                        ByteRange::new(hr.start, Self::byte_add_i64(b, rep.len() as i64)?),
                    )?;
                } else if e < hr.end {
                    // 前缀替换：b == hr.start，叶 = rep + 后缀，整体为
                    // [hr.start, hr.end + delta)，rep 保留在叶内而非掉进 gap。
                    self.set_range(
                        head,
                        ByteRange::new(hr.start, Self::byte_add_i64(hr.end, delta)?),
                    )?;
                } else if rep.is_empty() {
                    // 整叶删除且无 tail：叶消失（整块清空由 EmptyBlockResult 拦截）。
                    self.tree.tombstone(head);
                    self.tree.drop_children(parent, &[head]);
                } else {
                    // 整叶替换：叶 = [b, b + len(rep))。
                    self.set_range(
                        head,
                        ByteRange::new(b, Self::byte_add_i64(b, rep.len() as i64)?),
                    )?;
                }
            }
        }
        //  4) 块容器（段落）的再锚定：
        //     - e == 块末尾：块随 delta 伸缩（否则叶越过块末端 → 缺口反转）；
        //     - b == 块起点（纯段首插入/段首替换）：uniform shift 把块整体右移，
        //       而叶被吸回段首 → 必须把块也钉回 [pr.start, pr.end+delta)。
        //     中部替换由 shift_from 自动处理（end += delta）。
        if let Some((blk, pr)) = block {
            if e == pr.end && !pr.is_empty() {
                // 段末尾：段落随 delta 伸缩；变更前捕获的、end==旧段尾的
                // 祖先容器同步伸缩（它们因 shift_range 的 end==t 保持规则而未动）。
                self.set_range(
                    blk,
                    ByteRange::new(pr.start, Self::byte_add_i64(pr.end, delta)?),
                )?;
                for cid in &end_cids {
                    let csrc = self.tree.node(*cid).source.unwrap();
                    self.set_range(
                        *cid,
                        ByteRange::new(csrc.start, Self::byte_add_i64(pr.end, delta)?),
                    )?;
                }
            } else if b == pr.start {
                // 段首（纯段首插入/段首替换）：uniform shift 把块整体右移，
                // 需钉回 [pr.start, pr.end+delta)。祖先容器只靠 shift 平移，
                // 不再额外伸缩（end_cids 在段首删除场景会误伤平移者，已弃用）。
                self.set_range(
                    blk,
                    ByteRange::new(pr.start, Self::byte_add_i64(pr.end, delta)?),
                )?;
                // 但纯段首插入（b == e == pr.start）时阈值=b，shift_from 把起点
                // 与 pr.start 重合的祖先容器（文档首段时的 Document、列表首段的
                // Item）整体右移 —— 必须把它们的起点一并拉回（终点已增长）。
                if delta > 0 {
                    self.reanchor_containers_starting_at(blk, pr.start, delta as u32)?;
                }
            }
        }
        log.text.push(TextDelta {
            start: b,
            end: e,
            old_bytes: old,
            new_bytes: rep.to_string(),
        });
        self.postop();

        // 光标落在 rep 之后；选区记录已用操作前坐标（pre_sel）。
        let after_v = self
            .byte_to_visual(ByteOffset(b.0 + rep.len() as u32))
            .ok_or(EditError::InternalInvariant)?;
        let after = VisualSelection::collapsed(after_v);
        let op = if is_delete {
            SemanticOp::DeleteText {
                range: VisualRange {
                    start: pre_sel.anchor,
                    end: pre_sel.head,
                },
            }
        } else {
            SemanticOp::ReplaceSelection {
                range: VisualRange {
                    start: pre_sel.anchor,
                    end: pre_sel.head,
                },
                text: raw.to_string(),
            }
        };
        Ok(self.record(log, op, pre_sel, after))
    }
}

// ---- 小工具 ----

fn block_body(source: &str, kind: &NodeKind) -> String {
    match kind {
        NodeKind::Heading { setext: true, .. } => {
            source.lines().next().unwrap_or_default().trim().to_string()
        }
        NodeKind::Heading { .. } => {
            let line = source.lines().next().unwrap_or_default().trim_start();
            let body = line.trim_start_matches('#').trim_start();
            body.trim_end_matches('#').trim_end().to_string()
        }
        NodeKind::BlockQuote => source
            .lines()
            .map(|line| {
                let line = line.trim_start();
                line.strip_prefix('>')
                    .map(|rest| rest.strip_prefix(' ').unwrap_or(rest))
                    .unwrap_or(line)
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => source.to_string(),
    }
}

fn longest_run(text: &str, needle: char) -> usize {
    text.split(|ch| ch != needle)
        .map(str::len)
        .max()
        .unwrap_or(0)
}

fn text_delta_bounds(old: &str, new: &str) -> (usize, usize, usize) {
    let common = old.len().min(new.len());
    let mut prefix = 0usize;
    while prefix < common && old.as_bytes()[prefix] == new.as_bytes()[prefix] {
        prefix += 1;
    }
    while prefix > 0 && (!old.is_char_boundary(prefix) || !new.is_char_boundary(prefix)) {
        prefix -= 1;
    }
    let mut suffix = 0usize;
    let max_suffix = old
        .len()
        .saturating_sub(prefix)
        .min(new.len().saturating_sub(prefix));
    while suffix < max_suffix
        && old.as_bytes()[old.len() - 1 - suffix] == new.as_bytes()[new.len() - 1 - suffix]
    {
        suffix += 1;
    }
    while suffix > 0
        && (!old.is_char_boundary(old.len() - suffix) || !new.is_char_boundary(new.len() - suffix))
    {
        suffix -= 1;
    }
    (prefix, old.len() - suffix, new.len() - suffix)
}

fn rewrite_table_lines(source: &str, mut rewrite: impl FnMut(&str, usize) -> String) -> String {
    let mut result = String::with_capacity(source.len());
    for (index, segment) in source.split_inclusive('\n').enumerate() {
        let (line, ending) = match segment.strip_suffix('\n') {
            Some(without_lf) => without_lf
                .strip_suffix('\r')
                .map_or((without_lf, "\n"), |line| (line, "\r\n")),
            None => (segment, ""),
        };
        result.push_str(&rewrite(line, index));
        result.push_str(ending);
    }
    if !source.ends_with('\n') && !source.is_empty() && result.is_empty() {
        result.push_str(&rewrite(source, 0));
    }
    result
}

fn table_line_insert_column(line: &str, column: usize, separator: bool) -> String {
    let mut value = line.trim_end_matches('\r');
    let has_end_pipe = value.trim_end().ends_with('|');
    if has_end_pipe {
        value = value.trim_end();
        value = &value[..value.len().saturating_sub(1)];
    }
    let cell = if separator { " --- " } else { "   " };
    let mut parts = value.split('|').map(str::to_string).collect::<Vec<_>>();
    let position = (column + 1).min(parts.len());
    parts.insert(position, cell.to_string());
    let mut result = parts.join("|");
    if has_end_pipe {
        result.push('|');
    }
    result
}

fn table_line_delete_column(line: &str, column: usize) -> String {
    let value = line.trim_end_matches('\r');
    let has_end_pipe = value.trim_end().ends_with('|');
    let mut body = value.trim_end().to_string();
    if has_end_pipe {
        body.pop();
    }
    let mut parts = body.split('|').map(str::to_string).collect::<Vec<_>>();
    let position = column + usize::from(parts.first().is_some_and(|part| part.trim().is_empty()));
    if position < parts.len() {
        parts.remove(position);
    }
    let mut result = parts.join("|");
    if has_end_pipe {
        result.push('|');
    }
    result
}

fn byte_checked_add(bo: ByteOffset, n: u32) -> Result<ByteOffset, EditError> {
    let v = bo.0.checked_add(n).ok_or(EditError::Overflow)?;
    Ok(ByteOffset(v))
}

fn grapheme_count(s: &str) -> u32 {
    u32::try_from(s.graphemes(true).count()).unwrap_or(u32::MAX)
}
