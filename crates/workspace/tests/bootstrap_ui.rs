use std::{fs, path::{Path, PathBuf}, sync::Arc};

use gpui_kit::test::{TestAppContextExt as _, TestWindowExt as _};
use gpui_kit::{
    AppContext as _, Bounds, Point, Role, TestAppContext, WindowBounds, WindowOptions, px, size,
};
use workspace::{BootstrapOptions, BootstrapView, DirectoryPicker};
use project::AppDataPaths;
use uuid::Uuid;

struct TestPicker(PathBuf);

impl DirectoryPicker for TestPicker {
    fn pick(&self) -> Result<Option<PathBuf>, String> {
        Ok(Some(self.0.clone()))
    }
}

fn temp_directory(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("ohmywork-ui-{name}-{}", Uuid::new_v4()));
    fs::create_dir(&path).expect("create test directory");
    path
}

fn open_view(
    cx: &mut TestAppContext,
    app_data: AppDataPaths,
    picker: Arc<dyn DirectoryPicker>,
) -> (gpui_kit::AnyWindowHandle, gpui_kit::Entity<BootstrapView>) {
    let worker = std::env::current_exe().expect("test executable")
        .parent().and_then(Path::parent).expect("debug directory")
        .join("omw-preview-worker.exe");
    assert!(worker.is_file(), "preview worker must be built beside tests");
    let runtime = Arc::new(tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2).enable_all().build().expect("test runtime"));
    cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(720.), px(560.)),
                })),
                ..Default::default()
            },
            cx,
            |window, cx| {
                let options = BootstrapOptions {
                    app_data: Some(app_data),
                    picker,
                    runtime,
                    preview_worker: worker,
                };
                cx.new(|cx| BootstrapView::new(options, window, cx))
            },
        )
        .expect("open bootstrap test window")
    })
}

#[gpui_kit::test]
async fn directory_picker_populates_root_and_confirm_initializes_workspace(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let root = temp_directory("picker");
    let app_data_root = temp_directory("app-data");
    let app_data = AppDataPaths::from_base(app_data_root.clone()).expect("app data path");
    let (handle, view) = open_view(cx, app_data, Arc::new(TestPicker(root.clone())));

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("workspace-status").role(), Some(Role::Status));
        window.click("choose-directory", cx);
    })
    .expect("choose directory");
    cx.run_until_parked();

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("workspace-root-input").value(),
            Some(root.to_string_lossy().as_ref())
        );
        window.click("initialize-workspace", cx);
    })
    .expect("initialize workspace");
    cx.run_until_parked();

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("workspace-status").is_none());
        assert!(window.try_find("show-files").is_some());
        assert!(window.try_find("home-pane").is_some());
    })
    .expect("observe initialization");


    cx.update(|cx| {
        let view = view.read(cx);
        assert!(view.status().contains("Workspace 已就绪"));
        let entered = PathBuf::from(view.root_input().read(cx).value().as_ref());
        assert_eq!(
            fs::canonicalize(entered).expect("canonicalize initialized root input"),
            fs::canonicalize(&root).expect("canonicalize picker root")
        );
    });

    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(app_data_root);
}
