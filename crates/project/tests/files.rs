use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    thread,
};

use project::{Workspace, WorkspaceError};
use uuid::Uuid;

fn temp_directory(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("ohmywork-{name}-{}", Uuid::new_v4()));
    fs::create_dir(&path).expect("create test directory");
    path
}

fn entries(path: &Path) -> Vec<String> {
    let mut entries = fs::read_dir(path)
        .expect("read test directory")
        .map(|entry| {
            entry
                .expect("read entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect::<Vec<_>>();
    entries.sort();
    entries
}

#[test]
fn initializes_empty_workspace_without_creating_project_objects() {
    let root = temp_directory("empty");
    let workspace = Workspace::initialize(&root).expect("initialize workspace");

    assert_eq!(
        entries(&root),
        vec![".ohmywork", "Archive", "Atlas", "Efforts"]
    );
    assert_eq!(
        fs::read_to_string(workspace.paths().id_file.clone())
            .expect("read id")
            .len(),
        37
    );
    assert!(
        workspace
            .paths()
            .efforts
            .join("普通项目")
            .parent()
            .is_some()
    );
    assert!(!root.join("projects.json").exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn preserves_existing_files_and_only_creates_missing_directories() {
    let root = temp_directory("partial");
    fs::create_dir(root.join("Atlas")).expect("create Atlas");
    fs::write(root.join("Atlas").join("existing.md"), "keep").expect("write content");

    let first = Workspace::initialize(&root).expect("initialize partial workspace");
    let id = fs::read_to_string(&first.paths().id_file).expect("read id");
    let second = Workspace::initialize(&root).expect("reopen workspace");

    assert_eq!(
        id,
        fs::read_to_string(second.paths().id_file.clone()).expect("read stable id")
    );
    assert_eq!(
        fs::read_to_string(root.join("Atlas/existing.md")).expect("read existing content"),
        "keep"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn rejects_partition_file_and_invalid_id_without_overwriting_content() {
    let root = temp_directory("conflict");
    fs::write(root.join("Atlas"), "user content").expect("create conflicting file");
    assert!(matches!(
        Workspace::initialize(&root),
        Err(WorkspaceError::EntryMustBeDirectory { .. })
    ));
    assert_eq!(
        fs::read_to_string(root.join("Atlas")).expect("read conflicting file"),
        "user content"
    );

    fs::remove_file(root.join("Atlas")).expect("remove conflicting file");
    fs::create_dir(root.join(".ohmywork")).expect("create metadata");
    fs::write(root.join(".ohmywork/id"), "not-a-uuid\n").expect("write invalid id");
    assert!(matches!(
        Workspace::initialize(&root),
        Err(WorkspaceError::InvalidId { .. })
    ));
    assert_eq!(
        fs::read_to_string(root.join(".ohmywork/id")).expect("read invalid id"),
        "not-a-uuid\n"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn concurrent_initialization_converges_on_one_uuid() {
    let root = Arc::new(temp_directory("concurrent"));
    let handles = (0..8)
        .map(|_| {
            let root = root.clone();
            thread::spawn(move || {
                Workspace::initialize(root.as_path()).map(|workspace| workspace.uuid())
            })
        })
        .collect::<Vec<_>>();
    let uuids = handles
        .into_iter()
        .map(|handle| {
            handle
                .join()
                .expect("join initializer")
                .expect("initialize concurrently")
        })
        .collect::<Vec<_>>();
    assert!(uuids.windows(2).all(|pair| pair[0] == pair[1]));
    let _ = fs::remove_dir_all(root.as_path());
}

#[test]
fn atomic_save_replaces_complete_file_and_reports_commit_version() {
    let root = temp_directory("atomic-success");
    let path = root.join("note.md");
    fs::write(&path, b"before").expect("write baseline");
    let baseline = project::read_snapshot(&path).expect("read baseline");

    let ack =
        project::atomic_write(&path, Some(&baseline.version), b"after", 7).expect("atomic save");

    assert_eq!(fs::read(&path).expect("read committed file"), b"after");
    assert_eq!(ack.request_revision, 7);
    assert_eq!(ack.bytes_written, 5);
    assert_eq!(ack.old_version, Some(baseline.version));
    assert_eq!(ack.new_version.len, 5);
    assert!(
        entries(&root)
            .iter()
            .all(|entry| !entry.contains(".ohmywork-"))
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn atomic_save_rejects_external_change_without_touching_disk() {
    let root = temp_directory("atomic-external");
    let path = root.join("note.md");
    fs::write(&path, b"before").expect("write baseline");
    let baseline = project::read_snapshot(&path).expect("read baseline");
    fs::write(&path, b"external").expect("write external version");

    let error = project::atomic_write(&path, Some(&baseline.version), b"local", 9)
        .expect_err("external change must win");
    assert!(matches!(
        error,
        project::AtomicWriteError::ExternalChanged { .. }
    ));
    assert_eq!(fs::read(&path).expect("read external file"), b"external");
    assert!(
        entries(&root)
            .iter()
            .all(|entry| !entry.contains(".ohmywork-"))
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn atomic_save_creates_new_file_without_replacing_unknown_target() {
    let root = temp_directory("atomic-new");
    let path = root.join("new.md");

    let ack = project::atomic_write(&path, None, b"new-content", 3)
        .expect("create new file atomically");

    assert_eq!(fs::read(&path).expect("read new file"), b"new-content");
    assert_eq!(ack.old_version, None);
    assert_eq!(ack.bytes_written, 11);
    assert!(
        entries(&root)
            .iter()
            .all(|entry| !entry.contains(".ohmywork-"))
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn new_file_save_does_not_overwrite_a_created_target() {
    let root = temp_directory("atomic-new-race");
    let path = root.join("new.md");
    fs::write(&path, b"created-by-other-process").expect("create competing target");

    let error = project::atomic_write(&path, None, b"local", 1)
        .expect_err("new target must not be overwritten");
    assert!(matches!(
        error,
        project::AtomicWriteError::ExternalChanged { .. }
    ));
    assert_eq!(
        fs::read(&path).expect("read competing target"),
        b"created-by-other-process"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn save_queue_serializes_same_document_and_keeps_save_requests() {
    let root = temp_directory("queue-serial");
    let path = root.join("note.md");
    fs::write(&path, b"one").expect("write baseline");
    let baseline = project::read_snapshot(&path).expect("read baseline");
    let queue = project::WriteQueue::new(2);

    let first = queue
        .submit_save(
            project::DocumentKey::new(&path),
            &path,
            Some(baseline.version.clone()),
            b"two".to_vec(),
            1,
        )
        .expect("submit first save");
    let second = queue
        .submit_save(
            project::DocumentKey::new(&path),
            &path,
            Some(baseline.version),
            b"three".to_vec(),
            2,
        )
        .expect("submit second save");
    let first_ack = first
        .recv()
        .expect("first save result")
        .expect("first save succeeds");
    let second_error = second
        .recv()
        .expect("second save result")
        .expect_err("stale second save must not overwrite first");

    assert_eq!(fs::read(&path).expect("read final file"), b"two");
    assert_eq!(first_ack.request_revision, 1);
    assert!(matches!(
        second_error,
        project::AtomicWriteError::ExternalChanged { .. }
    ));
    queue.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn failed_temporary_creation_leaves_no_partial_file() {
    let root = temp_directory("atomic-temp-failure");
    let path = root.join("missing-parent").join("note.md");

    let error = project::atomic_write(&path, None, b"content", 4)
        .expect_err("missing parent must fail before commit");
    assert!(matches!(error, project::AtomicWriteError::Io { .. }));
    assert_eq!(entries(&root), Vec::<String>::new());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn file_tree_uses_only_ohmyworkignore_and_honors_negation() {
    let root = temp_directory("scan-ignore");
    let workspace = Workspace::initialize(&root).expect("initialize workspace");
    fs::write(root.join(".gitignore"), "visible.md\n").expect("write ignored git rules");
    fs::write(root.join(".ohmyworkignore"), "secret/*\n!secret/keep.md\n")
        .expect("write ohmywork rules");
    fs::write(root.join("visible.md"), "visible").expect("write visible file");
    fs::create_dir(root.join("secret")).expect("create secret directory");
    fs::write(root.join("secret/keep.md"), "keep").expect("write negated file");
    fs::write(root.join("secret/drop.md"), "drop").expect("write ignored file");
    fs::write(workspace.paths().archive.join("browsable.md"), "archive")
        .expect("write archive file");

    let report = project::scan_workspace(&root, Default::default()).expect("scan workspace");
    let logical = report
        .entries()
        .iter()
        .map(|entry| entry.logical_path().to_string_lossy().replace('\\', "/"))
        .collect::<Vec<_>>();
    assert!(
        logical.contains(&".gitignore".to_owned()),
        ".gitignore is not a rule source"
    );
    assert!(logical.contains(&"secret/keep.md".to_owned()));
    assert!(logical.contains(&"Archive/browsable.md".to_owned()));
    assert!(!logical.contains(&"secret/drop.md".to_owned()));
    assert!(!logical.iter().any(|path| path.starts_with(".ohmywork/")));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn file_tree_keeps_unicode_and_reports_single_path_failures() {
    let root = temp_directory("scan-unicode");
    let workspace = Workspace::initialize(&root).expect("initialize workspace");
    let nested = workspace.paths().atlas.join("中文").join("很长的目录名称");
    fs::create_dir_all(&nested).expect("create unicode path");
    let file = nested.join("页面.md");
    fs::write(&file, "内容").expect("write unicode file");

    let report = project::scan_workspace(&root, Default::default()).expect("scan workspace");
    assert!(
        report
            .entries()
            .iter()
            .any(|entry| entry.target_path() == file)
    );
    assert!(report.diagnostics().is_empty());

    let missing =
        project::scan_directory_children(&root, Path::new("Atlas/missing"), Default::default());
    assert!(matches!(missing, Err(project::ScanError::Root { .. })));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn file_operations_leave_existing_content_on_failure_and_do_not_touch_links() {
    let root = temp_directory("file-ops");
    let workspace = Workspace::initialize(&root).expect("initialize workspace");
    fs::write(workspace.paths().atlas.join("existing.md"), "original").expect("write original");

    let error = project::create_markdown(&root, "Atlas/existing.md")
        .expect_err("create-new must reject an existing file");
    assert!(matches!(error, project::FileOperationError::Io { .. }));
    assert_eq!(
        fs::read_to_string(workspace.paths().atlas.join("existing.md")).expect("read original"),
        "original"
    );

    project::create_markdown(&root, "Atlas/new.md").expect("create markdown");
    project::rename_entry(&root, "Atlas/new.md", "renamed.md").expect("rename markdown");
    assert!(workspace.paths().atlas.join("renamed.md").is_file());
    project::move_entry(&root, "Atlas/renamed.md", "Archive/renamed.md").expect("move markdown");
    assert!(workspace.paths().archive.join("renamed.md").is_file());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn link_resolution_preserves_fragments_without_heading_slug_logic() {
    let root = PathBuf::from("C:/workspace");
    assert_eq!(
        project::resolve_link(
            &root,
            Path::new("Atlas/current.md"),
            "../Archive/page.md#标题"
        ),
        Ok(project::ResolvedLink::WorkspacePath {
            path: PathBuf::from("C:/workspace/Archive/page.md"),
            fragment: Some("标题".to_owned()),
        })
    );
    assert_eq!(
        project::resolve_link(
            &root,
            Path::new("Atlas/current.md"),
            "https://example.com/a"
        ),
        Ok(project::ResolvedLink::Browser {
            url: "https://example.com/a".to_owned(),
        })
    );
    assert_eq!(
        project::resolve_link(
            &root,
            Path::new("Atlas/current.md"),
            "mailto:user@example.com"
        ),
        Ok(project::ResolvedLink::System {
            value: "mailto:user@example.com".to_owned(),
        })
    );
}

#[test]
fn symlink_aliases_are_bounded_by_filesystem_identity() {
    let root = temp_directory("scan-alias");
    let workspace = Workspace::initialize(&root).expect("initialize workspace");
    let target = workspace.paths().atlas.join("target");
    fs::create_dir(&target).expect("create target");
    fs::write(target.join("note.md"), "note").expect("write target");

    let alias = workspace.paths().atlas.join("alias");
    if !create_directory_link(&alias, &target) {
        let _ = fs::remove_dir_all(root);
        return;
    }
    let cycle = target.join("cycle");
    let _ = create_directory_link(&cycle, &workspace.paths().atlas);

    let report = project::scan_workspace(&root, Default::default()).expect("scan aliases");
    assert!(
        report
            .entries()
            .iter()
            .any(|entry| entry.logical_path() == Path::new("Atlas/alias"))
    );
    assert!(
        report
            .aliases()
            .iter()
            .any(|alias| alias.logical_path() == Path::new("Atlas/alias"))
    );
    let target_key = report
        .entries()
        .iter()
        .find(|entry| entry.logical_path() == Path::new("Atlas/target"))
        .expect("target entry")
        .document_key();
    let alias_key = report
        .entries()
        .iter()
        .find(|entry| entry.logical_path() == Path::new("Atlas/alias"))
        .expect("alias entry")
        .document_key();
    assert_eq!(target_key, alias_key);
    assert!(
        report.entries().len() < 100,
        "cycle scan must remain bounded"
    );

    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
fn create_directory_link(link: &Path, target: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}

#[cfg(windows)]
fn create_directory_link(link: &Path, target: &Path) -> bool {
    std::os::windows::fs::symlink_dir(target, link).is_ok()
}
