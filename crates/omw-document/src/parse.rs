//! 解析入口：编码门禁 + Comrak AST 到 owned 树的转换。
//!
//! 转换规则（经委托实现探查验证）：
//! - 1-based `sourcepos` 的 start 列与 end 列都按 UTF-8 字节计数。
//!   end 列是**包含**最后一个字符的列：半开终点 = `line_start[end_line-1] + end_col`，
//!   绝不机械地"统一加 1"。
//! - `end_col == 0`（EOF 附近）时按最后一个直接子节点已修复的终点归属，
//!   保留该子容器尾部空白；不能只取最深 Text 的终点而截断父容器。
//!   无子节点时回退到本节点起点。Document 恒覆盖 `[0, len)`。
//! - 退化 sourcepos（`start.line == 0`）→ `None`（不可归属）。
//! - 无子节点的表格单元（空单元或自动补足单元，范围可能指向分隔符如行尾
//!   竖线）→ `SYNTHETIC` 标记，不做编辑目标。
//! - 任何未知节点类型 → [`NodeKind::Raw`]（防御，绝不 panic）。
//!
//! Comrak 的 `Arena` 在函数返回前释放；本模块不把任何借用暴露到树外。

use comrak::nodes::{Ast, AstNode, ListDelimType, ListType, NodeList, NodeValue, Sourcepos};
use comrak::{Arena, Options, parse_document};
use ropey::Rope;

use crate::model::{
    ByteOffset, ByteRange, DocumentSession, ListDelimiter, LosslessTree, NodeFlags, NodeId,
    NodeKind, RawDetail, SourceEncoding, TreeNode,
};
use crate::source_map::SourceMap;

/// 解析前门禁：编码诊断。只读，不伪装成功。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    /// 检出 UTF-16/UTF-32 BOM：当前不支持，需调用方按原编码解码后重试。
    UnsupportedEncoding { detected: &'static str },
    /// 不是合法 UTF-8；`position` 是第一个失败字节的偏移。
    InvalidUtf8 { position: usize },
    /// 超过 u32 字节上限，无法用 ByteOffset 表达。
    DocumentTooLarge { size: usize },
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::UnsupportedEncoding { detected } => write!(
                f,
                "unsupported encoding (detected {detected}); decode externally to UTF-8 and retry"
            ),
            ParseError::InvalidUtf8 { position } => {
                write!(f, "input is not valid UTF-8 at byte offset {position}")
            }
            ParseError::DocumentTooLarge { size } => {
                write!(f, "document too large ({size} bytes > u32 max)")
            }
        }
    }
}

impl std::error::Error for ParseError {}

/// 解析 Markdown 源为 [`DocumentSession`]。成功后无编辑时的
/// `serialize()` 与输入 bytes 逐字节一致（含 BOM/CRLF）。
pub fn parse(bytes: &[u8]) -> Result<DocumentSession, ParseError> {
    let encoding = detect_encoding(bytes)?;
    let text_str = std::str::from_utf8(bytes).map_err(|e| ParseError::InvalidUtf8 {
        position: e.valid_up_to(),
    })?;
    let source_map = SourceMap::new(bytes);
    let Some(text_len) = ByteOffset::from_usize(bytes.len()) else {
        return Err(ParseError::DocumentTooLarge { size: bytes.len() });
    };

    let arena: Arena = Arena::new();
    let root = parse_document(&arena, text_str, &options());

    let mut ctx = ConvertCtx {
        map: &source_map,
        text: text_str,
        nodes: Vec::new(),
        text_len,
    };
    let root_ast = root.data();
    let root_id = ctx.convert(&root_ast, root);
    drop(root_ast);
    ctx.canonicalize_thematic_breaks();
    // Document 恒覆盖 [0, len)（非空文档）：尾随空行/BOM 等字节由 Document
    // 的尾随/前导 gap 归属。
    let (root_range, root_children) = {
        let n = &mut ctx.nodes[root_id.0 as usize];
        if text_len.0 > 0 {
            n.source = Some(ByteRange::new(ByteOffset(0), text_len));
            n.flags &= !NodeFlags::UNATTRIBUTED;
        }
        (n.source, n.children.clone())
    };
    let gaps = ConvertCtx::gaps_for(root_range, &root_children, &ctx.nodes);
    ctx.nodes[root_id.0 as usize].gaps = gaps;
    let tree = LosslessTree::new(ctx.nodes, root_id);

    // arena 借用于 convert 结束时已结束；在此自然析构即可。
    let session = DocumentSession::new(Rope::from_str(text_str), tree, source_map, encoding);
    debug_assert_eq!(session.bytes(), bytes, "roundtrip must be byte-exact");
    Ok(session)
}

