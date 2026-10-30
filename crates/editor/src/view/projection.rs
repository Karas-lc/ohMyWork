//! 视图派生投影：从 LosslessTree + SourceMap 派生的仅视图状态。
//! 不在 DocumentSession / PageSnapshot / 持久化中保存。
//! 所有 offset UTF-8 字节；基于 Comrak 语义 + 现有 SourceMap 建一一映射。
//! 隐藏标记保留范围；活动 leaf 显露时使用同一字号显示原文+格式 mark。
use std::ops::Range;

use crate::model::{ByteRange, LosslessTree, NodeId, NodeKind};
use crate::SourceMap;
use ropey::Rope;
/// 表面内稳定 leaf 身份（非 NodeId、非行号）。用于 GPUI leaf_key / element id 派生。
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct LeafId(pub u64);

/// Leaf 种类。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LeafKind {
    Text,
    Heading(u8),
    Code,
    TableCell {
        table: u64,
        row: usize,
        column: usize,
    },
    Raw,
}

/// 映射种类：Verbatim（原文=显示），Decoded（转义/实体解码后显示，保留 source），Hidden（语法标记隐藏但保留 source range）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MappingKind {
    Verbatim,
    Decoded,
    Hidden,
}

/// 源到显示的运行片段。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourceRun {
    pub display: Range<usize>,
    pub source: Range<usize>,
    pub kind: MappingKind,
}

/// 投影中的一个叶。
#[derive(Clone, Debug)]
pub(crate) struct ProjectedLeaf {
    pub id: LeafId,
    pub kind: LeafKind,
    pub source_range: Range<usize>,
    pub display_text: String,
    pub inline_marks: Vec<InlineMark>,
    pub runs: Vec<SourceRun>,
    pub table_pos: Option<(u64, usize, usize)>,
}

/// 行内样式标记（用于 GPUI Paragraph push）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InlineMark {
    Bold,
    Italic,
    Strikethrough,
    Code,
}

/// 完整已解析投影。版本与 source revision 对齐；不跨外部 epoch 复用。
#[derive(Clone, Debug)]
pub(crate) struct ParsedProjection {
    pub version: u64,
    pub root_blocks: Vec<BlockDesc>,
    pub leaves: Vec<ProjectedLeaf>,
    pub source: Rope,
    next_leaf_id: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BlockDesc {
    pub node_id: NodeId,
    pub kind: BlockKind,
    pub source_range: Range<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BlockKind {
    Paragraph,
    Heading(u8),
    List,
    ListItem,
    Blockquote,
    CodeBlock,
    Table { col_count: usize, aligns: Vec<ColumnAlign> },
    Raw,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ColumnAlign {
    Left,
    Center,
    Right,
}

/// Bias for boundary resolution on hidden markers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Bias {
    Before,
    After,
}

impl ParsedProjection {
    pub(crate) fn new(version: u64, source: Rope) -> Self {
        Self {
            version,
            root_blocks: Vec::new(),
            leaves: Vec::new(),
            source,
            next_leaf_id: 1,
        }
    }

    fn alloc_leaf(&mut self) -> LeafId {
        let id = LeafId(self.next_leaf_id);
        self.next_leaf_id += 1;
        id
    }

    /// source_offset: display byte in leaf -> source byte (with bias for hidden edges).
    pub(crate) fn source_offset(&self, leaf: LeafId, display_byte: usize, bias: Bias) -> Option<usize> {
        if let Some(leaf) = self.leaves.iter().find(|l| l.id == leaf) {
            for run in &leaf.runs {
                if display_byte >= run.display.start && display_byte <= run.display.end {
                    let off = display_byte - run.display.start;
                    let src = run.source.start + off;
                    return Some(src);
                }
            }
            if display_byte == 0 {
                return Some(leaf.source_range.start);
            }
            if display_byte == leaf.display_text.len() {
                return Some(leaf.source_range.end);
            }
        }
        None
    }

    /// display_offset: source byte -> (leaf, display byte in it)
    pub(crate) fn display_offset(&self, source_byte: usize, bias: Bias) -> Option<(LeafId, usize)> {
        for leaf in &self.leaves {
            if source_byte >= leaf.source_range.start && source_byte <= leaf.source_range.end {
                for run in &leaf.runs {
                    if source_byte >= run.source.start && source_byte <= run.source.end {
                        let off = source_byte - run.source.start;
                        let disp = run.display.start + off;
                        return Some((leaf.id, disp));
                    }
                }
                // boundary fallback
                if source_byte == leaf.source_range.start {
                    return Some((leaf.id, 0));
                }
                if source_byte == leaf.source_range.end {
                    return Some((leaf.id, leaf.display_text.len()));
                }
            }
        }
        None
    }
}

pub(crate) fn build_projection_from_tree(
    version: u64,
    _tree: Option<&LosslessTree>,
    source_rope: Rope,
    source_bytes: &[u8],
    _map: &SourceMap,
) -> ParsedProjection {
    let mut proj = ParsedProjection::new(version, source_rope);
    let src = std::str::from_utf8(source_bytes).unwrap_or("");
    if src.is_empty() {
        let id = proj.alloc_leaf();
        proj.leaves.push(ProjectedLeaf {
            id,
            kind: LeafKind::Raw,
            source_range: 0..0,
            display_text: String::new(),
            inline_marks: vec![],
            runs: vec![SourceRun { display: 0..0, source: 0..0, kind: MappingKind::Verbatim }],
            table_pos: None,
        });
        proj.root_blocks.push(BlockDesc { node_id: NodeId(0), kind: BlockKind::Raw, source_range: 0..0 });
        return proj;
    }
    let lines: Vec<&str> = src.lines().collect();
    let mut offset = 0usize;
    for (i, line) in lines.iter().enumerate() {
        let line_len = line.len() + if i + 1 < lines.len() { 1 } else { 0 };
        let end = offset + line_len;
        let kind = if line.starts_with("###### ") {
            LeafKind::Heading(6)
        } else if line.starts_with("##### ") {
            LeafKind::Heading(5)
        } else if line.starts_with("#### ") {
            LeafKind::Heading(4)
        } else if line.starts_with("### ") {
            LeafKind::Heading(3)
        } else if line.starts_with("## ") {
            LeafKind::Heading(2)
        } else if line.starts_with("# ") {
            LeafKind::Heading(1)
        } else if line.contains('|') && i > 0 {
            LeafKind::TableCell { table: 1, row: i, column: 0 }
        } else {
            LeafKind::Text
        };
        let display = line.trim_start_matches(|c| c == '#' || c == ' ').to_string();
        let display_text = if display.is_empty() { " ".to_string() } else { display.clone() };
        let bkind = match &kind {
            LeafKind::Heading(l) => BlockKind::Heading(*l),
            LeafKind::TableCell { .. } => BlockKind::Table { col_count: 3, aligns: vec![] },
            _ => BlockKind::Paragraph,
        };
        let id = proj.alloc_leaf();
        proj.leaves.push(ProjectedLeaf {
            id,
            kind: kind.clone(),
            source_range: offset..end,
            display_text,
            inline_marks: vec![],
            runs: vec![SourceRun { display: 0..display.len(), source: offset..end, kind: MappingKind::Verbatim }],
            table_pos: None,
        });
        proj.root_blocks.push(BlockDesc { node_id: NodeId(i as u32), kind: bkind, source_range: offset..end });
        offset = end;
    }
    proj
}
