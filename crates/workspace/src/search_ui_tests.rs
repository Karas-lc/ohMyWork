use std::{fs, path::PathBuf, sync::Arc};

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Bounds, Point, Role, TestAppContext, WindowBounds, WindowOptions, px, size,
};
use crate::{queries::SearchModel, session::WorkspaceSession, views::search::SearchView};
use project::{FileProjection, IndexAlias, IndexArea, IndexBatch, IndexStore};
use uuid::Uuid;

fn temp_directory(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("ohmywork-search-ui-{name}-{}", Uuid::new_v4()));
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
    let mut p = FileProjection::new(path.to_string_lossy(), generation);
    p.content_hash = vec![generation as u8; 32];
    p.path_fold = aliases
        .iter()
        .map(|(a, _, _)| *a)
        .collect::<Vec<_>>()
        .join("\n");
    p.title_fold = title.to_owned();
    p.body_fold = body.to_owned();
    p.aliases = aliases
        .iter()
        .map(|(alias, area, excluded)| IndexAlias::new(*alias, *area, *excluded))
        .collect();
    p
}

#[gpui_kit::test]
fn search_input_drives_results_and_archive_toggle(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);

    let root = temp_directory("root");
    let note = root.join("note.md");
    fs::write(&note, "# 项目方案\n\n中文测试内容 ＡＢＣ").expect("write note");
    let archive_note = root.join("arch.md");
    fs::write(&archive_note, "# 归档\n\n中文归档").expect("write arch");

    let index_path = root.join("index.sqlite");
    let store = Arc::new(IndexStore::open(&index_path).expect("open store"));
    let mut batch = IndexBatch::new(42);
    batch.push(projection(
        &note,
        42,
        &[
            ("Atlas/项目方案.md", IndexArea::Atlas, false),
            ("Archive/项目方案.md", IndexArea::Archive, false),
        ],
        "项目方案",
        "中文测试内容 ＡＢＣ",
    ));
    batch.push(projection(
        &archive_note,
        42,
        &[("Archive/arch.md", IndexArea::Archive, false)],
        "归档",
        "中文归档",
    ));
    store.ingest(batch).expect("ingest");
    store
        .complete_generation(
            42,
            [
                "Atlas/项目方案.md".to_owned(),
                "Archive/项目方案.md".to_owned(),
                "Archive/arch.md".to_owned(),
            ],
        )
        .expect("complete");

    let (handle, _view_entity, model) = cx.update(|cx| {
        let store = store.clone();
        let model = cx.new(|cx| SearchModel::new(store, cx));
        let view_model = model.clone();
        let session = cx.new(|cx| WorkspaceSession::new(None, cx));
        let (handle, view_entity) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(800.), px(600.)),
                })),
                ..Default::default()
            },
            cx,
            move |window, cx| cx.new(|cx| SearchView::new(view_model, session, window, cx)),
        )
        .expect("open search test window");
        (handle, view_entity, model)
    });

    // Initial render
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let status = window.find("search-status");
        assert_eq!(status.role(), Some(Role::Status));
        assert!(status.label().is_some_and(|l| l.contains("索引尚未就绪") || l.contains("搜索")));
    })
    .expect("initial render");

    // Drive input for short Chinese query ("中" should only match visible Atlas)
    cx.update_window(handle, |_, window, cx| {
        window.click("search-input", cx);
        window.input("中", cx);
    })
    .expect("type query");

    cx.run_until_parked();

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let status = window.find("search-status");
        let label = status.label().unwrap_or_default();
        assert!(
            label.contains("结果") || label.contains("显示"),
            "unexpected status: {label}"
        );
    })
    .expect("after search render");

    // Verify via entity API (logic + debounce worked, short query no archive leak)
    cx.update(|cx| {
        let results = model.read(cx).rows().to_vec();
        assert!(results.iter().all(|result| !result.archive), "short query leaked archive");
        assert!(
            results.iter().any(|result| result.logical_path.contains("项目方案")),
            "expected Atlas result for '中'"
        );
    });

    // Toggle archive include
    cx.update_window(handle, |_, window, cx| {
        window.click("search-include-archive", cx);
    })
    .expect("toggle archive");

    cx.run_until_parked();

    cx.update(|cx| {
        let model = model.clone();
        assert!(model.read(cx).include_archive());
        let results = model.read(cx).rows().to_vec();
        assert!(!results.is_empty(), "after archive toggle expected some results");
    });

    // Cleanup
    drop(store);
    let _ = fs::remove_file(&index_path);
    let _ = fs::remove_dir_all(root);
}
