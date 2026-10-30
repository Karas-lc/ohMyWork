use std::{fs, path::PathBuf};

use workspace::indexing::index_workspace;
use project::{IndexArea, IndexStore, ScanOptions};
use rusqlite::Connection;
use uuid::Uuid;

fn temp_directory(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("ohmywork-index-{name}-{}", Uuid::new_v4()));
    fs::create_dir(&path).expect("create test directory");
    path
}

#[test]
fn workspace_ingest_projects_markdown_tasks_and_archive_area() {
    let root = temp_directory("workspace");
    fs::create_dir(root.join("Atlas")).expect("Atlas");
    fs::create_dir(root.join("Efforts")).expect("Efforts");
    fs::create_dir(root.join("Archive")).expect("Archive");
    fs::write(
        root.join("Atlas/note.md"),
        "# 标题\n\n- [ ] 做事 [start:: 2026-10-01] [due:: 2026-10-15] [priority:: 9]\n",
    )
    .expect("markdown");
    fs::write(root.join("Archive/old.md"), "# Archive\n").expect("archive markdown");
    let index_path = root.with_extension("index.sqlite");
    let store = IndexStore::open(&index_path).expect("open index");
    let result = index_workspace(&store, &root, ScanOptions::default()).expect("index workspace");
    assert_eq!(result.indexed_files, 2);
    let stats = store.stats().expect("stats");
    assert_eq!(stats.files, 2);
    assert_eq!(stats.tasks, 1);
    let record = store
        .file_for_alias("Atlas/note.md")
        .expect("lookup")
        .expect("note");
    assert_eq!(record.aliases[0].area, IndexArea::Atlas);
    drop(store);
    let connection = Connection::open(&index_path).expect("open sqlite");
    let task: (String, i64, Option<String>, Option<String>, Option<i64>) = connection
        .query_row(
            "SELECT title, completed, start_date, due_date, priority FROM tasks",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .expect("task projection");
    assert_eq!(task.0, "做事");
    assert_eq!(task.1, 0);
    assert_eq!(task.2.as_deref(), Some("2026-10-01"));
    assert_eq!(task.3.as_deref(), Some("2026-10-15"));
    assert_eq!(task.4, Some(9));
    drop(connection);
    let _ = fs::remove_file(&index_path);
    let _ = fs::remove_file(format!("{}-wal", index_path.display()));
    let _ = fs::remove_file(format!("{}-shm", index_path.display()));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn failed_scan_does_not_delete_existing_records() {
    let root = temp_directory("interrupted-scan");
    fs::create_dir(root.join("Atlas")).expect("Atlas");
    fs::write(root.join("Atlas/note.md"), "# note\n").expect("markdown");
    let index_path = root.with_extension("interrupted.sqlite");
    let store = IndexStore::open(&index_path).expect("open index");
    index_workspace(&store, &root, ScanOptions::default()).expect("initial scan");
    fs::remove_dir_all(&root).expect("remove workspace to interrupt scan");
    assert!(index_workspace(&store, &root, ScanOptions::default()).is_err());
    assert_eq!(store.stats().expect("stats").files, 1);
    drop(store);
    let _ = fs::remove_file(&index_path);
    let _ = fs::remove_file(format!("{}-wal", index_path.display()));
    let _ = fs::remove_file(format!("{}-shm", index_path.display()));
}
