use std::{fs, path::PathBuf};

use project::{
    FileProjection, IndexAlias, IndexArea, IndexBatch, IndexStore, SearchMatchKind, SearchRequest,
};
use rusqlite::Connection;
use uuid::Uuid;

fn temp_directory(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("ohmywork-search-{name}-{}", Uuid::new_v4()));
    fs::create_dir(&path).expect("create test directory");
    path
}

fn projection(
    path: &PathBuf,
    generation: u64,
    aliases: &[(&str, IndexArea, bool)],
    title: &str,
    body: &str,
) -> FileProjection {
    let mut projection = FileProjection::new(path.to_string_lossy(), generation);
    projection.content_hash = vec![generation as u8; 32];
    projection.path_fold = aliases
        .iter()
        .map(|(alias, _, _)| *alias)
        .collect::<Vec<_>>()
        .join("\n");
    projection.title_fold = title.to_owned();
    projection.body_fold = body.to_owned();
    projection.aliases = aliases
        .iter()
        .map(|(alias, area, excluded)| IndexAlias::new(*alias, *area, *excluded))
        .collect();
    projection
}

fn query(store: &IndexStore, text: &str) -> Vec<String> {
    store
        .search(SearchRequest::new(text, false, 0, 50))
        .expect("search")
        .results
        .into_iter()
        .map(|result| result.logical_path)
        .collect()
}

#[test]
fn normalized_short_long_and_special_queries_return_exact_visible_results() {
    let root = temp_directory("queries");
    let note = root.join("note.md");
    let readme = root.join("readme.md");
    let archive = root.join("archive.md");
    let ignored = root.join("ignored.md");
    fs::write(
        &note,
        "# 项目方案\n\n中文 中 ＡＢＣ e\u{301} 😀 \"引号\" 100% NEAR OR\n",
    )
    .expect("note");
    fs::write(&readme, "# Readme\n\nEnglish body\n").expect("readme");
    fs::write(&archive, "# 归档中文\n").expect("archive");
    fs::write(&ignored, "# 忽略中文\n").expect("ignored");

    let index_path = root.join("index.sqlite");
    let store = IndexStore::open(&index_path).expect("open index");
    let mut batch = IndexBatch::new(1);
    batch.push(projection(
        &note,
        1,
        &[
            ("Atlas/中文.md", IndexArea::Atlas, false),
            ("Efforts/同目标.md", IndexArea::Efforts, false),
            ("Archive/中文.md", IndexArea::Archive, false),
        ],
        "项目方案",
        "中文 中 ＡＢＣ e\u{301} 😀 \"引号\" 100% NEAR OR",
    ));
    batch.push(projection(
        &readme,
        1,
        &[("Atlas/readme.md", IndexArea::Atlas, false)],
        "Readme",
        "English body",
    ));
    batch.push(projection(
        &archive,
        1,
        &[("Archive/old.md", IndexArea::Archive, false)],
        "归档中文",
        "归档中文",
    ));
    batch.push(projection(
        &ignored,
        1,
        &[("Atlas/ignored.md", IndexArea::Atlas, true)],
        "忽略中文",
        "忽略中文",
    ));
    store.ingest(batch).expect("ingest");
    store
        .complete_generation(1, [
            "Atlas/中文.md".to_owned(),
            "Efforts/同目标.md".to_owned(),
            "Archive/中文.md".to_owned(),
            "Atlas/readme.md".to_owned(),
            "Archive/old.md".to_owned(),
            "Atlas/ignored.md".to_owned(),
        ])
        .expect("complete");

    assert_eq!(query(&store, "中"), vec!["Atlas/中文.md"]);
    assert_eq!(query(&store, "中文"), vec!["Atlas/中文.md"]);
    assert_eq!(query(&store, "项目方案"), vec!["Atlas/中文.md"]);
    assert_eq!(query(&store, "README"), vec!["Atlas/readme.md"]);
    assert_eq!(query(&store, "ＡＢＣ"), vec!["Atlas/中文.md"]);
    assert_eq!(query(&store, "é"), vec!["Atlas/中文.md"]);
    assert_eq!(query(&store, "😀"), vec!["Atlas/中文.md"]);
    assert_eq!(query(&store, "\"引号\""), vec!["Atlas/中文.md"]);
    assert_eq!(query(&store, "%"), vec!["Atlas/中文.md"]);
    assert_eq!(query(&store, "NEAR OR"), vec!["Atlas/中文.md"]);
    assert_eq!(query(&store, "归档中文"), Vec::<String>::new());

    let archive_results = store
        .search(SearchRequest::new("归档中文", true, 0, 50))
        .expect("archive search");
    assert_eq!(
        archive_results
            .results
            .iter()
            .map(|result| result.logical_path.as_str())
            .collect::<Vec<_>>(),
        ["Archive/old.md"]
    );
    assert_eq!(archive_results.results[0].match_kind, SearchMatchKind::Title);
    assert_eq!(
        archive_results.results[0].snippet.as_deref(),
        Some("# 归档中文\n")
    );

    let connection = Connection::open(&index_path).expect("inspect index");
    let grams: String = connection
        .query_row(
            "SELECT grams FROM search_short WHERE rowid = 1",
            [],
            |row| row.get(0),
        )
        .expect("short grams");
    assert!(grams.split_ascii_whitespace().any(|token| token == "u4e2d"));
    assert!(!grams.contains("中文"));

    let first_page = store
        .search(SearchRequest::new("中", true, 0, 1))
        .expect("first page");
    assert_eq!(first_page.results.len(), 1);
    assert!(first_page.has_more);
    let second_page = store
        .search(SearchRequest::new("中", true, 1, 1))
        .expect("second page");
    assert_eq!(second_page.results.len(), 1);
    assert_eq!(second_page.results[0].logical_path, "Archive/old.md");

    let cancel = project::SearchRequest::new("中文", false, 0, 50);
    cancel.cancel.cancel();
    assert!(matches!(
        store.search(cancel),
        Err(project::IndexError::SearchCancelled)
    ));

    drop(connection);
    drop(store);
    let _ = fs::remove_file(&index_path);
    let _ = fs::remove_file(format!("{}-wal", index_path.display()));
    let _ = fs::remove_file(format!("{}-shm", index_path.display()));
    let _ = fs::remove_dir_all(root);
}
