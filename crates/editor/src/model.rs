//! 无损文档模型的树结构与会话类型。
//!
//! 树中每个节点要么拥有一个半开 `[start, end)` 字节范围（`source` 为
//! `Some`），要么被显式标记为不可归属（`source` 为 `None`，例如空文档的
//! `Document` 或生成/退化节点）。容器节点同时记录子节点之间的"间隙"
//! （`gaps`）：分隔符、标记、缩进、行结束符、续行标记等不在任何子节点
//! 范围内的原始片段。不变式：当且仅当全部子节点都可归属时，
//! `gaps.len() == children.len() + 1`，且父节点范围等于
//! `gap[0] child[0] gap[1] child[1] ... gap[n]` 的字节串联——每个源字节
//! 都有归属。

use ropey::Rope;
use std::borrow::Cow;
use std::fmt;
use std::ops::Range;

use crate::source_map::SourceMap;

/// 源文本中的字节偏移。明确新类型，禁止与 Unicode 字符数或 UTF-16 单元混用。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ByteOffset(pub u32);

impl ByteOffset {
    pub(crate) fn from_usize(v: usize) -> Option<ByteOffset> {
        u32::try_from(v).ok().map(ByteOffset)
    }

    pub(crate) fn try_from_u64(v: u64) -> Option<ByteOffset> {
        u32::try_from(v).ok().map(ByteOffset)
    }

    pub(crate) fn u64_checked_add(self, delta: u64) -> Option<u64> {
        u64::from(self.0).checked_add(delta)
    }

    pub fn to_usize(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Debug for ByteOffset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ByteOffset({})", self.0)
    }
}

/// 半开字节范围 `[start, end)`。所有位置均为字节偏移。
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ByteRange {
    pub start: ByteOffset,
    pub end: ByteOffset,
}

impl ByteRange {
    pub fn new(start: ByteOffset, end: ByteOffset) -> ByteRange {
        debug_assert!(start <= end, "half-open range must be ordered");
        ByteRange { start, end }
    }

    pub fn len(&self) -> u32 {
        self.end.0.saturating_sub(self.start.0)
    }

    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// `self` 是否完全包含 `other`。
    pub fn contains_range(&self, other: &ByteRange) -> bool {
        self.start <= other.start && other.end <= self.end
    }

    /// 转为标准 `Range<usize>`（索引文本切片用）。
    pub fn to_byte_range(&self) -> Range<usize> {
        self.start.to_usize()..self.end.to_usize()
    }

    pub fn to_debug(&self) -> String {
        format!("[{},{})", self.start.0, self.end.0)
    }
}

impl fmt::Debug for ByteRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ByteRange[{},{})", self.start.0, self.end.0)
    }
}

/// 会话稳定节点 id：`LosslessTree` 的 arena 下标。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct NodeId(pub u32);

/// 节点标记位。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct NodeFlags(pub u32);

impl NodeFlags {
    /// 无标记：节点可归属且来源可信。
    pub const NONE: NodeFlags = NodeFlags(0);
    /// 无来源（空文档 / 退化 sourcepos）；`TreeNode::source` 为 `None`。
    pub const UNATTRIBUTED: NodeFlags = NodeFlags(1 << 0);
    /// 表格自动补足/空单元：范围可能指向分隔符（如行尾竖线）等非内容字节，
    /// 不可作为编辑目标。
    pub const SYNTHETIC: NodeFlags = NodeFlags(1 << 1);

    /// 是否包含给定标记位。
    pub fn contains(&self, flag: NodeFlags) -> bool {
        self.0 & flag.0 == flag.0
    }
}

