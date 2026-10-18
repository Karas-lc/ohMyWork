//! 任务元数据投影。
//!
//! 任务语义只来自 Task 扩展解析出的 [`NodeKind::TaskItem`]（checkbox 标记
//! 字符及字节范围由 AST 提供），**不扫描代码**：不会在
//! [`NodeKind::CodeBlock`]/[`NodeKind::Code`]/[`NodeKind::HtmlBlock`]/
//! [`NodeKind::HtmlInline`] 子树里识别任务，也不把原始 HTML 当正文处理。

use std::collections::HashSet;

use crate::model::DocumentSession;
use crate::model::{ByteOffset, ByteRange, LosslessTree, NodeId, NodeKind, TreeNode};

/// 任务列表项信息。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskItemInfo {
    pub node: NodeId,
    /// 是否勾选（`[x]`/`[X]`）。
    pub checked: bool,
    /// 标记字符（`x`/`X`；未勾选为 `None`）。
    pub symbol: Option<char>,
    /// 标记字符（不可为空）的字节范围。
    pub checkbox_range: Option<ByteRange>,
}

/// 收集全树的任务列表项。
pub fn task_items(tree: &LosslessTree) -> Vec<TaskItemInfo> {
    tree.nodes()
        .iter()
        .enumerate()
        .filter_map(|(i, n)| match &n.kind {
            NodeKind::TaskItem {
                symbol,
                symbol_source,
            } => {
                let checked = matches!(symbol, Some('x' | 'X'));
                Some(TaskItemInfo {
                    node: NodeId(u32::try_from(i).unwrap_or(u32::MAX)),
                    checked,
                    symbol: *symbol,
                    checkbox_range: *symbol_source,
                })
            }
            _ => None,
        })
        .collect()
}

/// V1 任务字段类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TaskField {
    Start,
    Due,
    Priority,
}

/// Task 日期字段（供 rebase / UI 共享；避免重复定义）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TaskDateField {
    Start,
    Due,
}

/// 解析后的任务信息，包含用于精确编辑的 source span。
/// 仅从语义文本范围（非 code/html/raw 子树）中识别字段。
/// 重复或无效字段保留原文文本，生成诊断，不提供可排序的值。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Task {
    pub item: TaskItemInfo,
    /// 标题：已移除合法的已识别字段；未知/无效/重复字段保留原文。
    pub title: String,
    /// 合法的 start 日期及对应 `[start::...]` 的精确源范围。
    pub start: Option<(String, ByteRange)>,
    pub due: Option<(String, ByteRange)>,
    /// 合法的 priority (1..=9, 9最高) 及范围。
    pub priority: Option<(u8, ByteRange)>,
    /// 诊断信息（无效日期、优先级越界、重复字段等）。
    pub diagnostics: Vec<String>,
}

/// 收集节点子树中属于语义文本的 Text 节点源范围（跳过代码、HTML、Raw）。
fn collect_semantic_text_ranges(tree: &LosslessTree, root: NodeId) -> Vec<ByteRange> {
    let mut ranges = Vec::new();
    fn walk(tree: &LosslessTree, id: NodeId, ranges: &mut Vec<ByteRange>) {
        let node = tree.node(id);
        if !is_semantic_text_node(node) {
            return;
        }
        if matches!(node.kind, NodeKind::Text) {
            if let Some(r) = node.source {
                ranges.push(r);
            }
        }
        for &child in &node.children {
            walk(tree, child, ranges);
        }
    }
    walk(tree, root, &mut ranges);
    ranges
}

fn is_in_semantic(ranges: &[ByteRange], r: ByteRange) -> bool {
    ranges
        .iter()
        .any(|sr| sr.start.0 <= r.start.0 && r.end.0 <= sr.end.0)
}

