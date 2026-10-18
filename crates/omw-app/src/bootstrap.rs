use std::{path::PathBuf, sync::Arc};

use gpui_kit::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Role, SharedString,
    Styled as _, TestSupportExt as _, Window,
    component::{
        ActiveTheme as _,
        button::Button,
        form::{Field, Form},
        input::{Input, InputState},
    },
    div,
    prelude::*,
    px,
};
use omw_local::{AppDataPaths, StateError, WorkspaceState};

pub trait DirectoryPicker: Send + Sync {
    fn pick(&self) -> Result<Option<PathBuf>, String>;
}

#[derive(Default)]
pub struct SystemDirectoryPicker;

impl DirectoryPicker for SystemDirectoryPicker {
    fn pick(&self) -> Result<Option<PathBuf>, String> {
        pick_directory()
    }
}

pub struct BootstrapView {
    input: Entity<InputState>,
    app_data: Option<AppDataPaths>,
    picker: Arc<dyn DirectoryPicker>,
    status: SharedString,
    state: Option<WorkspaceState>,
    busy: bool,
    setup_blocked: bool,
    shell: Option<Entity<crate::ShellView>>,
}
impl BootstrapView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let app_data = AppDataPaths::from_local_app_data().ok();
        Self::with_app_data_and_picker(window, cx, app_data, Arc::new(SystemDirectoryPicker))
    }

    pub fn with_app_data(
        window: &mut Window,
        cx: &mut Context<Self>,
        app_data: AppDataPaths,
    ) -> Self {
        Self::with_app_data_and_picker(window, cx, Some(app_data), Arc::new(SystemDirectoryPicker))
    }

    pub fn with_app_data_and_picker(
        window: &mut Window,
        cx: &mut Context<Self>,
        app_data: Option<AppDataPaths>,
        picker: Arc<dyn DirectoryPicker>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("选择或输入绝对路径"));
        let has_app_data = app_data.is_some();
        let mut view = Self {
            input,
            app_data,
            picker,
            status: if has_app_data {
                "正在读取本地设置…".into()
            } else {
                "无法确定 %LOCALAPPDATA%，设置不会写入。".into()
            },
            state: None,
            busy: false,
            setup_blocked: false,
            shell: None,
        };
        if let Some(app_data) = view.app_data.clone() {
            let task = cx.background_spawn(async move { WorkspaceState::load(app_data) });
            cx.spawn_in(window, async move |this, cx| {
                let result = task.await;
                let _ = this.update_in(cx, |this, window, cx| match result {
                    Ok(Some(state)) => {
                        let root = state.root().to_path_buf();
                        this.input.update(cx, |input, cx| {
                            input.set_value(root.to_string_lossy().into_owned(), window, cx);
                        });
                        this.status = format!("已恢复 Workspace：{}", root.display()).into();
                        this.state = Some(state);
                    }
                    Ok(None) => {
                        this.status = "选择一个 Workspace 根目录。不会搬移已有内容。".into();
                    }
                    Err(error) => {
                        this.status = format!("读取本地设置失败：{}", error).into();
                        this.setup_blocked = true;
                    }
                });
            })
            .detach();
        }
        view
    }

    pub fn status(&self) -> &SharedString {
        &self.status
    }

    pub fn root_input(&self) -> &Entity<InputState> {
        &self.input
    }

    pub fn workspace_state(&self) -> Option<&WorkspaceState> {
        self.state.as_ref()
    }

    fn choose_directory(
        &mut self,
        _: &gpui_kit::ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        self.status = "正在打开系统目录选择器…".into();
        self.busy = true;
        let picker = self.picker.clone();
        let task = cx.background_spawn(async move { picker.pick() });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(Some(path)) => {
                        this.input.update(cx, |input, cx| {
                            input.set_value(path.to_string_lossy().into_owned(), window, cx);
                        });
                        this.status = "已选择目录，请确认初始化。".into();
                    }
                    Ok(None) => this.status = "已取消目录选择。".into(),
                    Err(error) => this.status = format!("目录选择失败：{error}").into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn initialize_workspace(
        &mut self,
        _: &gpui_kit::ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        if self.state.is_some() {
            self.status = "Workspace 已存在，未切换根目录。".into();
            return;
        }
        if self.setup_blocked {
            self.status = "设置文件无法安全读取，已保留原文件；请先修复或备份后再重试。".into();
            return;
        }
        let Some(app_data) = self.app_data.clone() else {
            self.status = "缺少 %LOCALAPPDATA%，无法保存设置。".into();
            return;
        };
        let root = PathBuf::from(self.input.read(cx).value().as_ref());
        if root.as_os_str().is_empty() {
            self.status = "请输入或选择 Workspace 根目录。".into();
            return;
        }
        self.busy = true;
        self.status = "正在检查目录并保存设置…".into();
        let task = cx.background_spawn(async move { WorkspaceState::initialize(root, app_data) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(state) => {
                        this.status = format!(
                            "Workspace 已就绪：{}（UUID {}）",
                            state.root().display(),
                            state.workspace_uuid()
                        )
                        .into();
                        this.state = Some(state);
                    }
                    Err(error) => this.status = format_state_error(error),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for BootstrapView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // plan18 integration: once workspace chosen, show shell with dock
        if self.shell.is_none() {
            if let (Some(st), Some(paths)) = (&self.state, &self.app_data) {
                let root = st.root().to_path_buf();
                let sh = cx.new(|cx| crate::ShellView::new(root, paths.clone(), window, cx));
                self.shell = Some(sh);
            }
        }
        if let Some(shell) = &self.shell {
            return div().size_full().child(shell.clone());
        }

        let root = self.input.read(cx).value();
        let status = self.status.clone();
        let root_preview = if root.is_empty() {
            "选择目录后将在这里显示".to_owned()
        } else {
            let path = PathBuf::from(root.as_ref());
            format!(
                "Atlas: {}\nEfforts: {}\nArchive: {}\n身份：{}",
                path.join("Atlas").display(),
                path.join("Efforts").display(),
                path.join("Archive").display(),
                path.join(".ohmywork/id").display()
            )
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_4()
            .p_6()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child("初始化 ohMyWork Workspace")
            .child(
                Form::new()
                    .w(px(560.))
                    .child(
                        Field::new().label("Workspace 根目录").required(true).child(
                            div()
                                .flex()
                                .gap_2()
                                .child(
                                    Input::new(&self.input)
                                        .id("workspace-root-input")
                                        .aria_label("Workspace 根目录")
                                        .flex_1(),
                                )
                                .child(
                                    Button::new("choose-directory")
                                        .label("选择目录")
                                        .on_click(cx.listener(Self::choose_directory)),
                                ),
                        ),
                    )
                    .footer(
                        Button::new("initialize-workspace")
                            .label("确认并初始化")
                            .on_click(cx.listener(Self::initialize_workspace)),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(status.clone()),
            )
            .child(
                div()
                    .id("workspace-preview")
                    .test_support()
                    .aria_label(root_preview.clone())
                    .child(root_preview),
            )
            .child(
                div()
                    .id("workspace-status")
                    .role(Role::Status)
                    .test_support()
                    .aria_label(status.clone())
                    .child(status),
            )
    }
}

fn format_state_error(error: StateError) -> SharedString {
    format!("初始化失败：{error}").into()
}

#[cfg(windows)]
fn pick_directory() -> Result<Option<PathBuf>, String> {
    use windows::{
        Win32::{
            Foundation::ERROR_CANCELLED,
            System::Com::{
                CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
                CoUninitialize,
            },
            UI::Shell::{
                FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST, FOS_PICKFOLDERS, FileOpenDialog,
                IFileOpenDialog, SIGDN_FILESYSPATH,
            },
        },
        core::HRESULT,
    };

    let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    if let Err(error) = initialized.ok() {
        return Err(format!("初始化 Windows COM 失败：{error}"));
    }
    let result = (|| {
        let dialog: IFileOpenDialog = unsafe {
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)
                .map_err(|error| format!("创建目录选择器失败：{error}"))?
        };
        unsafe {
            dialog
                .SetOptions(FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST)
                .map_err(|error| format!("配置目录选择器失败：{error}"))?;
            if let Err(error) = dialog.Show(None) {
                if error.code() == HRESULT::from_win32(ERROR_CANCELLED.0) {
                    return Ok(None);
                }
                return Err(format!("显示目录选择器失败：{error}"));
            }
            let item = dialog
                .GetResult()
                .map_err(|error| format!("读取所选目录失败：{error}"))?;
            let path = item
                .GetDisplayName(SIGDN_FILESYSPATH)
                .map_err(|error| format!("读取所选目录路径失败：{error}"))?
                .to_string()
                .map_err(|error| format!("转换所选目录路径失败：{error}"))?;
            Ok(Some(PathBuf::from(path)))
        }
    })();
    unsafe { CoUninitialize() };
    result
}

#[cfg(not(windows))]
fn pick_directory() -> Result<Option<PathBuf>, String> {
    Err("系统目录选择器仅在 Windows V1 上可用".to_owned())
}
