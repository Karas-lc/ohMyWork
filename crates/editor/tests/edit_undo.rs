//! Undo/Redo 语义：每次操作可逆；undo 序列逐条还原到初始字节；
//! redo 序列逐条重放回到最终态；失败操作不污染历史。

mod common;

use common::{check_invariants, current_bytes, session};
use editor::{EditError, MarkKind, VisualPos, VisualRange};

fn pos(es: &editor::EditSession, byte: u32) -> VisualPos {
    es.byte_to_visual(editor::ByteOffset(byte))
        .expect("byte must be a grapheme start")
}

fn undo_all(es: &mut editor::EditSession) {
    while es.history().undo_depth() > 0 {
        es.undo().unwrap().expect("undo should yield a record");
        check_invariants(es, "after undo");
    }
}

fn redo_all(es: &mut editor::EditSession) {
    while es.history().redo_depth() > 0 {
        es.redo().unwrap().expect("redo should yield a record");
        check_invariants(es, "after redo");
    }
}

#[test]
fn insert_undo_redo_roundtrip() {
    let fixture = "alpha beta gamma\n";
    let (_, mut es) = session(fixture);
    let at = pos(&es, 6); // "beta" 前
    es.insert_text(at, "X").unwrap();
    assert_eq!(current_bytes(&es), b"alpha Xbeta gamma\n");
    es.undo().unwrap().unwrap();
    assert_eq!(current_bytes(&es), fixture.as_bytes());
    es.redo().unwrap().unwrap();
    assert_eq!(current_bytes(&es), b"alpha Xbeta gamma\n");
}

#[test]
fn delete_undo_restores_exact_bytes() {
    let fixture = "alpha beta gamma\n";
    let (_, mut es) = session(fixture);
    // 删除 "beta "（6..11）
    let b = pos(&es, 6);
    let e = pos(&es, 11);
    es.delete_text(VisualRange::new(b, e)).unwrap();
    assert_eq!(current_bytes(&es), b"alpha gamma\n");
    es.undo().unwrap().unwrap();
    assert_eq!(current_bytes(&es), fixture.as_bytes());
}

#[test]
fn replace_selection_undo_redo() {
    let fixture = "alpha beta gamma\n";
    let (_, mut es) = session(fixture);
    let b = pos(&es, 6);
    let e = pos(&es, 10); // "beta"
    es.replace_selection(VisualRange::new(b, e), "BETA")
        .unwrap();
    assert_eq!(current_bytes(&es), b"alpha BETA gamma\n");
    es.undo().unwrap().unwrap();
    assert_eq!(current_bytes(&es), fixture.as_bytes());
    es.redo().unwrap().unwrap();
    assert_eq!(current_bytes(&es), b"alpha BETA gamma\n");
}

#[test]
fn split_join_undo_redo_roundtrip() {
    let fixture = "alpha beta\n";
    let (_, mut es) = session(fixture);
    let at = pos(&es, 5); // 空格处
    es.split_block(at).unwrap();
    assert_eq!(current_bytes(&es), b"alpha\n\n beta\n");
    es.undo().unwrap().unwrap();
    assert_eq!(current_bytes(&es), fixture.as_bytes());
    es.redo().unwrap().unwrap();
    assert_eq!(current_bytes(&es), b"alpha\n\n beta\n");

    // 再 join 回去：join 用 \n 分隔符替换分隔区 [P.end, Q.start)
    // （含第二段前导空格已归入父级 gap），段内成为 SoftBreak。
    let ja = pos(&es, 1);
    assert!(es.join_blocks(ja).is_ok());
    assert_eq!(current_bytes(&es), b"alpha\nbeta\n");
    es.undo().unwrap().unwrap();
    assert_eq!(current_bytes(&es), b"alpha\n\n beta\n");
}

#[test]
fn toggle_mark_undo_redo() {
    let fixture = "one two three\n";
    let (_, mut es) = session(fixture);
    let b = pos(&es, 4);
    let e = pos(&es, 7); // "two"
    es.toggle_mark(MarkKind::Bold, VisualRange::new(b, e))
        .unwrap();
    assert_eq!(current_bytes(&es), b"one **two** three\n");
    es.undo().unwrap().unwrap();
    assert_eq!(current_bytes(&es), fixture.as_bytes());
    es.redo().unwrap().unwrap();
    assert_eq!(current_bytes(&es), b"one **two** three\n");
    // unwrap（再次 toggle = 移除标记）：redone 后坐标已移位，重新定位内容
    // one **two** three: t=6,w=7,o=8 → 内容叶 [6,9)
    let inside = pos(&es, 6);
    let content_e = pos(&es, 9);
    es.toggle_mark(MarkKind::Bold, VisualRange::new(inside, content_e))
        .unwrap();
    assert_eq!(current_bytes(&es), fixture.as_bytes());
    es.undo().unwrap().unwrap();
    assert_eq!(current_bytes(&es), b"one **two** three\n");
}

#[test]
fn cjk_undo_roundtrip() {
    let fixture = "你好世界\n";
    let (_, mut es) = session(fixture);
    let at = pos(&es, 3); // "好" 前
    es.insert_text(at, "美").unwrap();
    assert_eq!(current_bytes(&es), "你美好世界\n".as_bytes());
    es.undo().unwrap().unwrap();
    assert_eq!(current_bytes(&es), fixture.as_bytes());
}

