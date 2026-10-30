use std::{fs, path::PathBuf};

use project::{AppDataPaths, FileProjection, IndexAlias, IndexArea, IndexBatch, IndexStore};
use rusqlite::Connection;
use uuid::Uuid;

fn temp_directory(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("ohmywork-index-{name}-{}", Uuid::new_v4()));
    fs::create_dir(&path).expect("create test directory");
    path
}

fn projection(path: &str, generation: u64, aliases: &[(&str, IndexArea)]) -> FileProjection {
    let mut file = FileProjection::new(path, generation);
    file.content_hash = vec![generation as u8; 32];
    file.path_fold = path.to_owned();
    file.title_fold = "标题".to_owned();
    file.body_fold = "正文".to_owned();
    file.aliases = aliases
        .iter()
        .map(|(logical, area)| IndexAlias::new(*logical, *area, false))
        .collect();
    file
}

#[test]
fn migration_reopen_uses_wal_and_keeps_settings_separate() {
    let root = temp_directory("migration");
    let app_data = AppDataPaths::from_base(root.join("LocalAppData")).expect("app data");
    app_data.ensure_root().expect("create app data");
    fs::write(app_data.settings_file(), "settings-survive-index-rebuild").expect("settings");

    let store = IndexStore::open_app_data(&app_data).expect("open index");
    let stats = store.stats().expect("stats");
    assert_eq!(stats.journal_mode.to_ascii_lowercase(), "wal");
    assert!(stats.foreign_keys);
    assert_eq!(stats.busy_timeout_ms, 2_000);
    assert_eq!(stats.cache_size, -32_768);
    drop(store);

    let reopened = IndexStore::open_app_data(&app_data).expect("reopen index");
    assert_eq!(reopened.stats().expect("reopened stats").files, 0);
    drop(reopened);
    assert_eq!(
        fs::read_to_string(app_data.settings_file()).expect("settings"),
        "settings-survive-index-rebuild"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn one_file_can_have_multiple_aliases_and_fts_rowid_is_file_id() {
    let root = temp_directory("aliases");
    let store = IndexStore::open(root.join("index.sqlite")).expect("open index");
    let mut batch = IndexBatch::new(1);
    batch.push(projection(
        r"C:\workspace\Atlas\note.md",
        1,
        &[
            ("Atlas/note.md", IndexArea::Atlas),
            ("Efforts/note.md", IndexArea::Efforts),
            ("Archive/note.md", IndexArea::Archive),
        ],
    ));
    let report = store.ingest(batch).expect("ingest");
    assert_eq!(report.accepted, 1);
    assert_eq!(store.stats().expect("stats").aliases, 3);
    let record = store
        .file_for_alias("Archive/note.md")
        .expect("lookup")
        .expect("record");
    assert_eq!(record.aliases.len(), 3);
    assert_eq!(record.aliases[2].area, IndexArea::Efforts);
    let id = record.id;
    drop(store);

    let connection = Connection::open(root.join("index.sqlite")).expect("open sqlite");
    let long_rowid: i64 = connection
        .query_row("SELECT rowid FROM search_long", [], |row| row.get(0))
        .expect("long fts row");
    let short_rowid: i64 = connection
        .query_row("SELECT rowid FROM search_short", [], |row| row.get(0))
        .expect("short fts row");
    assert_eq!(long_rowid, id);
    assert_eq!(short_rowid, id);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn stale_generation_is_ignored_and_complete_scan_removes_missing_aliases() {
    let root = temp_directory("generation");
    let store = IndexStore::open(root.join("index.sqlite")).expect("open index");
    let mut first = IndexBatch::new(2);
    first.push(projection(
        "C:\\file.md",
        2,
        &[("Atlas/file.md", IndexArea::Atlas)],
    ));
    store.ingest(first).expect("first ingest");

    let mut stale = IndexBatch::new(1);
    stale.push(projection(
        "C:\\file.md",
        1,
        &[("Efforts/file.md", IndexArea::Efforts)],
    ));
    let report = store.ingest(stale).expect("stale ingest");
    assert_eq!(report.accepted, 0);
    assert_eq!(report.skipped_stale, 1);
    store
        .complete_generation(2, ["Atlas/file.md".to_owned()])
        .expect("complete");
    assert!(
        store
            .file_for_alias("Atlas/file.md")
            .expect("old alias")
            .is_some()
    );
    assert!(
        store
            .file_for_alias("Efforts/file.md")
            .expect("stale alias")
            .is_none()
    );
    store
        .complete_generation(3, std::iter::empty::<String>())
        .expect("delete file");
    assert_eq!(store.stats().expect("empty stats").files, 0);
    drop(store);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn invalid_batch_is_atomic_and_corrupt_index_rebuild_does_not_touch_settings() {
    let root = temp_directory("rebuild");
    let index_path = root.join("index.sqlite");
    let store = IndexStore::open(&index_path).expect("open index");
    let mut good = IndexBatch::new(1);
    good.push(projection(
        "C:\\good.md",
        1,
        &[("Atlas/good.md", IndexArea::Atlas)],
    ));
    store.ingest(good).expect("good ingest");
    let mut invalid = IndexBatch::new(2);
    let mut bad = projection("C:\\bad.md", 2, &[("Atlas/bad.md", IndexArea::Atlas)]);
    bad.content_hash.clear();
    invalid.push(bad);
    assert!(store.ingest(invalid).is_err());
    assert_eq!(store.stats().expect("stats").files, 1);
    drop(store);

    let app_data = AppDataPaths::from_base(root.join("app-data")).expect("app data");
    app_data.ensure_root().expect("app data root");
    fs::write(app_data.settings_file(), "keep").expect("settings");
    fs::write(&index_path, b"not sqlite").expect("corrupt index");
    let rebuilt = IndexStore::open(&index_path).expect("rebuild index");
    assert_eq!(rebuilt.stats().expect("rebuilt stats").files, 0);
    drop(rebuilt);
    assert_eq!(
        fs::read_to_string(app_data.settings_file()).expect("settings"),
        "keep"
    );
    let _ = fs::remove_dir_all(root);
}

