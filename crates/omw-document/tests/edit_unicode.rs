//! Unicode 编辑语义：CJK、Emoji、组合字符、HTML 实体、反斜杠/星号/反引号转义，
//! 以及 UTF-8 字节 ↔ 视觉位置转换。

mod common;

use common::{byte_of, check_invariants, current_bytes, session, text};
use omw_document::{ByteOffset, EditError, EditSession, MarkKind, VisualPos, VisualRange};

fn pos(es: &EditSession, byte: u32) -> VisualPos {
    es.byte_to_visual(ByteOffset(byte))
        .expect("byte must be a grapheme start")
}

// ---- 字节 ↔ 视觉位置 ----

#[test]
fn visual_mapping_roundtrip() {
    // 多字节字符：聚合符号“😀”占 4 字节、CJK 占 3 字节、“\u{301}` 组合附标占 2 字节（与 e 合并为一个簇）。
    let fixture = "a\u{301}\u{4e16}\u{754c}\u{1F600}b\n"; // a◌́ 世界😀​b
    let (_, es) = session(fixture);
    assert_eq!(es.bytes().len(), 1 + 2 + 3 + 3 + 4 + 1 + 1); // 15 bytes
    // 每个簇起点都能正向/反向映射一致。
    for (i, byte) in [0u32, 3, 6, 9, 13, 14].into_iter().enumerate() {
        let vp = pos(&es, byte);
        assert_eq!(byte_of(&es, vp).0, byte, "byte {byte} roundtrip");
        assert_eq!(vp.0, i as u32, "byte {byte} → visual {i}");
    }
    // 簇中间的字节不能作为视觉位置。
    assert!(es.byte_to_visual(ByteOffset(1)).is_none(), "mid-combining");
    assert!(es.byte_to_visual(ByteOffset(7)).is_none(), "mid-CJK");
    assert!(es.byte_to_visual(ByteOffset(11)).is_none(), "mid-emoji");
}

#[test]
fn visual_len_is_grapheme_count() {
    let (_, es) = session("a\u{301}\u{4e16}\u{754c}\u{1F600}b\n");
    // 6 个可见簇（a◌́、世、界、😀、b、换行）：视觉位置 0..=5。
    assert_eq!(es.visual_len(), 6);
}

// ---- CJK 插入 / 删除 ----

#[test]
fn cjk_insert_and_delete() {
    let fixture = "\u{4f60}\u{597d}\u{4e16}\u{754c}\n"; // 你好世界
    let (_, mut es) = session(fixture);
    let at = pos(&es, 6); // “世”前
    es.insert_text(at, "\u{7f8e}").unwrap(); // 美
    assert_eq!(text(&es), "\u{4f60}\u{597d}\u{7f8e}\u{4e16}\u{754c}\n");
    check_invariants(&es, "cjk insert");
    // 删除刚插入的“美”（一个簇）。
    let start = pos(&es, 6);
    let end = pos(&es, 9);
    es.delete_text(VisualRange::new(start, end)).unwrap();
    assert_eq!(text(&es), fixture);
    check_invariants(&es, "cjk delete");
}

#[test]
fn cjk_selection_replace() {
    let (_, mut es) = session("\u{4e16}\u{754c}\u{4e2d}\u{6587}\n"); // 世界中　文
    let b = pos(&es, 3);
    let e = pos(&es, 9); // “界中”= bytes [3,9)
    es.replace_selection(VisualRange::new(b, e), "\u{65e5}\u{672c}")
        .unwrap(); // 日本
    assert_eq!(text(&es), "\u{4e16}\u{65e5}\u{672c}\u{6587}\n"); // 世 日本 文
    check_invariants(&es, "cjk replace");
}

// ---- Emoji / 组合字符 ----

#[test]
fn emoji_backspace_deletes_whole_cluster() {
    let fixture = "a\u{1F600}b\n"; // a😀b
    let (_, mut es) = session(fixture);
    // 视觉位置 v1 在 😀 之前，v2 在 😀 之后：删除 [1,2) 正好删掉整个 emoji。
    let b = pos(&es, 1);
    let e = pos(&es, 5);
    es.delete_text(VisualRange::new(b, e)).unwrap();
    assert_eq!(text(&es), "ab\n");
    check_invariants(&es, "emoji delete");
}

#[test]
fn zwj_family_treated_as_one_cluster() {
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}"; // 👨👩👧👦
    let fixture = format!("x{family}y\n");
    let (_, mut es) = session(&fixture);
    let b = pos(&es, 1);
    let e = pos(&es, 1 + 25); // 4 × 4 bytes + 3 × 3 bytes = 25
    es.delete_text(VisualRange::new(b, e)).unwrap();
    assert_eq!(text(&es), "xy\n");
    check_invariants(&es, "zwj delete");
}

