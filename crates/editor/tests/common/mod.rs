//! 编辑测试共享工具。
//!
//! - 构造会话：`session(src)` 给出基线 `DocumentSession` 与工作 `EditSession`。
//! - 光标位置：簇起点在文字叶内的都算合法可编辑位（不做假设，只枚举有效位）。
//! - 断言不变量：`check_invariants`（树校验 + 语义投影重解析一致性）。
//!
//! 每个测试二进制只使用部分工具；整套保持可用即允许未用项。
#![allow(dead_code)]

use editor::{ByteOffset, DocumentSession, EditSession, VisualPos, parse};

pub fn session(src: &str) -> (DocumentSession, EditSession) {
    let doc = parse(src.as_bytes()).expect("fixture must parse");
    if let Err(issues) = doc.validate() {
        panic!("base tree invalid: {issues:#?}");
    }
    let es = EditSession::from_document(&doc);
    (doc, es)
}

pub fn text(es: &EditSession) -> String {
    String::from_utf8(es.bytes()).expect("working text must be utf-8")
}

/// 工作文本当前全部字节（直接比较用）。
pub fn current_bytes(es: &EditSession) -> Vec<u8> {
    es.bytes()
}

/// 文档内所有"可编辑位"（文字叶内、簇起点）的 VisualPos，升序。
/// 注入/删选只在这些位置上做，保证不越界、不落在 Raw/间隙。
pub fn editable_positions(es: &EditSession) -> Vec<VisualPos> {
    let text_s = text(es);
    let leaves = es.text_leaves();
    let mut out = Vec::new();
    for (_, r) in leaves {
        let (s, e) = (r.start.0 as usize, r.end.0 as usize);
        let mut i = s;
        while i < e {
            let ch_len = text_s[i..].chars().next().map(char::len_utf8).unwrap_or(1);
            let p = ByteOffset(i as u32);
            if let Some(v) = es.byte_to_visual(p) {
                out.push(v);
            }
            i += ch_len;
        }
    }
    out.sort_by_key(|v| v.0);
    out.dedup();
    out
}

/// 某一叶内的可编辑位（含叶末尾，即 [start..=end] 的簇起点集合）。
pub fn leaf_positions(es: &EditSession, leaf: usize) -> Vec<VisualPos> {
    let text_s = text(es);
    let leaves = es.text_leaves();
    let (_, r) = &leaves[leaf];
    let (s, e) = (r.start.0 as usize, r.end.0 as usize);
    let mut out = Vec::new();
    let mut i = s;
    loop {
        let p = ByteOffset(i as u32);
        if let Some(v) = es.byte_to_visual(p) {
            out.push(v);
        }
        if i >= e {
            break;
        }
        let ch_len = text_s[i..].chars().next().map(char::len_utf8).unwrap_or(1);
        i += ch_len;
    }
    out
}

/// 可编辑位 → 字节偏移（借助会话映射，顺带验证双向一致）。
pub fn byte_of(es: &EditSession, v: VisualPos) -> ByteOffset {
    es.visual_to_byte(v).expect("visual pos must map back")
}

/// 树校验 + 语义投影重解析一致性。
pub fn check_invariants(es: &EditSession, ctx: &str) {
    if let Err(issues) = es.validate() {
        panic!("{ctx}: tree validation failed: {issues:#?}");
    }
    if let Err(detail) = es.assert_semantics_consistent() {
        panic!("{ctx}: semantics diverged from reparse: {detail}");
    }
}