fn detect_encoding(bytes: &[u8]) -> Result<SourceEncoding, ParseError> {
    if bytes.starts_with(&[0xFF, 0xFE])
        || bytes.starts_with(&[0xFE, 0xFF])
        || bytes.starts_with(&[0x00, 0x00, 0xFE, 0xFF])
        || bytes.starts_with(&[0xFF, 0xFE, 0x00, 0x00])
    {
        return Err(ParseError::UnsupportedEncoding {
            detected: "UTF-16/UTF-32 BOM",
        });
    }
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        Ok(SourceEncoding::Utf8WithBom)
    } else {
        Ok(SourceEncoding::Utf8)
    }
}

fn options() -> Options<'static> {
    let mut opts = Options::default();
    opts.extension.table = true;
    opts.extension.tasklist = true;
    opts.extension.strikethrough = true;
    opts.extension.autolink = true;
    // `sourcepos_chars` 保持默认 false：列按 UTF-8 字节计数。
    opts.parse.sourcepos_chars = false;
    opts
}

// ---------------------------------------------------------------------------
// 转换
// ---------------------------------------------------------------------------

struct ConvertCtx<'a> {
    map: &'a SourceMap,
    text: &'a str,
    nodes: Vec<TreeNode>,
    text_len: ByteOffset,
}

impl<'a> ConvertCtx<'a> {
    /// 把 1-based `LineColumn` 换算为字节偏移。`is_start=true` 时列必须 ≥1；
    /// `is_start=false`（结束侧）允许列 0，表示落在该行行首（含 0 字节）。
    fn pixel(&self, line: usize, column: usize, is_start: bool) -> Option<ByteOffset> {
        if line == 0 {
            return None;
        }
        let line_start = self.map.offset_of(line, 0, self.text_len)?;
        let delta = if is_start {
            column.checked_sub(1)? as u64
        } else {
            column as u64
        };
        let off = ByteOffset::try_from_u64(line_start.u64_checked_add(delta)?)?;
        (off <= self.text_len).then_some(off)
    }

    /// 从 sourcepos 计算半开范围。`end.column` 是包含列。
    fn sourcepos_range(&self, sp: Sourcepos) -> Option<ByteRange> {
        let start = self.pixel(sp.start.line, sp.start.column, true)?;
        let end = self.pixel(sp.end.line, sp.end.column, false)?;
        Some(ByteRange::new(start, end.max(start)))
    }

    /// 计算容器间隙：全部子节点都可归属时，返回恰好 `children.len() + 1`
    /// 个间隙（`gap[0]` 前导、`gap[i]` 在第 i-1 与第 i 个子块之间、
    /// `gap[n]` 尾随；空区间也保留，保证与子块拼接恰为父范围）；
    /// 否则返回空（不承诺拼合契约，validate() 会标记）。
    fn gaps_for(
        source: Option<ByteRange>,
        children: &[NodeId],
        nodes: &[TreeNode],
    ) -> Vec<ByteRange> {
        let Some(range) = source else {
            return Vec::new();
        };
        if children
            .iter()
            .any(|c| nodes[c.0 as usize].source.is_none())
        {
            return Vec::new();
        }
        let mut gaps = Vec::with_capacity(children.len() + 1);
        let mut cursor = range.start;
        for c in children {
            let cr = nodes[c.0 as usize].source.expect("checked above");
            gaps.push(ByteRange::new(cursor, cr.start));
            cursor = cursor.max(cr.end);
        }
        gaps.push(ByteRange::new(cursor, range.end));
        gaps
    }

