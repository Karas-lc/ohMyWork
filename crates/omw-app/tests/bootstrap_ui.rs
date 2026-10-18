use std::{fs, path::PathBuf, sync::Arc};

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Bounds, Point, Role, TestAppContext, WindowBounds, WindowOptions, px, size,
};
use omw_app::{BootstrapView, DirectoryPicker};
use omw_local::AppDataPaths;
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
                cx.new(|cx| {
                    BootstrapView::with_app_data_and_picker(window, cx, Some(app_data), picker)
                })
            },
        )
        .expect("open bootstrap test window")
    })
}

#[gpui_kit::test]
fn directory_picker_populates_root_and_confirm_initializes_workspace(cx: &mut TestAppContext) {
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
        let state = view.workspace_state().expect("workspace state");
        assert!(state.root().is_dir());
        assert!(state.root().join(".ohmywork/id").is_file());
        assert_eq!(
            view.root_input().read(cx).value().as_ref(),
            root.to_string_lossy()
        );
    });

    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(app_data_root);
}
