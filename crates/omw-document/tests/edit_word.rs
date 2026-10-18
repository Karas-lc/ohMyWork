//! 单词级最小序列化测试：改一个词 → 补丁只覆盖必要范围，
//! 不触碰相邻段落 / 空白 / 换行 / 未知字段（Raw）。
//! 同时覆盖输入转义（* ` # [ \ 等只作字面文本）。

mod common;

use common::{byte_of, check_invariants, current_bytes, leaf_positions, session, text};
use omw_document::{
    ByteOffset, EditError, MarkKind, VisualPos, VisualRange, apply_patches, escape_text,
    plan_source_patches, verify_patches,
};

fn patch_bytes(patch: &omw_document::TextPatch) -> (usize, usize) {
    (patch.range.start.0 as usize, patch.range.end.0 as usize)
}

/// 三段式文档：改中间段落的词，补丁应是且仅是该段落的文本跨度。
#[test]
fn word_change_single_patch() {
    let fixture = "alpha beta gamma\n\ndelta epsilon zeta\n\neta theta iota\n";
    let (base, mut es) = session(fixture);

    // 替换 epsilon → EPSILON（同位次，字节同长：7 vs 7）。
    let eps_start = fixture.find("epsilon").unwrap();
    let v0 = es.byte_to_visual(ByteOffset(eps_start as u32)).unwrap();
    let v1 = es
        .byte_to_visual(ByteOffset((eps_start + 7) as u32))
        .unwrap();
    let rec = es
        .replace_selection(VisualRange::new(v0, v1), "EPSILON")
        .expect("replace must succeed");
    assert_eq!(
        rec.op,
        omw_document::SemanticOp::ReplaceSelection {
            range: VisualRange::new(v0, v1),
            text: "EPSILON".to_string(),
        }
    );

    check_invariants(&es, "after replace");

    let plan = plan_source_patches(&base, &es).expect("plan must succeed");
    assert!(plan.verified);
    // 恰好一个补丁，范围 == 被改的词本身（前后缀裁剪后）。
    assert_eq!(plan.patches.len(), 1, "patches: {:?}", plan.patches);
    let (ps, pe) = patch_bytes(&plan.patches[0]);
    assert_eq!((ps, pe), (eps_start, eps_start + 7));
    assert_eq!(plan.patches[0].replacement, "EPSILON");
    // 补丁内容哈希与实际基线切片一致。
    let want_hash = omw_document::fnv1a64(fixture.as_bytes().get(ps..pe).unwrap());
    assert_eq!(plan.patches[0].expected_hash, want_hash);

    // 应用到基线 → 与工作文本逐字节一致。
    let applied = apply_patches(base.bytes().as_slice(), &plan.patches).unwrap();
    assert_eq!(applied, current_bytes(&es));
    verify_patches(base.bytes().as_slice(), &plan.patches, &current_bytes(&es))
        .expect("verify_patches");
}

/// 改行内标记中的词：补丁只覆盖内层文本，标记 ** 不进补丁。
#[test]
fn word_change_inside_markup() {
    let fixture = "one **two** three\n";
    let (base, mut es) = session(fixture);

    let two_start = fixture.find("**two**").unwrap() + 2;
    let v0 = es.byte_to_visual(ByteOffset(two_start as u32)).unwrap();
    let v1 = es
        .byte_to_visual(ByteOffset((two_start + 3) as u32))
        .unwrap();
    es.replace_selection(VisualRange::new(v0, v1), "TWO")
        .expect("replace must succeed");
    check_invariants(&es, "after markup word change");

    let plan = plan_source_patches(&base, &es).unwrap();
    assert_eq!(plan.patches.len(), 1, "patches: {:?}", plan.patches);
    let (ps, pe) = patch_bytes(&plan.patches[0]);
    // 恰好是内层 "two" 文本；** 标记不在补丁内。
    assert_eq!((ps, pe), (two_start, two_start + 3));
    assert_eq!(plan.patches[0].replacement, "TWO");
    verify_patches(base.bytes().as_slice(), &plan.patches, &current_bytes(&es)).unwrap();
}

/// 改词不触碰相邻段落 / 空白 / 换行 / Raw 块。
#[test]
fn word_change_leaves_neighbors_and_raw_untouched() {
    let fixture = "alpha\n\n<div>\nraw html\n</div>\n\nbeta gamma\n";
    let (base, mut es) = session(fixture);

    let alpha_start = fixture.find("alpha").unwrap();
    let v0 = es.byte_to_visual(ByteOffset(alpha_start as u32)).unwrap();
    let v1 = es
        .byte_to_visual(ByteOffset((alpha_start + 5) as u32))
        .unwrap();
    es.replace_selection(VisualRange::new(v0, v1), "ALPHA")
        .expect("replace must succeed");
    check_invariants(&es, "after alpha change");

    let plan = plan_source_patches(&base, &es).unwrap();
    assert!(plan.verified);
    // 补丁不得与分隔符/Raw 块重叠。
    let html_start = fixture.find("<div>").unwrap();
    let html_end = fixture.find("</div>").unwrap() + "</div>".len();
    for p in &plan.patches {
        let (ps, pe) = patch_bytes(p);
        assert!(
            pe <= html_start || ps >= html_end,
            "patch overlaps raw block: {ps}..{pe} vs {html_start}..{html_end}"
        );
        // 补丁内容不含 \n（不触碰换行/空白）。
        assert!(
            !p.replacement.contains('\n'),
            "replacement must not contain newlines: {:?}",
            p.replacement
        );
    }
    verify_patches(base.bytes().as_slice(), &plan.patches, &current_bytes(&es)).unwrap();
}

