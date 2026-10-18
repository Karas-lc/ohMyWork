//! 最小化序列化：转义、补丁规划与校验。
//!
//! 保存路径（`plan_source_patches` → `apply_patches`）只产生**必要范围**
//! 的补丁：未改动节点复用其原始 SourceSlice，改动只扩到最小内联/块容器；
//! 相邻未改动内容之间不产生补丁。补丁针对同一基线（磁盘字节），
//! 倒序应用，且必须通过"应用→重解析→语义对比"校验，失败返回可诊断错误，
//! 绝不退化为全文格式化兜底。
//!
//! 普通输入的转义也在这里：插入/替换文本在进入工作缓冲前做一次
//! CommonMark 可转义字符转义，保证输入只作字面文本（仓库约束：正文保存
//! 只改写必要源范围；输入不产生结构化副作用）。

use crate::edit::EditSession;
use crate::model::{ByteOffset, ByteRange, DocumentSession, NodeKind};
use crate::parse::parse;
use std::fmt;

/// CommonMark 可转义字符集。插入/替换文本中的这些字符前加反斜杠，
/// 使它们在重解析时仍为字面文本（`\#`、`\*`、`\\` 等）。
pub fn escape_char(c: char) -> String {
    match c {
        '\\' | '`' | '*' | '_' | '{' | '}' | '[' | ']' | '(' | ')' | '#' | '+' | '-' | '.'
        | '!' | '|' | '>' | '<' => {
            let mut s = String::with_capacity(2);
            s.push('\\');
            s.push(c);
            s
        }
        _ => c.to_string(),
    }
}

/// 对整段输入做一次转义（逐字符）。原始输入中已有的字节**不**再转义——
/// 这是唯一的转义关，保证不会二次转义。
pub fn escape_text(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        out.push_str(&escape_char(c));
    }
    out
}

/// 还原序列化时的转义（单遍）。工作在**原始源字节**上：把紧随可转义
/// 字符之前的反斜杠去掉。comrak 解析 `\\*` 时 Text 节点的来源文本会
/// 排除转义反斜杠，因此投影比较前必须对两侧（工作树与重解析）对称地
/// 做一次 unescape，才能得到一致的字面文本。
pub fn unescape_text(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c == '\\' {
            // 后一个字符属于 CommonMark 可转义标点集合（含空格）时剥掉反斜杠。
            if let Some(&(j, next)) = chars.peek()
                && j == i + 1
                && is_escaped_mark(next)
            {
                out.push(next);
                chars.next();
                continue;
            }
            out.push(c);
        } else {
            out.push(c);
        }
    }
    out
}

/// CommonMark 可转义标点（ASCII punctuation）：反斜杠后紧跟这些字符时,
/// comrak 会把反斜杠从 Text 节点字面内容中去掉（probe: `\\:` → `:`)。
/// 空格不是可转义标点：`a\\ b` 保留反斜杠（probe: `a\\ b`）。
fn is_escaped_mark(c: char) -> bool {
    ('!'..='/').contains(&c)
        || (':'..='@').contains(&c)
        || ('['..='`').contains(&c)
        || ('{'..='~').contains(&c)
}

/// FNV-1a 64 位哈希（非加密用途：仅作补丁内容的可信性校验）。
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// 针对同一基线的补丁项：半开字节范围 + 期望哈希 + 替换文本。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextPatch {
    pub range: ByteRange,
    pub expected_hash: u64,
    pub replacement: String,
}

/// 补丁应用错误（可诊断，绝不 panic）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatchError {
    /// 范围越出基线字节边界。
    OutOfBounds(ByteRange),
    /// 范围端点不是字符边界（会破坏 UTF-8）。
    NotCharBoundary(ByteRange),
    /// 基线内容与预期不一致（外部修改或规划错误）。
    HashMismatch {
        range: ByteRange,
        expected: u64,
        actual: u64,
    },
    /// 补丁间范围重叠。
    Overlap { a: ByteRange, b: ByteRange },
}

impl fmt::Display for PatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PatchError::OutOfBounds(r) => {
                write!(f, "patch range {} exceeds baseline length", r.to_debug())
            }
            PatchError::NotCharBoundary(r) => {
                write!(f, "patch range {} splits a UTF-8 char", r.to_debug())
            }
            PatchError::HashMismatch {
                range,
                expected,
                actual,
            } => write!(
                f,
                "baseline content at {} changed (expected hash {expected:#x}, got {actual:#x})",
                range.to_debug()
            ),
            PatchError::Overlap { a, b } => write!(
                f,
                "overlapping patches: {} and {}",
                a.to_debug(),
                b.to_debug()
            ),
        }
    }
}

impl std::error::Error for PatchError {}