#[test]
fn combining_marks_backspace() {
    let fixture = "cafe\u{301}\n"; // c a f e◌́（e 与组合重音合并为一个簇）
    let (_, mut es) = session(fixture);
    let b = pos(&es, 3); // “e”簇起点
    let e = pos(&es, 6); // “\n”簇起点（e+U+0301 = bytes [3,5)，加上 \n 前哨兵在 6）
    es.delete_text(VisualRange::new(b, e)).unwrap();
    assert_eq!(text(&es), "caf\n");
    check_invariants(&es, "combining delete");
}

// ---- HTML 实体 / 特殊字符 ----

#[test]
fn html_entities_are_literal_text() {
    let fixture = "\u{4e00}\u{884c}\n"; // 一行
    let (_, mut es) = session(fixture);
    let at = pos(&es, 6); // 行尾（在 \\n 之前）
    // 实体不转义：& 不在 CommonMark 转义集内，且重解析后仍是字面 Text。
    es.insert_text(at, "&amp;").unwrap();
    assert_eq!(text(&es), "\u{4e00}\u{884c}&amp;\n");
    check_invariants(&es, "entity insert");
}

#[test]
fn escaping_backslash_asterisk_backtick() {
    let (_, mut es) = session("plain\n");
    let at = pos(&es, 0);
    es.insert_text(at, "*").unwrap();
    assert_eq!(text(&es), "\\*plain\n");
    check_invariants(&es, "asterisk escaped");
    let at = pos(&es, 2);
    es.insert_text(at, "`").unwrap();
    assert_eq!(text(&es), "\\*\\`plain\n");
    check_invariants(&es, "backtick escaped");
    let at = pos(&es, 4);
    es.insert_text(at, "\\").unwrap();
    // 输入“反斜杠”被转义为两个反斜杠字节（\\→\\\\）。
    assert_eq!(text(&es), "\\*\\`\\\\plain\n");
    check_invariants(&es, "backslash escaped");
}

// ---- 多字节选区 ----

#[test]
fn multibyte_selection_toggle() {
    let fixture = "\u{4f60}\u{597d}\u{4e16}\u{754c}\n"; // 你好世界
    let (_, mut es) = session(fixture);
    let b = pos(&es, 6);
    let e = pos(&es, 12); // “世界” = bytes [6,12)
    es.toggle_mark(MarkKind::Bold, VisualRange::new(b, e))
        .unwrap();
    assert_eq!(text(&es), "\u{4f60}\u{597d}**\u{4e16}\u{754c}**\n");
    check_invariants(&es, "bold wrap cjk");
    // 再次 toggle 解掉标记：包裹后视觉簇变多，用视觉范围选“世界”。
    es.toggle_mark(MarkKind::Bold, VisualRange::new(VisualPos(4), VisualPos(6)))
        .unwrap();
    assert_eq!(text(&es), fixture);
    check_invariants(&es, "bold unwrap cjk");
}

#[test]
fn emoji_replacement_is_size_agnostic() {
    let fixture = "a\u{1F600}\u{1F601}b\n"; // a😀😁b
    let (_, mut es) = session(fixture);
    let b = pos(&es, 1);
    let e = pos(&es, 9); // 两个 emoji
    es.replace_selection(VisualRange::new(b, e), "\u{65e5}")
        .unwrap(); // 日
    assert_eq!(text(&es), "a\u{65e5}b\n");
    check_invariants(&es, "emoji replace");
}

// ---- 越界与拒绝路径 ----

#[test]
fn out_of_bounds_is_typed_error() {
    let (_, mut es) = session("\u{4e00}\n"); // 一
    let bad = VisualPos(es.visual_len() + 10);
    assert!(matches!(
        es.insert_text(bad, "x"),
        Err(EditError::PositionOutOfBounds)
    ));
    check_invariants(&es, "after out-of-bounds insert");
    // 状态未变。
    assert_eq!(current_bytes(&es), "\u{4e00}\n".as_bytes());
}

#[test]
fn combining_selection_edge_respects_boundaries() {
    let (_, mut es) = session("a\u{301}b\n");
    // 视觉范围 [1,2) = 簇 a◌́ 与 b 之间 → 选择 b 前的空档会落入 a◌́ .. b 中间的簇边界检查。
    // 这里选择 a◌́ 整个簇（视觉 [0,1)）。
    es.delete_text(VisualRange::new(VisualPos(0), VisualPos(1)))
        .unwrap();
    assert_eq!(text(&es), "b\n");
    check_invariants(&es, "cluster delete");
}