/// 删除一个词：补丁同样只覆盖该段落。
#[test]
fn delete_word_produces_minimal_patch() {
    let fixture = "aaa\n\nbbb ccc ddd\n\neee\n";
    let (base, mut es) = session(fixture);

    let ccc_start = fixture.find("ccc").unwrap();
    let v0 = es.byte_to_visual(ByteOffset(ccc_start as u32)).unwrap();
    let v1 = es
        .byte_to_visual(ByteOffset((ccc_start + 3) as u32))
        .unwrap();
    es.delete_text(VisualRange::new(v0, v1)).expect("delete");
    check_invariants(&es, "after delete word");
    assert_eq!(text(&es), "aaa\n\nbbb  ddd\n\neee\n");

    let plan = plan_source_patches(&base, &es).unwrap();
    assert!(plan.verified);
    assert_eq!(plan.patches.len(), 1, "patches: {:?}", plan.patches);
    verify_patches(base.bytes().as_slice(), &plan.patches, &current_bytes(&es)).unwrap();
}

/// 无实质变化 → 无补丁。
#[test]
fn no_change_yields_no_patches() {
    let fixture = "alpha beta\n\ngamma\n";
    let (base, mut es) = session(fixture);

    let v0 = es.byte_to_visual(ByteOffset(0)).unwrap();
    let beta_end = fixture.find("beta").unwrap() + 4;
    let v1 = es.byte_to_visual(ByteOffset(beta_end as u32)).unwrap();
    es.replace_selection(VisualRange::new(v0, v1), "alpha beta")
        .expect("no-op replace");
    let plan = plan_source_patches(&base, &es).unwrap();
    assert!(plan.verified);
    assert!(plan.patches.is_empty(), "patches: {:?}", plan.patches);
}

/// 输入转义：* ` # [ ] \ 等只作字面文本，不产生结构。
#[test]
fn typed_input_is_escaped() {
    let (_, mut es) = session("hello world\n");

    let pos = es.byte_to_visual(ByteOffset(5)).unwrap(); // "hello| world"
    es.insert_text(pos, "*").expect("insert *");
    check_invariants(&es, "after insert *");
    assert!(text(&es).contains("\\*"), "text: {:?}", text(&es));

    let pos = es.byte_to_visual(ByteOffset(5)).unwrap();
    es.insert_text(pos, "`code`").expect("insert code");
    check_invariants(&es, "after insert backticks");
    assert!(text(&es).contains("\\`code\\`"), "text: {:?}", text(&es));

    let pos = es.byte_to_visual(ByteOffset(5)).unwrap();
    es.insert_text(pos, "[x]").expect("insert brackets");
    check_invariants(&es, "after insert brackets");
    assert!(text(&es).contains("\\[x\\]"), "text: {:?}", text(&es));

    // 转义函数本身。
    assert_eq!(escape_text("a*b"), "a\\*b");
    assert_eq!(escape_text("**"), "\\*\\*");
    assert_eq!(escape_text("\\"), "\\\\");
    assert_eq!(escape_text("1."), "1\\.");
    assert_eq!(escape_text("3.14"), "3\\.14");
    assert_eq!(escape_text("> quote"), "\\> quote");
    assert_eq!(escape_text("# h"), "\\# h");
    assert_eq!(escape_text("- x"), "\\- x");
    assert_eq!(escape_text("+ x"), "\\+ x");
    assert_eq!(escape_text("| a | b |"), "\\| a \\| b \\|");
}

/// replace_selection 的输入同样转义。
#[test]
fn replacement_input_is_escaped() {
    let (_, mut es) = session("hello world\n");
    let w0 = es.byte_to_visual(ByteOffset(6)).unwrap();
    let w1 = es.byte_to_visual(ByteOffset(11)).unwrap();
    es.replace_selection(VisualRange::new(w0, w1), "**bold**")
        .expect("replace with markup");
    check_invariants(&es, "after markup replacement");
    assert_eq!(text(&es), "hello \\*\\*bold\\*\\*\n");
}

/// 输入含换行：typed 错误，状态不变（段内换行用 split_block）。
#[test]
fn newline_input_rejected() {
    let (_, mut es) = session("ab\n");
    let pos = es.byte_to_visual(ByteOffset(1)).unwrap();
    let before = current_bytes(&es);
    let err = es.insert_text(pos, "\n").expect_err("must reject newline");
    assert_eq!(err, EditError::NewlineInInput);
    assert_eq!(current_bytes(&es), before);

    let err = es
        .replace_selection(VisualRange::new(pos, pos), "x\r\n")
        .expect_err("must reject CRLF");
    assert_eq!(err, EditError::NewlineInInput);
    assert_eq!(current_bytes(&es), before);
}