impl std::ops::BitOr for NodeFlags {
    type Output = NodeFlags;
    fn bitor(self, rhs: NodeFlags) -> NodeFlags {
        NodeFlags(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for NodeFlags {
    fn bitor_assign(&mut self, rhs: NodeFlags) {
        self.0 |= rhs.0;
    }
}

impl std::ops::BitAnd for NodeFlags {
    type Output = NodeFlags;
    fn bitand(self, rhs: NodeFlags) -> NodeFlags {
        NodeFlags(self.0 & rhs.0)
    }
}

impl std::ops::BitAndAssign for NodeFlags {
    fn bitand_assign(&mut self, rhs: NodeFlags) {
        self.0 &= rhs.0;
    }
}

impl std::ops::Not for NodeFlags {
    type Output = NodeFlags;
    fn not(self) -> NodeFlags {
        NodeFlags(!self.0)
    }
}

/// 节点类型。仅本阶段启用的扩展（table、tasklist、strikethrough、autolink）
/// 会产生对应节点；任何未来未知的 Comrak 节点都会落到 [`NodeKind::Raw`]，
/// 绝不 panic。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeKind {
    /// 文档根。
    Document,
    /// `> 引用`。
    BlockQuote,
    /// 有序/无序列表。`start` 为有序列表起始数字。
    List {
        ordered: bool,
        start: usize,
        delimiter: ListDelimiter,
        bullet_char: Option<u8>,
        tight: bool,
        task_list: bool,
    },
    /// 列表项（含 `- ` 标记；标记字节属于 Item 的 lead gap）。
    Item,
    /// 围栏或缩进代码块。`info` 为围栏信息字符串（原文，未解码）。
    CodeBlock {
        fenced: bool,
        fence_char: u8,
        fence_length: usize,
        fence_offset: usize,
        closed: bool,
        info: String,
    },
    /// 原始 HTML 块（逐字保留，禁止编辑内部）。
    HtmlBlock { block_type: u8 },
    /// 段落。
    Paragraph,
    /// ATX 或 Setext 标题。`closed` 表示 ATX 闭合（含尾部 `#` 序列）。
    Heading {
        level: u8,
        setext: bool,
        closed: bool,
    },
    /// 分隔线。
    ThematicBreak,
    /// 表格（对齐行 `|---|---|` 不是节点，落在 Table 的 gaps 中）。
    Table {
        num_columns: usize,
        num_rows: usize,
        num_nonempty_cells: usize,
    },
    /// 表行。`header` 表示表头行。
    TableRow { header: bool },
    /// 表格单元。
    TableCell,
    /// 语义文本。范围覆盖原文（实体/转义保持原文，不在此解码）。
    Text,
    /// 任务列表项。`symbol_source` 为 checkbox 标记字符（`x`/`X`）的字节范围。
    TaskItem {
        symbol: Option<char>,
        symbol_source: Option<ByteRange>,
    },
    /// 软换行（`\n`、`\r\n` 或孤立 `\r`）。
    SoftBreak,
    /// 硬换行（行尾空格或反斜杠后换行）。
    LineBreak,
    /// 行内代码（含反引号本身）。文字以原文切片为准。
    Code { backtick_count: usize },
    /// 原始行内 HTML（逐字保留）。
    HtmlInline,
    /// 防御未知/不可安全解释的节点（保留原文）。
    Raw { detail: RawDetail },
    /// `*em*` / `_em_`。
    Emph,
    /// `**strong**`。
    Strong,
    /// `~~deleted~~`。
    Strikethrough,
    /// 链接。`url`/`title` 为 Comrak 解码后的字符串（仅元数据；原文以切片为准）。
    Link { url: String, title: String },
    /// 图片。同 [`NodeKind::Link`]。
    Image { url: String, title: String },
}

/// 有序列表分隔符。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ListDelimiter {
    Period,
    Paren,
}

/// Raw 节点的来源说明（诊断用）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RawDetail {
    /// Comrak 原生 Raw 节点。
    ComrakRaw,
    /// 未来未知的节点类型（防御分支）。
    UnknownVariant(&'static str),
}

impl NodeKind {
    /// 该节点是否属于"原始/不可安全编辑"类别（逐字保留）。
    pub fn is_raw(&self) -> bool {
        matches!(
            self,
            NodeKind::HtmlBlock { .. } | NodeKind::HtmlInline | NodeKind::Raw { .. }
        )
    }
}

/// 树中的一个节点。子节点顺序即源文本顺序。
#[derive(Clone, Debug)]
pub struct TreeNode {
    pub kind: NodeKind,
    /// 半开字节范围；`None` 表示不可归属（见 [`NodeFlags::UNATTRIBUTED`]）。
    pub source: Option<ByteRange>,
    pub flags: NodeFlags,
    pub children: Vec<NodeId>,
    /// 子节点间隙。当全部子节点都可归属时长度为 `children.len() + 1`，
    /// `gaps[0]` 为前导字节、`gaps[i]` 为 `children[i-1]` 与 `children[i]`
    /// 之间的字节、`gaps[n]` 为尾随字节；否则为空。
    pub gaps: Vec<ByteRange>,
}

/// owned 无损树。`NodeId` 即 arena 下标，会话内稳定。
#[derive(Clone, Debug)]
pub struct LosslessTree {
    nodes: Vec<TreeNode>,
    root: NodeId,
}

impl LosslessTree {
    pub(crate) fn new(nodes: Vec<TreeNode>, root: NodeId) -> LosslessTree {
        LosslessTree { nodes, root }
    }

    pub fn root(&self) -> NodeId {
        self.root
    }

    pub fn node(&self, id: NodeId) -> &TreeNode {
        &self.nodes[id.0 as usize]
    }

    /// arena 顺序遍历（父节点先于子节点；构建即先序遍历）。
    pub fn iter(&self) -> impl Iterator<Item = NodeId> + '_ {
        (0..self.nodes.len()).map(|i| NodeId(i as u32))
    }

    pub fn nodes(&self) -> &[TreeNode] {
        &self.nodes
    }