/// 补丁规划结果。`verified` 为 `true` 表示已通过
/// "应用→重解析→语义对比" 校验（规划时即执行）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatchPlan {
    pub patches: Vec<TextPatch>,
    /// 是否已通过与工作语义的重解析校验。
    pub verified: bool,
    /// 基线字节长度。
    pub base_len: u32,
    /// 基线全文 FNV-1a（诊断用途）。
    pub base_hash: u64,
}

/// 规划错误。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlanError {
    /// 工作文本为空（V1 不支持空文档编辑，无可靠锚点）。
    EmptyWorkingText,
    /// 补丁校验失败（内容与基线不一致等）。
    Patch(PatchError),
    /// 应用结果无法解析。
    Parse(String),
    /// 应用结果与工作语义不一致（或字节不一致）。
    ReparseMismatch { detail: String },
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlanError::EmptyWorkingText => {
                write!(f, "cannot plan patches for empty working text")
            }
            PlanError::Patch(e) => write!(f, "patch planning failed: {e}"),
            PlanError::Parse(e) => write!(f, "cannot re-parse patched output: {e}"),
            PlanError::ReparseMismatch { detail } => {
                write!(
                    f,
                    "patched output does not match working semantics: {detail}"
                )
            }
        }
    }
}

impl std::error::Error for PlanError {}

/// 把补丁应用到同一基线字节上。补丁必须互不重叠，应用顺序为
/// 倒序（可从任意的低基数位置安全修改）。不产生半文件：只返回新字节，
/// 写盘由调用方在保存事务中完成。
pub fn apply_patches(base_bytes: &[u8], patches: &[TextPatch]) -> Result<Vec<u8>, PatchError> {
    let mut sorted: Vec<&TextPatch> = patches.iter().collect();
    sorted.sort_by_key(|p| std::cmp::Reverse(p.range.start.0));

    // 校验：边界、字符边界、哈希、重叠。
    let mut prev: Option<ByteRange> = None;
    for p in &sorted {
        let r = p.range.to_byte_range();
        if r.end > base_bytes.len() {
            return Err(PatchError::OutOfBounds(p.range));
        }
        let s = std::str::from_utf8(base_bytes).map_err(|_| PatchError::OutOfBounds(p.range))?;
        if !s.is_char_boundary(r.start) || !s.is_char_boundary(r.end) {
            return Err(PatchError::NotCharBoundary(p.range));
        }
        if let Some(prev) = prev
            && r.end > prev.start.to_usize()
        {
            return Err(PatchError::Overlap {
                a: prev,
                b: p.range,
            });
        }
        prev = Some(p.range);
        let actual = fnv1a64(&base_bytes[r.start..r.end]);
        if actual != p.expected_hash {
            return Err(PatchError::HashMismatch {
                range: p.range,
                expected: p.expected_hash,
                actual,
            });
        }
    }

    // 按 range.start 升序顺序拼装：补丁互不重叠（上面已校验），
    // 每个补丁用替换内容替代对应的基线片段，未覆盖的片段原样保留。
    let mut sorted_asc: Vec<&TextPatch> = patches.iter().collect();
    sorted_asc.sort_by_key(|p| p.range.start.0);
    let mut out = Vec::new();
    let mut cursor = 0usize;
    for p in &sorted_asc {
        let r = p.range.to_byte_range();
        out.extend_from_slice(&base_bytes[cursor..r.start]);
        out.extend_from_slice(p.replacement.as_bytes());
        cursor = r.end;
    }
    out.extend_from_slice(&base_bytes[cursor..]);
    Ok(out)
}

/// 锚点：工作树中"内容未变且父结构未变"的叶节点，其基线与工作范围一一对应。
#[derive(Clone, Copy, Debug)]
struct Anchor {
    base_range: ByteRange,
    work_range: ByteRange,
}

/// 语义投影：文档顺序 DFS 的 `(NodeKind, 原文)` 序列，用于树↔重解析对比。
/// NodeKind 的 Eq 覆盖结构（含解码后的 Link/Image 元数据）。
pub fn tree_semantic_projection(
    tree: &crate::model::LosslessTree,
    text: &str,
) -> Vec<(NodeKind, String)> {
    fn walk(
        tree: &crate::model::LosslessTree,
        text: &str,
        id: crate::model::NodeId,
        out: &mut Vec<(NodeKind, String)>,
    ) {
        let node = tree.node(id);
        // Text 节点的字面内容由 comrak 剥掉转义反斜杠（probe: `\\*plain` →
        // Text=`*plain`），而工作树存的是转义后字节；投影两侧统一本 unescape
        // 后才能做语义一致性比较。容器节点（Paragraph 等）保留原始字节。
        let raw = node
            .source
            .map(|r| text[r.to_byte_range()].to_string())
            .unwrap_or_default();
        let bytes = if node.kind == NodeKind::Text {
            unescape_text(&raw)
        } else {
            raw
        };
        out.push((node.kind.clone(), bytes));
        for &child in &node.children {
            walk(tree, text, child, out);
        }
    }
    let mut out = Vec::new();
    if !tree.is_empty() {
        walk(tree, text, tree.root(), &mut out);
    }
    out
}

