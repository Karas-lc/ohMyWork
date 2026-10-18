//! 多字节/行结束符边界的字节范围正确性。

use omw_document::parse::parse;
use omw_document::{DocumentSession, NodeId, NodeKind};

fn rt(text: &str) -> DocumentSession {
    let bytes = text.as_bytes();
    let s = parse(bytes).expect("parse should succeed");
    assert_eq!(s.serialize().as_bytes(), bytes, "byte-identical roundtrip");
    if let Err(issues) = s.validate() {
        panic!("tree validation failed: {issues:#?}");
    }
    s
}

fn node_ids(s: &DocumentSession, pred: impl Fn(&NodeKind) -> bool) -> Vec<NodeId> {
    s.tree()
        .iter()
        .filter(|id| pred(&s.tree().node(*id).kind))
        .collect()
}

#[test]
fn emoji_at_multibyte_boundary() {
    // 4 字节字符紧贴行首/行尾。
    let s = rt("# 😀🚀\n\n文本\n");
    let ids = node_ids(&s, |k| matches!(k, NodeKind::Heading { .. }));
    let r = s.source_range(ids[0]).unwrap();
    assert!(r.to_byte_range().len() >= 3, "标题含 4 字节 emoji");
}

#[test]
fn mixed_lf_crlf_and_lone_cr() {
    let s = rt("一\r\n二\n三\r四\n");
    // 三种行结束符共存：范围仍逐字节有效。
    let texts: Vec<_> = node_ids(&s, |k| *k == NodeKind::Text)
        .iter()
        .map(|id| s.source_slice(*id).unwrap().text)
        .collect();
    assert_eq!(texts, vec!["一", "二", "三", "四"]);
}

#[test]
fn hard_break_with_multibyte_and_spaces() {
    let s = rt("好😀  \n下一行\n");
    assert_eq!(
        node_ids(&s, |k| *k == NodeKind::LineBreak).len(),
        1,
        "两空格后换行必须是硬换行"
    );
    // LineBreak 覆盖行尾空格 + 换行符。
    let slice = s
        .source_slice(node_ids(&s, |k| *k == NodeKind::LineBreak)[0])
        .unwrap();
    assert_eq!(slice.text, "  \n");
}

#[test]
fn tab_counts_as_one_column() {
    let s = rt("a\tb\n");
    let slice = s
        .source_slice(node_ids(&s, |k| *k == NodeKind::Text)[0])
        .unwrap();
    assert_eq!(slice.text, "a\tb");
}

#[test]
fn table_with_multibyte_cells() {
    let s = rt("| 中文 | 🚀 |\n|:----|----:|\n| 值 😀 | x |\n");
    assert!(
        node_ids(&s, |k| *k == NodeKind::TableCell).len() >= 4,
        "至少两行两列"
    );
}

#[test]
fn entity_and_escape_keep_source_bytes() {
    // 实体/转义在语义文本中被解码，但源切片必须保留原文。
    let s = rt("&amp; \\*x\\*&copy;\n");
    let slices: Vec<_> = node_ids(&s, |k| *k == NodeKind::Text)
        .iter()
        .map(|id| s.source_slice(*id).unwrap().text)
        .collect();
    assert!(
        slices
            .iter()
            .any(|s| s.contains("&amp;") && s.contains("\\*")),
        "原文实体/转义保留，实际 {slices:?}"
    );
}

#[test]
fn bom_bytes_are_first_gap() {
    let text = "\u{feff}标题\n";
    let s = rt(text);
    // BOM 字节 [0,3) 不被任何节点占用。
    let root = s.tree().root();
    let lead_gap = s.tree().node(root).gaps.first().copied().unwrap();
    assert_eq!(lead_gap.to_byte_range(), 0..3);
}
