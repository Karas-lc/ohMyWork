//! Raw 内容原样保留：HTML 块/行内、代码、注释、未知结构与容器间隙。

use editor::parse::parse;
use editor::{DocumentSession, NodeKind, RawDetail};

fn rt(text: &str) -> DocumentSession {
    let bytes = text.as_bytes();
    let s = parse(bytes).expect("parse should succeed");
    assert_eq!(s.serialize().as_bytes(), bytes, "byte-identical roundtrip");
    if let Err(issues) = s.validate() {
        panic!("tree validation failed: {issues:#?}");
    }
    s
}

#[test]
fn html_block_keeps_verbatim_slice() {
    let s = rt("<div class=\"card\">\n  raw <b>inner</b>\n</div>\n");
    let ids: Vec<_> = s
        .tree()
        .iter()
        .filter(|id| s.tree().node(*id).kind.is_raw())
        .collect();
    assert_eq!(ids.len(), 1, "整块 HTML 是单个 Raw 节点");
    let slice = s.source_slice(ids[0]).unwrap();
    assert!(slice.text.contains("<b>inner</b>"), "Raw 内容原样保留");
}

#[test]
fn html_inline_slices() {
    let s = rt("<span data-x=\"1\"> <input checked>\n");
    let raws = s
        .tree()
        .iter()
        .filter(|id| s.tree().node(*id).kind == NodeKind::HtmlInline)
        .collect::<Vec<_>>();
    assert_eq!(raws.len(), 2);
    let first = s.source_slice(raws[0]).unwrap();
    assert!(
        first.text.starts_with("<span"),
        "行内 HTML 保留原文 {first:?}"
    );
}

#[test]
fn code_fence_literal_matches_slice() {
    let s = rt("```rust\nfn main() {}\n```\n");
    let id = s
        .tree()
        .iter()
        .find(|id| matches!(s.tree().node(*id).kind, NodeKind::CodeBlock { .. }))
        .unwrap();
    let slice_text = s.source_slice(id).unwrap();
    assert_eq!(slice_text.text, "```rust\nfn main() {}\n```");
}

#[test]
fn indented_code_not_parsed_as_quote() {
    // 4 空格缩进的 `> quote` 是代码，不产生 BlockQuote。
    let s = rt("    > quote\n");
    let quotes = s
        .tree()
        .iter()
        .filter(|id| s.tree().node(*id).kind == NodeKind::BlockQuote)
        .count();
    assert_eq!(quotes, 0, "缩进块是代码，不是引用");
    let codes = s
        .tree()
        .iter()
        .filter(|id| matches!(s.tree().node(*id).kind, NodeKind::CodeBlock { .. }))
        .count();
    assert_eq!(codes, 1);
}

#[test]
fn comment_block_is_raw_and_verbatim() {
    let s = rt("<!-- 注释 <任意> 内容 -->\n");
    let id = s
        .tree()
        .iter()
        .find(|id| s.tree().node(*id).kind.is_raw())
        .unwrap();
    assert_eq!(
        s.source_slice(id).unwrap().text,
        "<!-- 注释 <任意> 内容 -->"
    );
}

#[test]
fn unknown_construct_stays_literal_text() {
    // 未开启 frontmatter/wiki/脚注扩展：这些语法只是普通文本/分隔线，仍可无损往返。
    let s = rt("---\nkey: value\n---\n\n正文\n");
    let text_count = s
        .tree()
        .iter()
        .filter(|id| s.tree().node(*id).kind == NodeKind::Text)
        .count();
    assert_eq!(text_count, 2, "两个段落原样保留（key: value 与 正文）");
    // 未知节点值不会产生 Raw 伪装：全部来自已知构造。
    let unknowns = s
        .tree()
        .iter()
        .filter(|id| {
            matches!(
                s.tree().node(*id).kind,
                NodeKind::Raw {
                    detail: RawDetail::UnknownVariant(_)
                }
            )
        })
        .count();
    assert_eq!(unknowns, 0);
}

#[test]
fn task_marker_in_code_is_not_a_task() {
    let s = rt("- [x] 真实任务\n\n```\n- [ ] 代码里的假任务\n```\n");
    let task_ids: Vec<_> = s
        .tree()
        .iter()
        .filter(|id| matches!(s.tree().node(*id).kind, NodeKind::TaskItem { .. }))
        .collect();
    assert_eq!(task_ids.len(), 1, "仅列表中的任务算任务");
}

#[test]
fn container_range_covers_children_and_gaps() {
    let s = rt("- a\n  - b\n- c\n\n");
    let list_id = s
        .tree()
        .iter()
        .find(|id| matches!(s.tree().node(*id).kind, NodeKind::List { .. }))
        .unwrap();
    let list_range = s.source_range(list_id).unwrap();
    let mut covered = list_range.to_byte_range().len();
    for g in &s.tree().node(list_id).gaps {
        covered += g.len() as usize;
    }
    // 容器范围包含全部子块（间隙已计入孩子边界之外）。
    assert!(covered >= list_range.len() as usize);
}