    /// ThematicBreak 的 comrak sourcepos 在「紧凑列表项紧跟兄弟行」或空行后
    /// 续文时会截短 TB 终点（实测 run−2 规律，如 `- ****\n- a` 的 TB 为
    /// `**`）；容器（Item/BlockQuote）终点可能同样被截短（如 `- ****\n\n- a`
    /// 的 Item 为 `- **`）。分隔线内容即整行标记，且整行都归属 TB：把
    /// TB 终点规范化为行尾（不含换行），并把终点与本 TB 原终点重合的祖先
    /// 容器一并延伸，使整文重解析与 reconcile 的片段重建产生一致的范围。
    fn canonicalize_thematic_breaks(&mut self) {
        // 父索引（children 列表反查）
        let mut parent_of = vec![None; self.nodes.len()];
        for i in 0..self.nodes.len() {
            for &c in &self.nodes[i].children {
                parent_of[c.0 as usize] = Some(i);
            }
        }
        for i in (0..self.nodes.len()).rev() {
            if self.nodes[i].kind != NodeKind::ThematicBreak {
                continue;
            }
            let Some(r) = self.nodes[i].source else {
                continue;
            };
            // 本 TB 所在行的行尾（不含换行）
            let rest = &self.text[r.end.0 as usize..];
            let line_end = match rest.find('\n') {
                Some(j) => {
                    let j = j as u32;
                    let base = r.end.0;
                    let raw_end = base + j;
                    if j > 0 && rest.as_bytes()[j as usize - 1] == b'\r' {
                        raw_end - 1
                    } else {
                        raw_end
                    }
                }
                None => self.text_len.0,
            };
            if line_end <= r.end.0 {
                continue;
            }
            let new_end = ByteOffset(line_end);
            let trunc_end = r.end;
            // 先延伸本 TB
            self.nodes[i].source = Some(ByteRange::new(r.start, new_end));
            // 再沿祖先链延伸终点与截短点重合的容器（Item/List/BlockQuote 等）：
            // 截短点对整条链是同一个字节，祖先的终点只与该点比较。
            let mut cur = parent_of[i];
            while let Some(p) = cur {
                let keep = matches!(
                    self.nodes[p].source,
                    Some(pr) if pr.end == trunc_end
                );
                if !keep {
                    break;
                }
                if let Some(pr) = self.nodes[p].source {
                    self.nodes[p].source = Some(ByteRange::new(pr.start, new_end));
                }
                cur = parent_of[p];
            }
        }
        // 范围变更后重算所有容器 gap（TB 无子不影响自身，祖先的 gap 依赖子范围）
        for i in 0..self.nodes.len() {
            let source = self.nodes[i].source;
            let children = self.nodes[i].children.clone();
            self.nodes[i].gaps = Self::gaps_for(source, &children, &self.nodes);
        }
    }

    /// 递归转换：先子后父（零列终点修复自底向上生效）。
    fn convert(&mut self, ast: &Ast, arena_node: &'a AstNode<'a>) -> NodeId {
        let is_document = matches!(ast.value, NodeValue::Document);
        let sp = ast.sourcepos;
        let kind = Self::map_kind(&ast.value);

        let child_ids: Vec<NodeId> = arena_node
            .children()
            .map(|c| {
                let child_ast = c.data();
                self.convert(&child_ast, c)
            })
            .collect();

        // 零列终点采用直接子节点的完整范围；最深 Text 不拥有容器尾部空白。
        let raw_range = self.sourcepos_range(sp);
        let source = if sp.end.column == 0 && !is_document {
            match (raw_range, child_ids.last().copied()) {
                (Some(r), Some(last)) => match self.nodes[last.0 as usize].source {
                    Some(child) => Some(ByteRange::new(r.start, child.end.max(r.start))),
                    None => Some(ByteRange::new(r.start, r.start)),
                },
                (Some(r), None) => Some(ByteRange::new(r.start, r.start)),
                (None, _) => None,
            }
        } else {
            raw_range
        };

        let mut flags = NodeFlags::NONE;
        if source.is_none() {
            flags |= NodeFlags::UNATTRIBUTED;
        }
        if kind == NodeKind::TableCell && child_ids.is_empty() {
            // 空单元/自动补足单元：范围可能指向分隔符，不做编辑目标。
            if let Some(r) = source {
                let slice = self.text.get(r.to_byte_range()).unwrap_or_default();
                if slice.chars().all(|c| matches!(c, '|' | ' ' | '\t' | ':')) {
                    flags |= NodeFlags::SYNTHETIC;
                }
            }
        }

        // TaskItem 的 checkbox 标记范围（symbol 是单个字符）。
        let symbol_range = match &ast.value {
            NodeValue::TaskItem(ti) => self.sourcepos_range(ti.symbol_sourcepos),
            _ => None,
        };
        let kind = match kind {
            NodeKind::TaskItem { symbol, .. } => NodeKind::TaskItem {
                symbol,
                symbol_source: symbol_range,
            },
            other => other,
        };

        let gaps = Self::gaps_for(source, &child_ids, &self.nodes);
        let id = NodeId(u32::try_from(self.nodes.len()).unwrap_or(u32::MAX));
        self.nodes.push(TreeNode {
            kind,
            source,
            flags,
            children: child_ids,
            gaps,
        });
        id
    }