/// 拆分块：段落 → 两段，补丁为纯插入 \n\n。
#[test]
fn split_block_patch_covers_both_paragraphs() {
    let fixture = "abcdef\n";
    let (base, mut es) = session(fixture);
    let split_at = es.byte_to_visual(ByteOffset(3)).unwrap(); // "abc|def"
    es.split_block(split_at).expect("split");
    check_invariants(&es, "after split");
    assert_eq!(text(&es), "abc\n\ndef\n");

    let plan = plan_source_patches(&base, &es).unwrap();
    assert!(plan.verified);
    // 无锚点（原叶已改）→ 整篇区域，裁剪后为光标处纯插入。
    assert_eq!(plan.patches.len(), 1, "patches: {:?}", plan.patches);
    let (ps, pe) = patch_bytes(&plan.patches[0]);
    assert_eq!((ps, pe), (3, 3));
    assert_eq!(plan.patches[0].replacement, "\n\n");
    verify_patches(base.bytes().as_slice(), &plan.patches, &current_bytes(&es)).unwrap();
}

/// 合并块：两段并一段。
#[test]
fn join_blocks_patch_single_block() {
    let fixture = "aaa\n\nbbb ccc\n";
    let (base, mut es) = session(fixture);
    let at = es.byte_to_visual(ByteOffset(1)).unwrap();
    es.join_blocks(at).expect("join");
    check_invariants(&es, "after join");
    assert_eq!(text(&es), "aaa\nbbb ccc\n");

    let plan = plan_source_patches(&base, &es).unwrap();
    assert!(plan.verified);
    // 无锚点 → 整篇区域，裁剪后为删除第二个 \n 的纯删除。
    assert_eq!(plan.patches.len(), 1, "patches: {:?}", plan.patches);
    let (ps, pe) = patch_bytes(&plan.patches[0]);
    assert_eq!((ps, pe), (4, 5));
    assert_eq!(plan.patches[0].replacement, "");
    verify_patches(base.bytes().as_slice(), &plan.patches, &current_bytes(&es)).unwrap();
}

/// toggle 标记：包裹后再还原，补丁范围保持最小。
#[test]
fn toggle_bold_patch_minimal() {
    let fixture = "one two three\n";
    let (base, mut es) = session(fixture);
    let two_start = fixture.find("two").unwrap();
    let v0 = es.byte_to_visual(ByteOffset(two_start as u32)).unwrap();
    let v1 = es
        .byte_to_visual(ByteOffset((two_start + 3) as u32))
        .unwrap();
    es.toggle_mark(MarkKind::Bold, VisualRange::new(v0, v1))
        .expect("wrap bold");
    check_invariants(&es, "after wrap bold");
    assert_eq!(text(&es), "one **two** three\n");

    let plan = plan_source_patches(&base, &es).unwrap();
    assert!(plan.verified);
    assert_eq!(plan.patches.len(), 1, "patches: {:?}", plan.patches);
    let (ps, pe) = patch_bytes(&plan.patches[0]);
    // 裁剪后只覆盖内层文本，标记不进补丁。
    assert_eq!((ps, pe), (4, 7));
    assert_eq!(plan.patches[0].replacement, "**two**");
    verify_patches(base.bytes().as_slice(), &plan.patches, &current_bytes(&es)).unwrap();
}

/// 吞字数=0 的输入（粘贴空串）作为删除；越界位置报错不改状态。
#[test]
fn empty_replacement_is_delete() {
    let (_, mut es) = session("x yz\n");
    // 空串替换等价于删除。
    let v0 = es.byte_to_visual(ByteOffset(0)).unwrap();
    let v1 = es.byte_to_visual(ByteOffset(1)).unwrap();
    es.replace_selection(VisualRange::new(v0, v1), "")
        .expect("empty replace = delete");
    assert_eq!(text(&es), " yz\n");
    check_invariants(&es, "after empty replace");
}

/// 越界 / 非叶位置：typed 错误，状态不变。
#[test]
fn out_of_bounds_is_typed_error() {
    let (_, mut es) = session("abc\n");
    let before = current_bytes(&es);
    let too_far = VisualPos(es.visual_len() + 5);
    let err = es
        .insert_text(too_far, "x")
        .expect_err("must fail with typed error");
    assert_eq!(err, EditError::PositionOutOfBounds);
    assert_eq!(current_bytes(&es), before);
}

/// 从叶位置推导的字节与视觉位置一致。
#[test]
fn leaf_positions_map_consistently() {
    let (_, es) = session("hello world\n");
    let poses = leaf_positions(&es, 0);
    assert!(!poses.is_empty());
    for v in poses {
        let b = byte_of(&es, v);
        assert_eq!(es.byte_to_visual(b), Some(v));
        assert_eq!(es.visual_to_byte(v), Some(b));
    }
}