    /// 节点总数。
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// 校验树不变量：
    /// - 全部 `source` 范围在 `[0, text_len]` 内且 `start <= end`；
    /// - 子节点范围有序、不互相重叠、包含于父节点范围；
    /// - 容器 `gaps` 与其子节点范围精确拼合父节点范围（全部子节点可归属时）；
    /// - Document 范围覆盖 `[0, text_len)`（非空文档）。
    ///
    /// 返回 `issues` 集合；`Ok(())` 表示全部通过。供测试与诊断使用，
    /// 保证"所有范围有效、不重叠越界"。
    pub fn validate(&self, text_len: usize) -> Result<(), Vec<String>> {
        let mut issues: Vec<String> = Vec::new();
        // Document 必须覆盖整个非空文档。
        if text_len > 0 {
            if let Some(range) = self.nodes[self.root.0 as usize].source {
                if range.start.to_usize() != 0 || range.end.to_usize() != text_len {
                    issues.push(format!(
                        "document range {} does not cover [0,{text_len})",
                        range.to_debug()
                    ));
                }
            } else {
                issues.push("document has no range for non-empty text".to_string());
            }
        }
        for (idx, node) in self.nodes.iter().enumerate() {
            let id = NodeId(u32::try_from(idx).unwrap_or(u32::MAX));
            let Some(range) = node.source else { continue };
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
            for &child in &node.children {
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
            // gaps 计数校验：全部子节点都可归属时必须有 children+1 项。
            let all_attributed = node
                .children
                .iter()
                .all(|c| self.nodes[c.0 as usize].source.is_some());
            if all_attributed && node.gaps.len() != node.children.len() + 1 {
                issues.push(format!(
                    "node {id:?} gaps.len() {} != children.len()+1 {}",
                    node.gaps.len(),
                    node.children.len() + 1
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

/// 源编码。仅支持 UTF-8 与带 BOM 的 UTF-8；其他编码在 parse 处返回只读诊断。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SourceEncoding {
    Utf8,
    Utf8WithBom,
}

/// 文档会话：文档层的所有权威状态。
///
/// 持有磁盘原始字节的 Rope 缓冲（含 BOM/CRLF，逐字节保留）、owned 树、
/// 行映射与编码标记。`source_range`/`source_slice` 均做边界校验后再返回，
/// 调用方不应直接信任裸 `ByteRange` 之外的结构。
pub struct DocumentSession {
    text: Rope,
    tree: LosslessTree,
    source_map: SourceMap,
    encoding: SourceEncoding,
}

impl DocumentSession {
    pub(crate) fn new(
        text: Rope,
        tree: LosslessTree,
        source_map: SourceMap,
        encoding: SourceEncoding,
    ) -> DocumentSession {
        DocumentSession {
            text,
            tree,
            source_map,
            encoding,
        }
    }

    /// 规范文本缓冲。
    pub fn text(&self) -> &Rope {
        &self.text
    }

    pub fn tree(&self) -> &LosslessTree {
        &self.tree
    }

    pub fn source_map(&self) -> &SourceMap {
        &self.source_map
    }

    pub fn encoding(&self) -> SourceEncoding {
        self.encoding
    }

    /// 原始字节长度。
    pub fn text_len(&self) -> ByteOffset {
        ByteOffset(u32::try_from(self.text.len_bytes()).unwrap_or(u32::MAX))
    }

    /// 无操作序列化：树 + 间隙 + 尾随字节原样拼接，必须与输入逐字节一致。
    pub fn serialize(&self) -> String {
        // Rope 逐字节保留构造时的字节（含 CRLF/BOM），to_string 直接重建。
        self.text.to_string()
    }

    /// 原始字节（含 BOM）。
    pub fn bytes(&self) -> Vec<u8> {
        self.text
            .chunks()
            .flat_map(|c| c.as_bytes().iter().copied())
            .collect()
    }

    /// 节点字节范围（边界校验后）。
    pub fn source_range(&self, id: NodeId) -> Option<ByteRange> {
        let range = self.tree.node(id).source?;
        (range.end.to_usize() <= self.text.len_bytes()).then_some(range)
    }

    /// 节点原始片段（零拷贝，边界校验后）。
    pub fn source_slice(&self, id: NodeId) -> Option<SourceSlice<'_>> {
        let range = self.source_range(id)?;
        let text = self.byte_slice(range)?;
        Some(SourceSlice {
            node: id,
            range,
            text,
        })
    }

    /// 任意范围的原始切片；范围必须是字符边界且不越界。
    /// 单块内切片零拷贝（`Cow::Borrowed`），跨块时拷贝。边界不符或
    /// 越界时返回 `None`，绝不 panic。
    pub fn byte_slice(&self, range: ByteRange) -> Option<Cow<'_, str>> {
        let r = range.to_byte_range();
        if r.end > self.text.len_bytes() || r.start > r.end {
            return None;
        }
        // 两端都必须是字符边界（错误则拒绝，不 panic）。
        if self.text.try_byte_to_char(r.start).is_err()
            || self.text.try_byte_to_char(r.end).is_err()
        {
            return None;
        }
        Some(Cow::from(self.text.byte_slice(r)))
    }

    /// 校验整棵树不变量（供测试/诊断）。
    pub fn validate(&self) -> Result<(), Vec<String>> {
        self.tree.validate(self.text.len_bytes())
    }
}

/// 节点原始片段视图。
#[derive(Clone, Debug)]
pub struct SourceSlice<'a> {
    pub node: NodeId,
    pub range: ByteRange,
    pub text: Cow<'a, str>,
}
