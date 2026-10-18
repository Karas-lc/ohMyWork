//! Task 局部提交测试（计划14）。
//!
//! 验证：只改目标字段的 baseline patch；不碰全文。
//! 缺失字段插入和既有字段替换均须保持任务结构与相邻原文。

use omw_document::{
    ByteOffset, ByteRange, DocumentSession, TaskAnchor, TaskCommitError, TaskDateField,
    TaskFieldUpdate, apply_task_update_to_baseline, locate_task_by_anchor, parse,
    task::{parse_task, task_items},
};

fn doc(src: &str) -> DocumentSession {
    let d = parse(src.as_bytes()).expect("fixture parse");
    d
}

#[test]
fn locate_by_checkbox_and_item_start() {
    let src = "- [ ] foo [priority:: 3] [start:: 2026-01-01]\n- [x] bar";
    let d = doc(src);
    let items = task_items(d.tree());
    assert_eq!(items.len(), 2);

    // checkbox 定位
    let cb0 = items[0].checkbox_range.expect("has checkbox");
    let a_cb = TaskAnchor {
        item_range: ByteRange {
            start: ByteOffset(0),
            end: ByteOffset(0),
        }, // dummy, will match cb first
        checkbox_range: Some(cb0),
    };
    let it = locate_task_by_anchor(&d, &a_cb).expect("locate by cb");
    assert!(it.checked == false);

    // item node source start 定位
    let node0 = d.tree().node(items[0].node);
    let item_src = node0.source.expect("item has source");
    let a_item = TaskAnchor {
        item_range: item_src,
        checkbox_range: None,
    };
    let it2 = locate_task_by_anchor(&d, &a_item).expect("locate by item start");
    assert_eq!(it2.node, items[0].node);
}

#[test]
fn apply_complete_toggle_changes_only_checkbox() {
    let src = "- [ ] task one [priority:: 1]\nmore text";
    let d = doc(src);
    let items = task_items(d.tree());
    let item = &items[0];
    let cb = item.checkbox_range.expect("cb");
    let anchor = TaskAnchor {
        item_range: d.tree().node(item.node).source.unwrap_or(ByteRange {
            start: ByteOffset(0),
            end: ByteOffset(0),
        }),
        checkbox_range: Some(cb),
    };

    // toggle to checked
    let newb =
        apply_task_update_to_baseline(&d, &anchor, &TaskFieldUpdate::Complete { checked: true })
            .expect("apply toggle");

    let s = String::from_utf8(newb).unwrap();
    assert!(s.contains("- [x] task one"));
    assert!(s.contains("[priority:: 1]")); // other field untouched
    assert!(s.contains("more text"));
    // original not mutated
    assert!(String::from_utf8_lossy(&d.bytes()).contains("- [ ] task one"));
}

#[test]
fn apply_priority_replace_only_that_field() {
    let src = "- [ ] prio [priority:: 2] rest";
    let d = doc(src);
    let items = task_items(d.tree());
    let item = &items[0];
    let task = parse_task(&d, item).expect("parse");
    let _pr = task
        .priority
        .as_ref()
        .map(|(_, r)| *r)
        .expect("has prio range");
    let anchor = TaskAnchor {
        item_range: d.tree().node(item.node).source.unwrap(),
        checkbox_range: item.checkbox_range,
    };

    let newb = apply_task_update_to_baseline(
        &d,
        &anchor,
        &TaskFieldUpdate::SetPriority { value: Some(9) },
    )
    .expect("set prio");

    let s = String::from_utf8(newb).unwrap();
    assert!(s.contains("[priority:: 9]"));
    assert!(!s.contains("[priority:: 2]"));
    assert!(s.contains("prio"));
}

