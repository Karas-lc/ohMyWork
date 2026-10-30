use crate::closing::{CloseCoordinator, CloseDecision, CloseEvent};
use crate::persistence::{PersistedDockState, UiStateWriter};
use crate::session::{WorkspaceEvent, WorkspaceSession};
use crate::ui::panes::{HomePanel, MarkdownPanel, PanelRegistry, register_pane_factories};
use crate::views::files::FileTreeView;
use crate::views::pi::PiView;
use crate::views::preview::PreviewPanel;
use crate::ui::tabs::TabRef;
use editor::documents::{DocumentEvent, DocumentStore};
use crate::queries::FileBrowser;
use gpui_kit::component::{
    ActiveTheme,
    button::Button,
    dock::{DockArea, DockEvent, DockLayout, DockPlacement, DockSkin, PanelId, panel_handle},
};
use gpui_kit::{AppContext as _, Context, Entity, IntoElement, Render, Subscription, Window, div, prelude::*, px};
use std::path::PathBuf;

pub struct ShellView {
    dock_area: Entity<DockArea>,
    panels: Entity<PanelRegistry>,
    session: Entity<WorkspaceSession>,
    documents: Entity<DocumentStore>,
    closing: Entity<CloseCoordinator>,
    writer: UiStateWriter,
    file_browser: Entity<FileBrowser>,
    tree: Entity<FileTreeView>,
    search: Entity<crate::views::search::SearchView>,
    tasks: Entity<crate::views::tasks::TasksView>,
    pi: Entity<PiView>,
    _subs: Vec<Subscription>,
    status: String,
    side: u8,
    close_prompt: Option<PathBuf>,
    palette: Option<Entity<gpui_kit::component::command::CommandState>>,
}
impl ShellView {
    pub fn new(
        writer: UiStateWriter,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let context = cx.global::<crate::context::WorkspaceContext>().clone();
        let session = context.workspace();
        let documents = context.documents();
        let file_browser = context.files();
        let search_model = context.search();
        let tasks_model = context.tasks();
        let homepage = context.homepage();
        let pi_session = context.pi();
        let closing = context.closing();
        let panels = cx.new(|_| PanelRegistry::default());
        register_pane_factories(
            cx,
            panels.clone(),
            documents.clone(),
            homepage.clone(),
            tasks_model.clone(),
            session.clone(),
        );

        let (dock_area, skin) = DockSkin::dock_area("main-dock", Some(1), window, cx);
        skin.set_close_button_visible(true, cx);
        let home = cx.new(|cx| HomePanel::new(homepage, tasks_model.clone(), session.clone(), window, cx));
        dock_area.update(cx, |area, cx| {
            area.set_center(DockLayout::tabs().panel_view(panel_handle(home), cx), window, cx)
        });
        let tree = cx.new(|cx| FileTreeView::new(file_browser.clone(), session.clone(), window, cx));
        let search = cx.new(|cx| {
            crate::views::search::SearchView::new(search_model, session.clone(), window, cx)
        });
        let tasks = cx.new(|cx| {
            crate::views::tasks::TasksView::new(tasks_model, session.clone(), window, cx)
        });
        let pi = cx.new(|cx| PiView::new(pi_session.clone(), window, cx));

        let dock_sub = cx.subscribe(&dock_area, |this, _, event: &DockEvent, cx| {
            if matches!(event, DockEvent::LayoutChanged) {
                if let Err(error) = this.persist(cx) {
                    this.status = error;
                }
            }
        });
        let document_sub = cx.subscribe(&documents, |this, _, event: &DocumentEvent, cx| {
            match event {
                DocumentEvent::Failed { message, .. } => this.status = message.clone(),
                DocumentEvent::Saved(_) => this.status = "已保存".into(),
                DocumentEvent::TaskCommitted(_) => {
                    this.status = "任务字段已保存；其他正文仍未保存".into()
                }
                _ => {}
            }
            cx.notify();
        });
        let session_sub = cx.subscribe_in(
            &session,
            window,
            |this, _, event: &WorkspaceEvent, window, cx| {
                match event {
                    WorkspaceEvent::Ready => {
                        this.status = "就绪 · 外部文件修改将覆盖未保存编辑".into()
                    }
                    WorkspaceEvent::Failed(error) => this.status = error.clone(),
                    WorkspaceEvent::OpenPage(path) | WorkspaceEvent::OpenPreview(path) => {
                        this.open_resolved(path.clone(), window, cx)
                    }
                    WorkspaceEvent::RevealDirectory(path) => {
                        let root = this.file_browser.read(cx).root().to_path_buf();
                        if let Ok(logical) = path.strip_prefix(root) {
                            this.side = 0;
                            this.tree.update(cx, |tree, cx| tree.open_directory(logical, cx));
                        } else {
                            this.status = "目录不在当前 Workspace 内".into();
                        }
                    }
                    WorkspaceEvent::ShowTasks => this.side = 2,
                    WorkspaceEvent::ShowPi => this.side = 3,
                }
                cx.notify();
            },
        );
        let close_sub = cx.subscribe_in(
            &closing,
            window,
            |this, _, event: &CloseEvent, window, cx| {
                match event {
                    CloseEvent::Prompt { path } => this.close_prompt = Some(path.clone()),
                    CloseEvent::RemoveTab { path } => {
                        let panel = this.panels.update(cx, |panels, _| panels.remove(path));
                        if let Some(panel) = panel {
                            this.dock_area.update(cx, |area, cx| {
                                area.remove_panel(panel, window, cx)
                            });
                        }
                        this.close_prompt = None;
                    }
                    CloseEvent::CaptureLayout => match this.persist(cx) {
                        Ok(()) => this.closing.update(cx, |owner, cx| owner.layout_captured(cx)),
                        Err(error) => this
                            .closing
                            .update(cx, |owner, cx| owner.layout_failed(error, cx)),
                    },
                    CloseEvent::ReadyToCloseWindow => window.remove_window(),
                    CloseEvent::Cancelled => this.close_prompt = None,
                    CloseEvent::Failed(error) => {
                        this.close_prompt = None;
                        this.status = error.clone();
                    }
                }
                cx.notify();
            },
        );
        let weak_closing = closing.clone();
        window.on_window_should_close(cx, move |_, cx| {
            weak_closing.update(cx, |closing, cx| closing.request_window(cx));
            false
        });

        if let (Some(paths), Some(root)) = (
            session.read(cx).app_data().cloned(),
            session.read(cx).root().map(PathBuf::from),
        ) {
            let load = cx.background_spawn(async move {
                crate::persistence::load_ui_state(&paths, &root)
            });
            cx.spawn_in(window, async move |this, cx| {
                let result = load.await;
                let _ = this.update_in(cx, |this, window, cx| {
                    match result {
                        Ok(state) => {
                            if let Some(dock) = state.dock {
                                match serde_json::from_value::<gpui_kit::component::dock::DockAreaState>(dock.dock) {
                                    Ok(dock) => if let Err(error) = this.dock_area.update(cx, |area, cx| area.load(dock, window, cx)) {
                                        this.status = error.to_string();
                                    },
                                    Err(error) => this.status = format!("Invalid dock state: {error}"),
                                }
                            }
                        }
                        Err(error) => this.status = error,
                    }
                    cx.notify();
                });
            }).detach();
        }
        Self {
            dock_area,
            panels,
            session,
            documents,
            closing,
            writer,
            file_browser,
            tree,
            search,
            tasks,
            pi,
            _subs: vec![dock_sub, document_sub, session_sub, close_sub],
            status: "正在恢复工作区…".into(),
            side: 0,
            close_prompt: None,
            palette: None,
        }
    }
    pub fn show_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = cx.new(|cx| gpui_kit::component::command::CommandState::new(window, cx));
        state.update(cx, |state, cx| state.focus(window, cx));
        self.palette = Some(state);
        cx.notify();
    }
    fn persist(&self, cx: &gpui_kit::App) -> Result<(), String> {
        let dock = serde_json::to_value(self.dock_area.read(cx).dump(cx))
            .map_err(|error| format!("无法序列化 Dock 布局：{error}"))?;
        self.writer.write_dock(PersistedDockState {
            schema_version: 1,
            kit_layout_version: 1,
            dock,
        })
    }

    fn save(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        match self.documents.update(cx, |store, cx| store.save(path, cx)) {
            Ok(()) => self.status = "正在保存…".into(),
            Err(error) => self.status = error.to_string(),
        }
        cx.notify();
    }
    fn choose_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selected = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = selected.await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.into_iter().next() {
                            this.open(path, window, cx);
                        }
                    }
                    Ok(Err(error)) => this.status = error.to_string(),
                    Err(error) => this.status = error.to_string(),
                    _ => {}
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn open(&mut self, path: PathBuf, _window: &mut Window, cx: &mut Context<Self>) {
        self.session.update(cx, |session, cx| session.request_open(path, cx));
    }
    fn open_resolved(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if extension == "md" || extension == "markdown" {
            if let Some(panel) = self.panels.read(cx).markdown(&path) {
                self.dock_area.update(cx, |area, cx| {
                    area.select_panel(PanelId::from(panel.entity_id()), window, cx)
                });
                return;
            }
            let panel = cx.new(|cx| {
                MarkdownPanel::open(
                    path.clone(),
                    self.session.clone(),
                    self.documents.clone(),
                    self.panels.clone(),
                    window,
                    cx,
                )
            });
            self.dock_area.update(cx, |area, cx| {
                area.add_panel_view(panel_handle(panel), DockPlacement::Center, None, window, cx)
            });
        } else if ["png", "jpg", "jpeg", "gif", "webp", "bmp", "pdf"].contains(&extension.as_str()) {
            let context = cx.global::<crate::context::WorkspaceContext>().clone();
            let session = context.new_preview(path.clone(), cx);
            let panel = cx.new(|cx| PreviewPanel::new(TabRef::Preview { path }, session, cx));
            self.dock_area.update(cx, |area, cx| {
                area.add_panel_view(panel_handle(panel), DockPlacement::Center, None, window, cx)
            });
        }
    }
}
impl Render for ShellView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use crate::keymap::{CloseTab, OpenFile, Save, Search, ToggleFileTree, TogglePi};
        let mut sidebar = div()
            .w(px(280.))
            .h_full()
            .flex()
            .flex_col()
            .overflow_hidden();
        sidebar = match self.side {
            1 => sidebar.child(self.search.clone()),
            2 => sidebar.child(self.tasks.clone()),
            3 => sidebar.child(self.pi.clone()),
            _ => sidebar.child(self.tree.clone()),
        };
        let mut body = div()
            .key_context("Global")
            .size_full()
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .on_action(cx.listener(|this, &OpenFile, window, cx| this.choose_file(window, cx)))
            .on_action(cx.listener(|this, &TogglePi, _, cx| {
                this.side = 3;
                cx.notify();
            }))
            .on_action(cx.listener(|this, &Search, _, cx| {
                this.side = 1;
                cx.notify();
            }))
            .on_action(cx.listener(|this, &ToggleFileTree, _, cx| {
                this.side = 0;
                cx.notify();
            }))
            .on_action(cx.listener(|this, &Save, _, cx| {
                if let Some(path) = this.session.read(cx).active().map(PathBuf::from) {
                    this.save(path, cx);
                }
            }))
            .on_action(cx.listener(|this, &CloseTab, _, cx| {
                if let Some(path) = this.session.read(cx).active().map(PathBuf::from) {
                    this.closing.update(cx, |closing, cx| closing.request_tab(path, cx));
                }
                cx.notify();
            }))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .p_2()
                    .child(
                        Button::new("show-files")
                            .label("文件")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.side = 0;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("show-search")
                            .label("搜索")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.side = 1;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("show-tasks")
                            .label("任务")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.side = 2;
                                cx.notify();
                            })),
                    )
                    .child(Button::new("show-pi").label("Pi").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.side = 3;
                            cx.notify();
                        },
                    )))
                    .child(
                        Button::new("show-commands")
                            .label("命令 / Commands")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.show_command_palette(window, cx)
                            })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(sidebar)
                    .child(div().flex_1().h_full().child(self.dock_area.clone())),
            )
            .child(div().text_sm().p_1().child(self.status.clone()));
        if let Some(palette) = &self.palette {
            use gpui_kit::component::command::{Command, CommandItem};
            let commands = [
                "保存 / Save",
                "关闭当前页 / Close",
                "打开文件 / Open file",
                "文件 / Files",
                "搜索 / Search",
                "Pi",
            ];
            let confirm = cx.weak_entity();
            let cancel = cx.weak_entity();
            body = body.child(
                Command::new(palette)
                    .items(commands.map(|label| CommandItem::new().label(label)))
                    .on_confirm(move |index, window, cx| {
                        let _ = confirm.update(cx, |this, cx| {
                            this.palette = None;
                            match index.row {
                                0 => {
                                    if let Some(path) = this.session.read(cx).active().map(PathBuf::from) {
                                        this.save(path, cx);
                                    }
                                }
                                1 => {
                                    if let Some(path) = this.session.read(cx).active().map(PathBuf::from) {
                                        this.closing.update(cx, |closing, cx| closing.request_tab(path, cx));
                                    }
                                }
                                2 => this.choose_file(window, cx),
                                3 => this.side = 0,
                                4 => this.side = 1,
                                5 => this.side = 3,
                                _ => {}
                            }
                            cx.notify();
                        });
                    })
                    .on_cancel(move |_, cx| {
                        let _ = cancel.update(cx, |this, cx| {
                            this.palette = None;
                            cx.notify();
                        });
                    }),
            );
        }
        if let Some(path) = self.close_prompt.clone() {
            body = body.child(
                div()
                    .p_3()
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(format!("{} 有未保存修改", path.display()))
                    .child(
                        Button::new("close-save")
                            .label("保存")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.close_prompt = None;
                                this.closing.update(cx, |closing, cx| closing.decide(CloseDecision::Save, cx));
                            })),
                    )
                    .child(
                        Button::new("close-discard")
                            .label("放弃")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.close_prompt = None;
                                this.closing.update(cx, |closing, cx| closing.decide(CloseDecision::Discard, cx));
                            })),
                    )
                    .child(
                        Button::new("close-cancel")
                            .label("取消")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.closing.update(cx, |closing, cx| closing.decide(CloseDecision::Cancel, cx));
                            })),
                    ),
            );
        }
        body
    }
}