/// 严格日期校验：仅 YYYY-MM-DD 全数字 + 连字符。
pub(crate) fn valid_date(value: &str) -> Option<&str> {
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !bytes
            .iter()
            .enumerate()
            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit())
    {
        return None;
    }
    let year: u32 = value[..4].parse().ok()?;
    let month: u32 = value[5..7].parse().ok()?;
    let day: u32 = value[8..].parse().ok()?;
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    (year > 0 && (1..=days).contains(&day)).then_some(value)
}

/// 解析单个任务项的字段（start/due/priority）。
/// 只在 TaskItem 语义文本中识别；保留精确字段 source span 用于后续局部写回。
/// 重复字段：保留原文 + 诊断，该字段不提供值（不参与排序）。
pub fn parse_task(document: &DocumentSession, item: &TaskItemInfo) -> Option<Task> {
    let source = document.source_slice(item.node)?;
    let raw = source.text.as_ref();
    let semantic_ranges = collect_semantic_text_ranges(document.tree(), item.node);

    // 取任务所在首行（列表项标题行）。
    let line = raw.lines().next().unwrap_or(raw);
    let marker = [line.find("[ ]"), line.find("[x]"), line.find("[X]")]
        .into_iter()
        .flatten()
        .min();
    let title_start_in_line = marker.map_or(0, |offset| offset + 3);
    let title_source = &line[title_start_in_line..];

    let mut title = String::with_capacity(title_source.len());
    let mut start = None;
    let mut due = None;
    let mut priority = None;
    let mut diagnostics = Vec::new();
    let mut seen: HashSet<TaskField> = HashSet::new();
    let mut cursor = 0usize;

    while let Some(open_relative) = title_source[cursor..].find('[') {
        let open = cursor + open_relative;
        title.push_str(&title_source[cursor..open]);

        let Some(close_relative) = title_source[open..].find(']') else {
            title.push_str(&title_source[open..]);
            cursor = title_source.len();
            break;
        };
        let close = open + close_relative;
        let field_content = &title_source[open + 1..close];
        let field_full = &title_source[open..=close];

        // 计算该字段在文档中的绝对源范围。
        let base = source.range.start.0 as usize;
        let field_abs_start = base + title_start_in_line + open;
        let field_abs_end = base + title_start_in_line + close + 1;
        let field_range = ByteRange {
            start: ByteOffset(u32::try_from(field_abs_start).unwrap_or(u32::MAX)),
            end: ByteOffset(u32::try_from(field_abs_end).unwrap_or(u32::MAX)),
        };

        let in_semantic = is_in_semantic(&semantic_ranges, field_range);

        let mut recognized = false;
        if in_semantic {
            if let Some(value) = field_content.strip_prefix("start::") {
                let v = value.trim();
                if seen.contains(&TaskField::Start) {
                    diagnostics.push("duplicate start date".to_owned());
                    start = None;
                } else if let Some(valid) = valid_date(v) {
                    seen.insert(TaskField::Start);
                    start = Some((valid.to_owned(), field_range));
                    recognized = true;
                } else {
                    diagnostics.push("invalid start date".to_owned());
                    seen.insert(TaskField::Start); // taint: invalid prevents later
                }
            } else if let Some(value) = field_content.strip_prefix("due::") {
                let v = value.trim();
                if seen.contains(&TaskField::Due) {
                    diagnostics.push("duplicate due date".to_owned());
                    due = None;
                } else if let Some(valid) = valid_date(v) {
                    seen.insert(TaskField::Due);
                    due = Some((valid.to_owned(), field_range));
                    recognized = true;
                } else {
                    diagnostics.push("invalid due date".to_owned());
                    seen.insert(TaskField::Due);
                }
            } else if let Some(value) = field_content.strip_prefix("priority::") {
                let trimmed = value.trim();
                match trimmed.parse::<u8>().ok().filter(|p| (1..=9).contains(p)) {
                    Some(p) if !seen.contains(&TaskField::Priority) => {
                        seen.insert(TaskField::Priority);
                        priority = Some((p, field_range));
                        recognized = true;
                    }
                    Some(_) => {
                        diagnostics.push("duplicate priority".to_owned());
                        priority = None;
                    }
                    None => {
                        diagnostics.push("invalid priority".to_owned());
                        seen.insert(TaskField::Priority); // taint
                    }
                }
            }
        }

        if !recognized {
            title.push_str(field_full);
        }
        cursor = close + 1;
    }

    if cursor < title_source.len() {
        title.push_str(&title_source[cursor..]);
    }

    // 规范化剥离字段后可能产生的多余空白。
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");

    Some(Task {
        item: *item,
        title,
        start,
        due,
        priority,
        diagnostics,
    })
}

