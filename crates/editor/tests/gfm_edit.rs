use editor::{BlockKind, EditSession, NodeKind, NodeId, VisualPos, VisualRange, parse};

fn session(source: &str) -> EditSession {
    EditSession::from_document(&parse(source.as_bytes()).expect("fixture parses"))
}

fn find(session: &EditSession, predicate: impl Fn(&NodeKind) -> bool) -> NodeId {
    session
        .live_node_ids()
        .into_iter()
        .find(|id| session.node_kind(*id).as_ref().is_some_and(&predicate))
        .expect("node exists")
}

#[test]
fn block_task_code_link_and_raw_edits_are_structural() {
    let mut edit = session(
        "plain\n\n- [ ] todo\n\n```rust\nold\n```\n\n[site](https://old.example)\n\n<div>keep</div>\n",
    );
    let paragraph = find(&edit, |kind| *kind == NodeKind::Paragraph);
    edit.set_block_type(
        paragraph,
        BlockKind::Heading {
            level: 2,
            setext: false,
        },
    )
    .expect("heading conversion");
    assert!(edit.bytes().starts_with(b"## plain"));

    let task = find(&edit, |kind| matches!(kind, NodeKind::TaskItem { .. }));
    edit.set_task_checked(task, true).expect("task toggle");
    assert!(edit.bytes().windows(6).any(|window| window == b"[x] to"));

    let code = find(&edit, |kind| matches!(kind, NodeKind::CodeBlock { .. }));
    edit.edit_code_content(code, "contains ``` safely")
        .expect("code edit");
    assert!(String::from_utf8_lossy(&edit.bytes()).contains("contains ``` safely"));

    let link = find(&edit, |kind| matches!(kind, NodeKind::Link { .. }));
    edit.set_link_target(link, "https://new.example")
        .expect("link target");
    assert!(String::from_utf8_lossy(&edit.bytes()).contains("https://new.example"));

    let raw = find(&edit, NodeKind::is_raw);
    edit.delete_node(raw).expect("raw delete");
    assert!(!String::from_utf8_lossy(&edit.bytes()).contains("<div>keep</div>"));
    edit.undo().expect("undo works");
    assert!(String::from_utf8_lossy(&edit.bytes()).contains("<div>keep</div>"));
    edit.assert_semantics_consistent()
        .expect("tree stays consistent");
}

#[test]
fn list_table_markdown_paste_and_undo_keep_valid_tree() {
    let original = "- one\n- two\n\n| a | b |\n|---|---|\n| c | d |\n";
    let mut edit = session(original);
    let list = find(&edit, |kind| matches!(kind, NodeKind::List { .. }));
    edit.insert_list_item(list, 1, "middle")
        .expect("insert list item");
    assert!(String::from_utf8_lossy(&edit.bytes()).contains("- middle"));

    let list = find(&edit, |kind| matches!(kind, NodeKind::List { .. }));
    let items = edit.children(list).expect("list children");
    edit.indent_list_item(items[1]).expect("indent item");
    edit.assert_semantics_consistent()
        .expect("indented list parses");

    let table = find(&edit, |kind| matches!(kind, NodeKind::Table { .. }));
    edit.edit_table_cell(table, 1, 1, "changed")
        .expect("cell edit");
    assert!(String::from_utf8_lossy(&edit.bytes()).contains("changed"));

    let end = VisualPos(edit.visual_len());
    edit.paste_markdown(VisualRange::new(end, end), "\n\n# pasted\n")
        .expect("markdown paste");
    assert!(String::from_utf8_lossy(&edit.bytes()).contains("# pasted"));
    while edit.history().undo_depth() > 0 {
        edit.undo().expect("undo step");
    }
    assert_eq!(String::from_utf8_lossy(&edit.bytes()), original);
}

#[test]
fn unchanged_siblings_keep_node_ids_and_raw_is_not_editable() {
    let mut edit = session("first\n\nsecond\n\n<script>raw</script>\n");
    let paragraphs = edit
        .live_node_ids()
        .into_iter()
        .filter(|id| edit.node_kind(*id) == Some(NodeKind::Paragraph))
        .collect::<Vec<_>>();
    let second = paragraphs[1];
    let first = paragraphs[0];
    edit.edit_node_text(first, "updated")
        .expect("edit paragraph");
    assert_eq!(edit.node_kind(second), Some(NodeKind::Paragraph));
    let raw = find(&edit, NodeKind::is_raw);
    assert!(edit.edit_node_text(raw, "unsafe").is_err());
    edit.assert_semantics_consistent()
        .expect("tree stays consistent");
}

#[test]
fn cross_block_delete_is_one_undoable_transaction() {
    let original = "one\n\ntwo\n\nthree\n";
    let mut edit = session(original);
    let start = edit.byte_to_visual(editor::ByteOffset(1)).unwrap();
    let end = edit.byte_to_visual(editor::ByteOffset(10)).unwrap();
    edit.delete_range(VisualRange::new(start, end))
        .expect("cross block delete");
    assert_eq!(String::from_utf8_lossy(&edit.bytes()), "othree\n");
    edit.undo().expect("undo delete");
    assert_eq!(String::from_utf8_lossy(&edit.bytes()), original);
}

#[test]
fn table_structure_and_inline_fence_operations_roundtrip() {
    let mut edit = session("| a | b |\n|---|---|\n| c | d |\n");
    let table = find(&edit, |kind| matches!(kind, NodeKind::Table { .. }));
    edit.insert_table_row(table, 2).expect("insert row");
    let table = find(&edit, |kind| matches!(kind, NodeKind::Table { .. }));
    edit.insert_table_column(table, 2).expect("insert column");
    let table = find(&edit, |kind| matches!(kind, NodeKind::Table { .. }));
    edit.delete_table_column(table, 2).expect("delete column");
    let table = find(&edit, |kind| matches!(kind, NodeKind::Table { .. }));
    edit.delete_table_row(table, 2).expect("delete row");
    edit.assert_semantics_consistent()
        .expect("table stays valid");

    let mut code = session("`old`\n");
    let inline = find(&code, |kind| matches!(kind, NodeKind::Code { .. }));
    code.edit_code_content(inline, "a`b").expect("inline code");
    assert!(String::from_utf8_lossy(&code.bytes()).contains("a`b"));
    code.assert_semantics_consistent()
        .expect("inline code stays valid");
}
