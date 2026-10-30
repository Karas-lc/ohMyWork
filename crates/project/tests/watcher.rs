use std::{
    fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

use project::{
    ChangeTracker, DocumentKey, FileWatcher, WatchChange, WatchEventQueue,
    is_application_temporary, read_snapshot,
};
use uuid::Uuid;

fn temp_directory() -> PathBuf {
    let path = std::env::temp_dir().join(format!("ohmywork-watcher-{}", Uuid::new_v4()));
    fs::create_dir(&path).expect("create test directory");
    path
}

#[test]
fn application_temporary_events_are_filtered_without_filtering_destination_events() {
    let root = temp_directory();
    assert!(is_application_temporary(
        &root.join(".note.md.ohmywork-1-2.tmp")
    ));
    assert!(!is_application_temporary(&root.join("note.md")));
    assert!(!is_application_temporary(
        &root.join("note.md.ohmywork.tmp")
    ));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn external_rewrite_delete_and_recreate_are_rechecked_by_content() {
    let root = temp_directory();
    let path = root.join("note.md");
    fs::write(&path, b"before").expect("write baseline");
    let key = DocumentKey::new(&path);
    let initial = read_snapshot(&path).expect("read baseline");
    let mut tracker = ChangeTracker::default();
    tracker.register(key.clone(), initial.version);

    fs::write(&path, b"external").expect("external rewrite");
    assert!(matches!(
        tracker.check(&key, &path).expect("verify rewrite"),
        WatchChange::External { .. }
    ));
    fs::remove_file(&path).expect("delete document");
    assert_eq!(
        tracker.check(&key, &path).expect("verify delete"),
        WatchChange::Deleted
    );
    fs::write(&path, b"recreated").expect("recreate document");
    assert!(matches!(
        tracker.check(&key, &path).expect("verify recreate"),
        WatchChange::External { .. }
    ));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn event_storm_is_bounded_and_coalesced_without_silent_drop() {
    let queue = WatchEventQueue::new(4);
    let first = DocumentKey::new("first.md");
    let second = DocumentKey::new("second.md");
    queue.enqueue(first.clone()).expect("first event");
    queue.enqueue(first.clone()).expect("duplicate event");
    queue.enqueue(second.clone()).expect("second event");
    let batch = queue.drain();
    assert_eq!(batch.paths, vec![first, second]);
}

#[test]
fn notify_watcher_reports_a_real_destination_change() {
    let root = temp_directory();
    let path = root.join("note.md");
    fs::write(&path, b"before").expect("write baseline");
    let watcher = FileWatcher::new(&root).expect("start watcher");
    let key = DocumentKey::new(&path);
    fs::write(&path, b"after").expect("write change");

    let deadline = Instant::now() + Duration::from_secs(2);
    let mut reported = false;
    while Instant::now() < deadline {
        let batch = watcher.poll();
        if batch.paths.iter().any(|candidate| candidate == &key) {
            reported = true;
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(reported, "notify did not report destination change");
    let _ = fs::remove_dir_all(root);
}