/// 兼容旧投影的简单字段提取（无 span）。新代码应使用 `parse_task`。
/// 保留以不破坏现有索引/测试。内部仍用旧逻辑（投影路径未全迁移）。
pub fn parse_task_fields(
    raw: &str,
) -> (
    String,
    Option<String>,
    Option<String>,
    Option<u8>,
    Vec<String>,
) {
    let line = raw.lines().next().unwrap_or(raw);
    let marker = [line.find("[ ]"), line.find("[x]"), line.find("[X]")]
        .into_iter()
        .flatten()
        .min();
    let title_start = marker.map_or(0, |offset| offset + 3);
    let title_source = &line[title_start..];
    let mut title = String::with_capacity(title_source.len());
    let mut start_date = None;
    let mut due_date = None;
    let mut priority = None;
    let mut diagnostics = Vec::new();
    let mut cursor = 0;
    while let Some(open_relative) = title_source[cursor..].find('[') {
        let open = cursor + open_relative;
        title.push_str(&title_source[cursor..open]);
        let Some(close_relative) = title_source[open..].find(']') else {
            title.push_str(&title_source[open..]);
            cursor = title_source.len();
            break;
        };
        let close = open + close_relative;
        let field = &title_source[open + 1..close];
        let recognized = if let Some(value) = field.strip_prefix("start::") {
            match valid_date(value.trim()) {
                Some(value) => start_date = Some(value.to_owned()),
                None => diagnostics.push("invalid start date".to_owned()),
            }
            true
        } else if let Some(value) = field.strip_prefix("due::") {
            match valid_date(value.trim()) {
                Some(value) => due_date = Some(value.to_owned()),
                None => diagnostics.push("invalid due date".to_owned()),
            }
            true
        } else if let Some(value) = field.strip_prefix("priority::") {
            match value
                .trim()
                .parse::<u8>()
                .ok()
                .filter(|value| (1..=9).contains(value))
            {
                Some(value) => priority = Some(value),
                None => diagnostics.push("invalid priority".to_owned()),
            }
            true
        } else {
            false
        };
        if !recognized {
            title.push_str(&title_source[open..=close]);
        }
        cursor = close + 1;
    }
    if cursor < title_source.len() {
        title.push_str(&title_source[cursor..]);
    }
    (
        title.trim().to_owned(),
        start_date,
        due_date,
        priority,
        diagnostics,
    )
}