/// 规划"工作树 → 基线文本"的最小补丁集。
///
/// 只把实际变化（改动/新增/删除的节点与间隙）映射为补丁：`base` 中
/// 未被改动的内容（原文、间隙、换行、空白）不产生任何补丁；新增内容
/// 的补丁覆盖到其最小内联/块容器。
pub fn plan_source_patches(
    base: &DocumentSession,
    working: &EditSession,
) -> Result<PatchPlan, PlanError> {
    let base_bytes = base.bytes();
    let live_base =
        String::from_utf8(base_bytes.clone()).map_err(|e| PlanError::Parse(e.to_string()))?;
    let work_len = working.len_bytes();
    if work_len == 0 {
        return Err(PlanError::EmptyWorkingText);
    }
    let base_len = live_base.len();

    // 1) 收集锚点（文档顺序）。
    let mut anchors: Vec<Anchor> = Vec::new();
    {
        let wtree = working.tree();
        let parents = wtree.parent_index();
        for leaf in wtree.alive_dfs() {
            if !wtree.node(leaf).children.is_empty() {
                continue; // 只锚定叶节点
            }
            let id_u = leaf.0 as usize;
            if id_u >= working.base_len() {
                continue; // 本次编辑新建的节点不是锚点
            }
            let Some(w_range) = wtree.node(leaf).source else {
                continue;
            };
            let b_node = base.tree().node(leaf);
            if wtree.node(leaf).kind != b_node.kind {
                continue;
            }
            let Some(b_range) = b_node.source else {
                continue;
            };
            // 叶内容一致（基线与工作字节相同）
            let Some(w_slice) = working.byte_slice(w_range) else {
                continue;
            };
            if b_range.len() != w_range.len()
                || live_base.get(b_range.to_byte_range()) != Some(w_slice.as_ref())
            {
                continue;
            }
            // 父节点干净：基线存在、类型一致、子列表一致、间隙字节一致
            let Some(parent_id) = parents.get(leaf.0 as usize).copied().flatten() else {
                continue;
            };
            if parent_id.0 as usize >= working.base_len() {
                continue;
            }
            let w_parent = wtree.node(parent_id);
            let b_parent = base.tree().node(parent_id);
            if w_parent.kind != b_parent.kind || w_parent.children != b_parent.children {
                continue;
            }
            let mut gaps_ok = w_parent.gaps.len() == b_parent.gaps.len();
            if gaps_ok {
                for (w_gap, b_gap) in w_parent.gaps.iter().zip(b_parent.gaps.iter()) {
                    let Some(ws) = working.byte_slice(*w_gap) else {
                        gaps_ok = false;
                        break;
                    };
                    if live_base.get(b_gap.to_byte_range()) != Some(ws.as_ref()) {
                        gaps_ok = false;
                        break;
                    }
                }
            }
            if !gaps_ok {
                continue;
            }
            anchors.push(Anchor {
                base_range: b_range,
                work_range: w_range,
            });
        }
    }

    // 2) 锚点之间与首尾的"区域"：内容一致则跳过，不一致则出补丁。
    let mut patches: Vec<TextPatch> = Vec::new();
    let push_region = |b_span: ByteRange, w_span: ByteRange, patches: &mut Vec<TextPatch>| {
        let Some(b) = live_base.get(b_span.to_byte_range()) else {
            return;
        };
        let Some(w) = working.byte_slice(w_span) else {
            return;
        };
        if b == w {
            return; // 一致：无补丁
        }
        // 裁剪前后字节完全相同的部分（例如段落前/后的 \n\n 分隔、列表标记），
        // 使补丁只覆盖实际变化的内部范围（"最小容器"）。边界对齐到字符边界，
        // 避免截断多字节字符。
        let b_len = b.len();
        let w_len = w.len();
        let total = b_len.min(w_len);
        let mut cp = 0usize;
        while cp < total && b.as_bytes()[cp] == w.as_bytes()[cp] {
            cp += 1;
        }
        let mut cs = 0usize;
        while cs < total - cp && b.as_bytes()[b_len - 1 - cs] == w.as_bytes()[w_len - 1 - cs] {
            cs += 1;
        }
        // 将裁剪点推至字符边界（向区域内扩，宁可多包几个字节）。
        let mut cp_adj = cp;
        while !live_base.is_char_boundary(b_span.start.0 as usize + cp_adj) {
            cp_adj += 1;
        }
        let mut cs_adj = cs;
        while !live_base.is_char_boundary(b_span.end.0 as usize - cs_adj) {
            cs_adj += 1;
        }
        // 基线内域 [start+cp_adj, end-cs_adj) 与工作内域：任一侧可为空
        // （纯插入：基线为空；纯删除：工作为空；例如 join/split 的 \n 净变）。
        let inner_b = ByteOffset(b_span.start.0 + cp_adj as u32);
        let inner_e = ByteOffset(b_span.end.0 - cs_adj as u32);
        let w_inner_b = ByteOffset(w_span.start.0 + cp_adj as u32);
        let w_inner_e = ByteOffset(w_span.end.0 - cs_adj as u32);
        let b_empty = inner_b.0 >= inner_e.0;
        let w_empty = w_inner_b.0 >= w_inner_e.0;
        if b_empty && w_empty {
            return; // 无实质差异
        }
        // 补丁范围：基线内域；纯插入（基线侧为空）以共同前缀末为插入点。
        let (r_start, r_end) = if b_empty {
            (inner_b, inner_b)
        } else {
            (inner_b, inner_e)
        };
        let Some(w_inner) = working.byte_slice(ByteRange::new(w_inner_b, w_inner_e)) else {
            return;
        };
        patches.push(TextPatch {
            range: ByteRange::new(r_start, r_end),
            expected_hash: fnv1a64(
                live_base
                    .as_bytes()
                    .get(r_start.0 as usize..r_end.0 as usize)
                    .unwrap_or_default(),
            ),
            replacement: w_inner.to_string(),
        });
    };

    if !anchors.is_empty() {
        let mut prev: Option<Anchor> = None;
        for a in &anchors {
            let b_span = ByteRange {
                start: prev.map(|p| p.base_range.end).unwrap_or(ByteOffset(0)),
                end: a.base_range.start,
            };
            let w_span = ByteRange {
                start: prev.map(|p| p.work_range.end).unwrap_or(ByteOffset(0)),
                end: a.work_range.start,
            };
            push_region(b_span, w_span, &mut patches);
            prev = Some(*a);
        }
        let last = prev.unwrap();
        let b_tail = ByteRange {
            start: last.base_range.end,
            end: ByteOffset(u32::try_from(base_len).unwrap_or(u32::MAX)),
        };
        let w_tail = ByteRange {
            start: last.work_range.end,
            end: ByteOffset(u32::try_from(work_len).unwrap_or(u32::MAX)),
        };
        push_region(b_tail, w_tail, &mut patches);
    } else {
        // 零锚点：整篇都变了（例如单段落拆分）。这是"必要范围"的唯一形态，
        // 不是格式化兜底；字节一致时不会产生补丁（无锚点但无差异
        // 只可能是空文档，已在上面拒绝）。
        let b_span = ByteRange {
            start: ByteOffset(0),
            end: ByteOffset(u32::try_from(base_len).unwrap_or(u32::MAX)),
        };
        let w_span = ByteRange {
            start: ByteOffset(0),
            end: ByteOffset(u32::try_from(work_len).unwrap_or(u32::MAX)),
        };
        push_region(b_span, w_span, &mut patches);
    }

    // 3) 校验：应用补丁后字节必须与工作文本逐字节一致，且重解析语义一致。
    let applied = apply_patches(&base_bytes, &patches).map_err(PlanError::Patch)?;
    let applied_str = String::from_utf8(applied).map_err(|e| PlanError::Parse(e.to_string()))?;
    if applied_str.as_bytes() != working.bytes() {
        return Err(PlanError::ReparseMismatch {
            detail: "applied bytes differ from working text".to_string(),
        });
    }
    let reparsed = parse(applied_str.as_bytes()).map_err(|e| PlanError::Parse(e.to_string()))?;
    let want = working.semantic_projection();
    let got = tree_semantic_projection(reparsed.tree(), reparsed.serialize().as_str());
    if want != got {
        return Err(PlanError::ReparseMismatch {
            detail: format!(
                "semantics diverged ({} vs {} entries)",
                want.len(),
                got.len()
            ),
        });
    }

    Ok(PatchPlan {
        patches,
        verified: true,
        base_len: u32::try_from(base_len).unwrap_or(u32::MAX),
        base_hash: fnv1a64(&base_bytes),
    })
}

/// 供测试直接在字节上验证补丁集与目标文本一致。
pub fn verify_patches(
    base_bytes: &[u8],
    patches: &[TextPatch],
    expected: &[u8],
) -> Result<(), String> {
    let applied = apply_patches(base_bytes, patches).map_err(|e| format!("apply failed: {e}"))?;
    if applied != expected {
        return Err(format!(
            "patched bytes differ from expected ({} vs {} bytes)",
            applied.len(),
            expected.len()
        ));
    }
    Ok(())
}
