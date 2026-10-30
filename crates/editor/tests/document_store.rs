use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use editor::{
    EditorSurface, PageCommand, PageDocument, TaskAnchor, TaskFieldUpdate, parse, task::task_items,
    documents::{DocumentStore, TaskIntent},
};
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use project::WriteQueue;

static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

struct TempPage(PathBuf);
impl Drop for TempPage {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn temp_page() -> (TempPage, PathBuf) {
    let directory = std::env::temp_dir().join(format!(
        "ohmywork-editor-store-{}-{}",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed),
    ));
    fs::create_dir(&directory).expect("temporary directory");
    let path = directory.join("note.md");
    (TempPage(directory), path)
}

fn new_store(cx: &mut TestAppContext) -> Entity<DocumentStore> {
    cx.update(|cx| cx.new(|cx| DocumentStore::new(WriteQueue::new(2), cx)))
}

fn open_store(cx: &mut TestAppContext, path: &PathBuf) -> (Entity<DocumentStore>, Entity<PageDocument>) {
    let store = new_store(cx);
    cx.update(|cx| store.update(cx, |store, cx| store.open(path.clone(), cx)));
    cx.run_until_parked();
    let page = cx.update(|cx| store.read(cx).get(path).expect("opened Page"));
    (store, page)
}

#[gpui_kit::test]
fn document_store_opens_edits_and_saves_the_page(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let (_directory, path) = temp_page();
    fs::write(&path, b"first\n\nsecond").expect("initial Page");
    let (store, page) = open_store(cx, &path);
    let (node, revision) = cx.update(|cx| {
        let snapshot = page.read(cx).snapshot().expect("snapshot");
        let block = snapshot.blocks.iter().find(|block| block.visible.text == "second").expect("second block");
        (block.node, snapshot.revision())
    });
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.apply(PageCommand::ReplaceBlock { node, revision, value: "Xsecond".into() }, cx)
                .expect("edit Page")
        })
    });
    assert!(cx.update(|cx| page.read(cx).is_dirty()));
    cx.update(|cx| {
        store.update(cx, |store, cx| store.save(path.clone(), cx)).expect("submit save")
    });
    cx.run_until_parked();
    assert_eq!(fs::read(&path).expect("saved Page"), b"first\n\nXsecond");
    assert!(!cx.update(|cx| page.read(cx).is_dirty()));
}

#[gpui_kit::test]
fn task_commit_writes_only_task_and_keeps_page_draft_through_undo(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let (_directory, path) = temp_page();
    let original = b"Body\n\n- [ ] task [priority:: 2]\n";
    fs::write(&path, original).expect("initial Page");
    let (store, page) = open_store(cx, &path);
    let (body_node, revision) = cx.update(|cx| {
        let snapshot = page.read(cx).snapshot().expect("snapshot");
        let body = snapshot.blocks.iter().find(|block| block.visible.text == "Body").expect("body block");
        (body.node, snapshot.revision())
    });
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.apply(PageCommand::ReplaceBlock { node: body_node, revision, value: "Body draft".into() }, cx)
                .expect("edit draft")
        })
    });

    let snapshot = project::read_snapshot(&path).expect("disk baseline");
    let baseline = parse(&snapshot.bytes).expect("parse baseline");
    let item = task_items(baseline.tree()).into_iter().next().expect("Task");
    let anchor = TaskAnchor {
        item_range: baseline.tree().node(item.node).source.expect("Task range"),
        checkbox_range: item.checkbox_range,
    };
    let intent = TaskIntent {
        path: path.clone(),
        version: snapshot.version,
        anchor,
        update: TaskFieldUpdate::SetPriority { value: Some(9) },
    };
    cx.update(|cx| {
        store.update(cx, |store, cx| store.commit_task(intent, cx)).expect("submit Task")
    });
    cx.run_until_parked();

    let committed = b"Body\n\n- [ ] task [priority:: 9]\n";
    let working = b"Body draft\n\n- [ ] task [priority:: 9]\n";
    assert_eq!(fs::read(&path).expect("Task saved to disk"), committed);
    assert_eq!(cx.update(|cx| page.read(cx).bytes().expect("working bytes")), working);
    assert!(cx.update(|cx| page.read(cx).is_dirty()));

    cx.update(|cx| {
        page.update(cx, |page, cx| page.apply(PageCommand::Undo, cx).expect("undo body edit"))
    });
    assert_eq!(cx.update(|cx| page.read(cx).bytes().expect("undo bytes")), committed);
    assert_eq!(fs::read(&path).expect("disk unchanged by undo"), committed);
}

#[gpui_kit::test]
async fn external_replacement_reloads_page_and_rejects_old_revision(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let (_directory, path) = temp_page();
    fs::write(&path, b"original\n\nbody").expect("initial Page");
    let (store, page) = open_store(cx, &path);
    let (node, old_revision) = cx.update(|cx| {
        let snapshot = page.read(cx).snapshot().expect("snapshot");
        let body = snapshot.blocks.iter().find(|block| block.visible.text == "body").expect("body");
        (body.node, snapshot.revision())
    });
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.apply(PageCommand::ReplaceBlock { node, revision: old_revision, value: "local draft".into() }, cx)
                .expect("local edit")
        })
    });
    use gpui_kit::{WindowOptions, test::{TestAppContextExt as _, TestWindowExt as _}};
    let window_handle = cx.update(|cx| {
        let view = cx.new(|cx| EditorSurface::new(page.clone(), store.clone(), cx));
        gpui_kit::open_window(WindowOptions::default(), cx, {
            let view = view.clone();
            move |_, _| view
        })
        .expect("open Page window")
        .0
    });
    cx.update_window(window_handle, |_, window, cx| window.render_frame(cx))
        .expect("render Page window");
    fs::write(&path, b"external\n\nreplacement").expect("external replacement");
    cx.wait_for(window_handle.into(), Duration::from_secs(2), |_, cx| {
        page.read(cx)
            .bytes()
            .is_ok_and(|bytes| bytes == b"external\n\nreplacement")
    })
    .await;
    assert_eq!(cx.update(|cx| page.read(cx).bytes().expect("reloaded Page")), b"external\n\nreplacement");
    assert!(!cx.update(|cx| page.read(cx).is_dirty()));
    let stale = cx.update(|cx| {
        page.update(cx, |page, cx| page.apply(PageCommand::ReplaceBlock { node, revision: old_revision, value: "stale".into() }, cx))
    });
    assert!(matches!(stale, Err(editor::PageError::StaleRevision)));
}