    /// `NodeValue` → `NodeKind`。启用扩展之外的变体落入 `Raw` 防御分支。
    fn map_kind(v: &NodeValue) -> NodeKind {
        match v {
            NodeValue::Document => NodeKind::Document,
            NodeValue::BlockQuote => NodeKind::BlockQuote,
            NodeValue::List(NodeList {
                list_type,
                start,
                delimiter,
                bullet_char,
                tight,
                is_task_list,
                ..
            }) => NodeKind::List {
                ordered: matches!(list_type, ListType::Ordered),
                start: *start,
                delimiter: match delimiter {
                    ListDelimType::Period => ListDelimiter::Period,
                    ListDelimType::Paren => ListDelimiter::Paren,
                },
                bullet_char: (*bullet_char != 0).then_some(*bullet_char),
                tight: *tight,
                task_list: *is_task_list,
            },
            NodeValue::Item(_) => NodeKind::Item,
            NodeValue::CodeBlock(ncb) => NodeKind::CodeBlock {
                fenced: ncb.fenced,
                fence_char: ncb.fence_char,
                fence_length: ncb.fence_length,
                fence_offset: ncb.fence_offset,
                closed: ncb.closed,
                info: ncb.info.clone(),
            },
            NodeValue::HtmlBlock(nhb) => NodeKind::HtmlBlock {
                block_type: nhb.block_type,
            },
            NodeValue::Paragraph => NodeKind::Paragraph,
            NodeValue::Heading(nh) => NodeKind::Heading {
                level: nh.level,
                setext: nh.setext,
                closed: nh.closed,
            },
            NodeValue::ThematicBreak => NodeKind::ThematicBreak,
            NodeValue::Table(nt) => NodeKind::Table {
                num_columns: nt.num_columns,
                num_rows: nt.num_rows,
                num_nonempty_cells: nt.num_nonempty_cells,
            },
            NodeValue::TableRow(header) => NodeKind::TableRow { header: *header },
            NodeValue::TableCell => NodeKind::TableCell,
            NodeValue::Text(_) => NodeKind::Text,
            NodeValue::TaskItem(nti) => NodeKind::TaskItem {
                symbol: nti.symbol,
                symbol_source: None, // convert() 中补填
            },
            NodeValue::SoftBreak => NodeKind::SoftBreak,
            NodeValue::LineBreak => NodeKind::LineBreak,
            NodeValue::Code(nc) => NodeKind::Code {
                backtick_count: nc.num_backticks,
            },
            NodeValue::HtmlInline(_) => NodeKind::HtmlInline,
            NodeValue::Raw(_) => NodeKind::Raw {
                detail: RawDetail::ComrakRaw,
            },
            NodeValue::Emph => NodeKind::Emph,
            NodeValue::Strong => NodeKind::Strong,
            NodeValue::Strikethrough => NodeKind::Strikethrough,
            NodeValue::Link(nl) => NodeKind::Link {
                url: nl.url.clone(),
                title: nl.title.clone(),
            },
            NodeValue::Image(nl) => NodeKind::Image {
                url: nl.url.clone(),
                title: nl.title.clone(),
            },
            _ => NodeKind::Raw {
                detail: RawDetail::UnknownVariant("unmapped-node"),
            },
        }
    }
}