#[test]
fn apply_date_set_and_remove_minimal() {
    let src = "- [ ] has date [due:: 2025-06-01] rest text";
    let d = doc(src);
    let items = task_items(d.tree());
    let item = &items[0];
    let anchor = TaskAnchor {
        item_range: d.tree().node(item.node).source.unwrap(),
        checkbox_range: item.checkbox_range,
    };

    // change due value
    let b1 = apply_task_update_to_baseline(
        &d,
        &anchor,
        &TaskFieldUpdate::SetDate {
            which: TaskDateField::Due,
            value: Some("2026-12-31".into()),
        },
    )
    .expect("set date");
    let s1 = String::from_utf8(b1).unwrap();
    assert!(s1.contains("[due:: 2026-12-31]"));
    assert!(!s1.contains("[due:: 2025-06-01]"));

    // remove due
    let d2 = doc(&s1);
    let items2 = task_items(d2.tree());
    let anchor2 = TaskAnchor {
        item_range: d2.tree().node(items2[0].node).source.unwrap(),
        checkbox_range: items2[0].checkbox_range,
    };
    let b3 = apply_task_update_to_baseline(
        &d2,
        &anchor2,
        &TaskFieldUpdate::SetDate {
            which: TaskDateField::Due,
            value: None,
        },
    )
    .expect("remove");
    let s3 = String::from_utf8(b3).unwrap();
    assert!(!s3.contains("[due::"));
    assert!(s3.contains("has date"));
}

#[test]
fn cannot_locate_missing_returns_error() {
    let src = "- [ ] only";
    let d = doc(src);
    let bad_anchor = TaskAnchor {
        item_range: ByteRange {
            start: ByteOffset(999),
            end: ByteOffset(1000),
        },
        checkbox_range: None,
    };
    let res = apply_task_update_to_baseline(
        &d,
        &bad_anchor,
        &TaskFieldUpdate::Complete { checked: true },
    );
    assert!(matches!(
        res,
        Err(TaskCommitError::CannotLocateUnique { .. })
    ));
}

#[test]
fn missing_fields_append_to_target_title_preserving_other_bytes() {
    for (update, field) in [
        (
            TaskFieldUpdate::SetPriority { value: Some(9) },
            "[priority:: 9]",
        ),
        (
            TaskFieldUpdate::SetDate {
                which: TaskDateField::Due,
                value: Some("2026-10-20".into()),
            },
            "[due:: 2026-10-20]",
        ),
    ] {
        let source =
            "\u{feff}Body stays.\r\n\r\n- [ ] 中文 task\r\n  continuation\r\n- [x] Other\r\n";
        let base = doc(source);
        let item = task_items(base.tree())[0];
        let anchor = TaskAnchor {
            item_range: base.tree().node(item.node).source.unwrap(),
            checkbox_range: item.checkbox_range,
        };
        let bytes = apply_task_update_to_baseline(&base, &anchor, &update).unwrap();
        assert_eq!(
            String::from_utf8(bytes.clone()).unwrap(),
            source.replace("中文 task\r\n", &format!("中文 task {field}\r\n"))
        );
        let parsed = parse(&bytes).unwrap();
        let tasks = task_items(parsed.tree());
        assert_eq!(tasks.len(), 2);
        assert!(!tasks[0].checked);
        assert!(tasks[1].checked);
    }
}

#[test]
fn committed_field_survives_undo_without_saving_dirty_body() {
    use omw_document::{EditSession, VisualRange};
    let source = "Body stays.\n\n- [ ] Work [priority:: 3]\n";
    let base = doc(source);
    let item = task_items(base.tree())[0];
    let mut working = EditSession::from_document(&base);
    let start = source.find('3').unwrap() as u32;
    working
        .replace_selection(
            VisualRange::new(
                working.byte_to_visual(ByteOffset(start)).unwrap(),
                working.byte_to_visual(ByteOffset(start + 1)).unwrap(),
            ),
            "2",
        )
        .unwrap();
    working
        .insert_text(working.byte_to_visual(ByteOffset(4)).unwrap(), " dirty")
        .unwrap();
    let update = TaskFieldUpdate::SetPriority { value: Some(9) };
    let mut merged = working
        .with_committed_task_field(item.node, &update)
        .unwrap();
    assert_eq!(
        String::from_utf8(merged.bytes()).unwrap(),
        source.replace("Body", "Body dirty").replace(":: 3", ":: 9")
    );
    merged.undo().unwrap();
    assert_eq!(
        String::from_utf8(merged.bytes()).unwrap(),
        source.replace(":: 3", ":: 9")
    );
    merged.undo().unwrap();
    assert_eq!(
        String::from_utf8(merged.bytes()).unwrap(),
        source.replace(":: 3", ":: 9")
    );
    merged.redo().unwrap();
    merged.redo().unwrap();
    assert_eq!(
        String::from_utf8(merged.bytes()).unwrap(),
        source.replace("Body", "Body dirty").replace(":: 3", ":: 9")
    );
}