/// 节点是否属于"语义文本"（非代码/原始 HTML）子树——供需要在正文中做
/// 额外文本识别（如下一步的 Dataview 字段）时使用。
pub fn is_semantic_text_node(node: &TreeNode) -> bool {
    !matches!(
        node.kind,
        NodeKind::CodeBlock { .. }
            | NodeKind::Code { .. }
            | NodeKind::HtmlBlock { .. }
            | NodeKind::HtmlInline
            | NodeKind::Raw { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse;

    #[test]
    fn task_items_collected_with_ranges() {
        let src = "- [ ] todo\n- [x] done\n- plain\n\n```\n- [ ] not a task\n```\n";
        let s = parse(src.as_bytes()).unwrap();
        let items = task_items(s.tree());
        assert_eq!(items.len(), 2);
        assert!(!items[0].checked);
        assert_eq!(items[0].symbol, None);
        assert!(items[1].checked);
        assert_eq!(items[1].symbol, Some('x'));
        let r = items[0].checkbox_range.unwrap();
        assert_eq!(s.byte_slice(r).as_deref(), Some(" "));
        let r = items[1].checkbox_range.unwrap();
        assert_eq!(s.byte_slice(r).as_deref(), Some("x"));
    }

    #[test]
    fn code_blocks_never_scanned() {
        let src = "    - [ ] indented code\n\n```md\n- [x] fenced\n```\n";
        let s = parse(src.as_bytes()).unwrap();
        assert_eq!(task_items(s.tree()).len(), 0);
    }

    #[test]
    fn parse_task_extracts_fields_with_spans_and_skips_code() {
        let src = "- [ ] 做事 [start:: 2026-10-01] [due:: 2026-10-15] [priority:: 9]\n\n```\n- [ ] fake [due:: 2030-01-01]\n```";
        let s = parse(src.as_bytes()).unwrap();
        let items = task_items(s.tree());
        assert_eq!(items.len(), 1);
        let t = parse_task(&s, &items[0]).expect("task parsed");
        assert_eq!(t.title, "做事");
        assert_eq!(
            t.start.as_ref().map(|(v, _)| v.as_str()),
            Some("2026-10-01")
        );
        assert_eq!(t.due.as_ref().map(|(v, _)| v.as_str()), Some("2026-10-15"));
        assert_eq!(t.priority.as_ref().map(|(p, _)| *p), Some(9));
        assert!(t.diagnostics.is_empty());
        let task_range = s.source_range(items[0].node).unwrap();
        if let Some((_, r)) = &t.start {
            assert!(task_range.start.0 <= r.start.0 && r.end.0 <= task_range.end.0);
        }
    }

    #[test]
    fn parse_task_skips_pseudo_fields_in_inline_code() {
        let src = "- [ ] real [start:: 2026-10-01] ` [due:: 2099-12-31] ` after";
        let s = parse(src.as_bytes()).unwrap();
        let items = task_items(s.tree());
        let t = parse_task(&s, &items[0]).expect("parsed");
        // note: normalize collapses spaces around stripped field
        assert_eq!(t.title, "real ` [due:: 2099-12-31] ` after");
        assert_eq!(
            t.start.as_ref().map(|(v, _)| v.as_str()),
            Some("2026-10-01")
        );
        assert!(t.due.is_none());
    }

    #[test]
    fn parse_task_handles_duplicates_and_invalids_without_stealing_last() {
        let src = "- [ ] t [start:: 2026-10-01] [start:: 2026-10-02] [priority:: 10] [priority:: 3] [due:: bad] foo";
        let s = parse(src.as_bytes()).unwrap();
        let items = task_items(s.tree());
        let t = parse_task(&s, &items[0]).expect("parsed");
        assert!(t.start.is_none());
        assert!(t.due.is_none());
        assert!(t.priority.is_none());
        assert!(t.diagnostics.iter().any(|d| d.contains("duplicate start")));
        assert!(
            t.diagnostics
                .iter()
                .any(|d| d.contains("duplicate priority"))
        );
        assert!(t.diagnostics.iter().any(|d| d.contains("invalid due")));
        assert!(t.diagnostics.iter().any(|d| d.contains("invalid priority")));
        assert!(t.title.contains("start:: 2026-10-02"));
        assert!(t.title.contains("priority:: 3"));
    }

    #[test]
    fn parse_task_keeps_unknown_fields() {
        let src = "- [ ] note [foo:: bar] [due:: 2026-11-11] [unknown:: x]";
        let s = parse(src.as_bytes()).unwrap();
        let items = task_items(s.tree());
        let t = parse_task(&s, &items[0]).expect("parsed");
        assert_eq!(t.title, "note [foo:: bar] [unknown:: x]");
        assert_eq!(t.due.as_ref().map(|(v, _)| v.as_str()), Some("2026-11-11"));
    }
}
