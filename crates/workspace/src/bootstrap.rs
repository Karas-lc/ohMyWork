use std::{path::PathBuf, sync::Arc};

use editor::documents::DocumentStore;
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
use project::{AppDataPaths, WriteQueue};

use crate::{
    closing::CloseCoordinator,
    context::WorkspaceContext,
    homepage::Homepage,
    persistence::UiStateWriter,
    pi::PiSession,
    queries::{FileBrowser, SearchModel, TasksModel},
    session::{WorkspaceEvent, WorkspaceSession},
};

pub struct BootstrapOptions {
    pub app_data: Option<AppDataPaths>,
    pub picker: Arc<dyn DirectoryPicker>,
    pub runtime: Arc<tokio::runtime::Runtime>,
    pub preview_worker: PathBuf,
}

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
    options: BootstrapOptions,
    status: SharedString,
    busy: bool,
    setup_blocked: bool,
    session: Entity<WorkspaceSession>,
    _session_subscription: gpui_kit::Subscription,
    shell: Option<Entity<crate::ui::shell::ShellView>>,
}

impl BootstrapView {
    pub fn new(options: BootstrapOptions, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("选择或输入绝对路径"));
        let has_app_data = options.app_data.is_some();
        let session = cx.new(|cx| WorkspaceSession::new(options.app_data.clone(), cx));
        let view = Self {
            input,
            options,
            status: if has_app_data {
                "正在读取本地设置…".into()
            } else {
                "无法确定 %LOCALAPPDATA%，设置不会写入。".into()
            },
            busy: false,
            setup_blocked: false,
            session: session.clone(),
            _session_subscription: cx.subscribe_in(&session, window, |this, _, event: &WorkspaceEvent, window, cx| {
                match event {
                    WorkspaceEvent::Ready => this.create_workspace_shell(window, cx),
                    WorkspaceEvent::Failed(error) => {
                        if !this.busy {
                            this.setup_blocked = true;
                        }
                        this.busy = false;
                        this.status = format!("初始化失败：{error}").into();
                        cx.notify();
                    }
                    _ => {}
                }
            }),
            shell: None,
        };
        if has_app_data {
            session.update(cx, |session, cx| session.restore(cx));
        }
        view
    }

    fn create_workspace_shell(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.shell.is_some() {
            return;
        }
        let (root, paths, index) = {
            let session = self.session.read(cx);
            let (Some(root), Some(paths), Some(index)) = (
                session.root().map(PathBuf::from),
                session.app_data().cloned(),
                session.index_handle(),
            ) else {
                self.status = "Workspace 已就绪，但缺少所需的状态或索引。".into();
                cx.notify();
                return;
            };
            (root, paths, index)
        };
        self.input.update(cx, |input, cx| {
            input.set_value(root.to_string_lossy().into_owned(), window, cx);
        });
        let writer = match UiStateWriter::new(paths.clone(), root.clone()) {
            Ok(writer) => writer,
            Err(error) => {
                self.status = format!("无法创建界面状态写入器：{error}").into();
                cx.notify();
                return;
            }
        };
        let documents = cx.new(|cx| DocumentStore::new(WriteQueue::new(2), cx));
        let files = cx.new(|cx| FileBrowser::new(root.clone(), cx));
        let search = cx.new(|cx| SearchModel::new(index, cx));
        let tasks = cx.new(|cx| TasksModel::new(root.clone(), documents.clone(), cx));
        let homepage = cx.new(|cx| Homepage::new(paths.clone(), root.clone(), writer.clone(), cx));
        let pi = cx.new(|cx| PiSession::new(self.options.runtime.clone(), cx));
        let closing = cx.new(|cx| {
            CloseCoordinator::new(documents.clone(), pi.clone(), homepage.clone(), writer.clone(), cx)
        });
        cx.set_global(WorkspaceContext::new(
            self.session.clone(),
            documents,
            files,
            search,
            tasks,
            homepage,
            pi,
            closing,
            self.options.runtime.clone(),
            self.options.preview_worker.clone(),
        ));
        self.status = format!("Workspace 已就绪：{}", root.display()).into();
        self.busy = false;
        self.shell = Some(cx.new(|cx| {
            crate::ui::shell::ShellView::new(writer, window, cx)
        }));
        cx.notify();
    }

    pub fn status(&self) -> &SharedString {
        &self.status
    }

    pub fn root_input(&self) -> &Entity<InputState> {
        &self.input
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
        let picker = self.options.picker.clone();
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
        if self.session.read(cx).ready() {
            self.status = "Workspace 已存在，未切换根目录。".into();
            return;
        }
        if self.setup_blocked {
            self.status = "设置文件无法安全读取，已保留原文件；请先修复或备份后再重试。".into();
            return;
        }
        if self.options.app_data.is_none() {
            self.status = "缺少 %LOCALAPPDATA%，无法保存设置。".into();
            return;
        }
        let root = PathBuf::from(self.input.read(cx).value().as_ref());
        if root.as_os_str().is_empty() {
            self.status = "请输入或选择 Workspace 根目录。".into();
            return;
        }
        self.busy = true;
        self.status = "正在检查目录并保存设置…".into();
        let session = self.session.clone();
        session.update(cx, |session, cx| session.initialize(root, cx));
    }
}

impl Render for BootstrapView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