#[test]
fn undo_all_restores_original_bytes() {
    let fixture = "alpha beta gamma\n\ndelta epsilon\n";
    let (_, mut es) = session(fixture);
    // 一串混合操作（全部为合法操作）
    es.insert_text(pos(&es, 0), "x ").unwrap();
    // 光标：x=0 ' '=1 alpha=2-6
    es.replace_selection(VisualRange::new(pos(&es, 2), pos(&es, 7)), "one")
        .unwrap();
    // 现在 “x one beta gamma\n\ndelta epsilon\n”：one=2-4
    es.toggle_mark(MarkKind::Italic, VisualRange::new(pos(&es, 2), pos(&es, 5)))
        .unwrap();
    check_invariants(&es, "mixed ops");
    let after = current_bytes(&es);
    // 全部撤销 → 回到初始字节
    undo_all(&mut es);
    assert_eq!(current_bytes(&es), fixture.as_bytes());
    // 全部重做 → 回到最终字节
    redo_all(&mut es);
    assert_eq!(current_bytes(&es), after);
}

#[test]
fn undo_redo_keeps_preceding_ops() {
    let fixture = "aaa\n";
    let (_, mut es) = session(fixture);
    es.insert_text(pos(&es, 1), "b").unwrap(); // 1 → "abaa\n"
    es.insert_text(pos(&es, 2), "c").unwrap(); // 2 → "abcaa\n"
    let mid = current_bytes(&es);
    es.undo().unwrap().unwrap(); // 撤销 2 → "abaa\n"
    assert_eq!(current_bytes(&es), b"abaa\n");
    es.redo().unwrap().unwrap();
    assert_eq!(current_bytes(&es), mid);
}

#[test]
fn new_edit_truncates_redo_tail() {
    let fixture = "aaa\n";
    let (_, mut es) = session(fixture);
    es.insert_text(pos(&es, 1), "b").unwrap();
    es.undo().unwrap().unwrap();
    assert_eq!(es.history().redo_depth(), 1);
    // 新操作会截断 redo tail（undo 后 undo_depth=0，新插入又加 1）
    es.insert_text(pos(&es, 0), "z").unwrap();
    assert_eq!(es.history().redo_depth(), 0);
    assert_eq!(es.history().undo_depth(), 1);
    // redo 无内容
    assert!(es.redo().unwrap().is_none());
}

#[test]
fn failed_op_leaves_state_unchanged() {
    let fixture = "普通的 文本\n";
    let (_, mut es) = session(fixture);
    let before = current_bytes(&es);
    // 空选区删除是 no-op 记录：不改变字节
    let at = pos(&es, 0);
    es.delete_text(VisualRange::new(at, at)).unwrap();
    assert_eq!(current_bytes(&es), before);
    assert_eq!(es.history().undo_depth(), 1);

    // 拒绝的操作不产生任何记录、也不改变状态
    let bad = es.insert_text(VisualPos(9999), "x");
    assert!(matches!(bad, Err(EditError::PositionOutOfBounds)));
    assert_eq!(current_bytes(&es), before);
    assert_eq!(es.history().undo_depth(), 1); // 只有 no-op 那条

    // 换行输入被拒绝
    let bad2 = es.insert_text(pos(&es, 0), "\n");
    assert!(matches!(bad2, Err(EditError::NewlineInInput)));
    assert_eq!(current_bytes(&es), before);

    // 空白开头的 wrap 被拒绝（"**a **" 会重解析为普通文本）
    // 普0-2 通3-5 的6-8 ' '9 文10-12 本13-15
    let bad3 = es.toggle_mark(MarkKind::Bold, VisualRange::new(pos(&es, 9), pos(&es, 10)));
    assert!(matches!(bad3, Err(EditError::MarkShapeMismatch)));
    assert_eq!(current_bytes(&es), before);

    // 跨标记节点（从 Strong 内部延伸到段外）被拒绝
    let b1 = pos(&es, 0);
    let e1 = pos(&es, 3);
    let _ = es.toggle_mark(MarkKind::Bold, VisualRange::new(b1, e1)); // 先加粗“普”
    // redone 后文本 "**普**通的 文本\n"：普2-4 通7-9
    let bad4 = es.toggle_mark(MarkKind::Bold, VisualRange::new(pos(&es, 2), pos(&es, 7)));
    assert!(matches!(bad4, Err(EditError::MarkShapeMismatch)));
    assert!(es.history().peek_undo().is_some()); // 成功的那次仍在
    es.undo().unwrap().unwrap(); // 撤销加粗，回到 before
    assert_eq!(current_bytes(&es), before);
    assert_eq!(es.history().undo_depth(), 1);
}

#[test]
fn undo_beyond_history_is_noop() {
    let fixture = "aaa\n";
    let (_, mut es) = session(fixture);
    assert!(es.undo().unwrap().is_none());
    assert!(es.redo().unwrap().is_none());
    assert_eq!(current_bytes(&es), fixture.as_bytes());
}

#[test]
fn mixed_operations_full_roundtrip() {
    let fixture = "alpha\n\nbeta gamma\n\ndelta\n";
    let (_, mut es) = session(fixture);
    // insert + split + join + toggle + replace 混合
    es.insert_text(pos(&es, 0), "> ").unwrap(); // 引用式前缀
    es.split_block(pos(&es, 7)).unwrap();
    es.join_blocks(pos(&es, 0)).unwrap();
    es.replace_selection(VisualRange::new(pos(&es, 6), pos(&es, 9)), "ZZZ")
        .unwrap();
    let final_bytes = current_bytes(&es);
    undo_all(&mut es);
    assert_eq!(current_bytes(&es), fixture.as_bytes());
    redo_all(&mut es);
    assert_eq!(current_bytes(&es), final_bytes);
}
