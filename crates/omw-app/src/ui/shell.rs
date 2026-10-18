use super::pi_panel::PiPanel;
use crate::i18n::{I18n, Lang};
use crate::keymap::{Keymap, load_keymap};
use crate::{
    document_session::{ExternalEvent, SaveStatus},
    file_tree::FileTreeView,
    search::SearchView,
    ui::panes::{
        HomePanel, MarkdownPanel, PersistedDockState, PreviewPanel, register_pane_factories,
    },
    ui::tabs::TabRef,
    workbench::{Workbench, WorkbenchEvent},
};
use gpui_kit::component::{
    ActiveTheme,
    button::Button,
    dock::{DockArea, DockEvent, DockLayout, DockPlacement, DockSkin, PanelId, panel_handle},
};
use gpui_kit::{Context, Entity, IntoElement, Render, Subscription, Window, div, prelude::*, px};
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};

pub struct ShellView {
    dock_area: Entity<DockArea>,
    services: Option<Workbench>,
    tree: Entity<FileTreeView>,
    search: Option<Entity<SearchView>>,
    tasks: Option<Entity<super::tasks::TasksView>>,
    pi: Option<Entity<PiPanel>>,
    panels: HashMap<PathBuf, Entity<MarkdownPanel>>,
    _subs: Vec<Subscription>,
    keymap: Keymap,
    i18n: I18n,
    status: String,
    side: u8,
    close_pending: Option<(PathBuf, bool)>,
    closing_window: bool,
    flushing_close: bool,
    reading_disk: bool,
    palette: Option<Entity<gpui_kit::component::command::CommandState>>,
}
impl ShellView {
    pub fn new(
        root: PathBuf,
        paths: omw_local::AppDataPaths,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let services = Workbench::new(root.clone(), paths.clone());
        let error = services.as_ref().err().cloned();
        let services = services.ok();
        if let Some(services) = &services {
            cx.set_global(services.clone());
        }
        register_pane_factories(cx, root.clone());
        let (area, skin) = DockSkin::dock_area("main-dock", Some(1), window, cx);
        skin.set_close_button_visible(true, cx);
        let home = cx.new(|cx| HomePanel::new(root.clone(), window, cx));
        area.update(cx, |area, cx| {
            area.set_center(
                DockLayout::tabs().panel_view(panel_handle(home), cx),
                window,
                cx,
            )
        });
        let tree = cx.new(|cx| FileTreeView::new(root.clone(), window, cx));
        let pi = services.as_ref().map(|services| {
            cx.new(|cx| {
                let mut pi = PiPanel::new(window, cx);
                pi.set_runtime(Arc::new(services.runtime.handle().clone()));
                pi
            })
        });
        let keymap = load_keymap(&paths);
        let dock_sub = cx.subscribe(&area, |this, _, event: &DockEvent, cx| {
            if matches!(event, DockEvent::LayoutChanged) {
                this.persist(cx);
            }
        });
        let root_load = root.clone();
        let load = cx.background_spawn(async move {
            let state = crate::state::load_ui_state(&paths, &root_load);
            let index =
                Arc::new(omw_local::IndexStore::open_app_data(&paths).map_err(|e| e.to_string())?);
            index
                .index_workspace(&root_load, omw_local::ScanOptions::default())
                .map_err(|e| e.to_string())?;
            Ok::<_, String>((state, index))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = load.await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok((state, index)) => {
                        this.search = Some(cx.new(|cx| SearchView::new(index.clone(), window, cx)));
                        this.tasks =
                            Some(cx.new(|cx| super::tasks::TasksView::new(index, window, cx)));
                        if let Some(dock) = state.dock {
                            if dock.schema_version == 1 && dock.kit_layout_version == 1 {
                                if let Err(error) = this
                                    .dock_area
                                    .update(cx, |area, cx| area.load(dock.dock, window, cx))
                                {
                                    this.status = error.to_string();
                                }
                            }
                        }
                        this.status = "就绪 · 外部文件修改将覆盖未保存编辑".into();
                    }
                    Err(error) => this.status = error,
                }
                cx.notify();
            });
        })
        .detach();
        let weak = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            weak.update(cx, |this, cx| {
                if this.services.is_none() {
                    return true;
                }
                this.closing_window = true;
                if let Some(path) = this.dirty_path(cx) {
                    this.close_pending = Some((path, true));
                }
                let _ = window;
                cx.notify();
                false
            })
            .unwrap_or(true)
        });
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(200))
                    .await;
                if this
                    .update_in(cx, |this, window, cx| this.tick(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        Self {
            dock_area: area,
            services,
            tree,
            search: None,
            tasks: None,
            pi,
            panels: HashMap::new(),
            _subs: vec![dock_sub],
            keymap,
            i18n: I18n::default(),
            status: error.unwrap_or_else(|| "正在索引…".into()),
            side: 0,
            close_pending: None,
            closing_window: false,
            flushing_close: false,
            reading_disk: false,
            palette: None,
        }
    }
    pub fn keymap(&self) -> &Keymap {
        &self.keymap
    }
    pub fn i18n(&self) -> &I18n {
        &self.i18n
    }
    pub fn set_lang(&mut self, lang: Lang) {
        self.i18n = I18n::new(lang);
    }
    pub fn build_palette(&self) -> Vec<gpui_kit::component::command::CommandItem> {
        crate::keymap::build_command_palette(&self.keymap, &self.i18n)
    }
    pub fn show_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = cx.new(|cx| gpui_kit::component::command::CommandState::new(window, cx));
        state.update(cx, |state, cx| state.focus(window, cx));
        self.palette = Some(state);
        cx.notify();
    }
    fn persist(&self, cx: &mut Context<Self>) {
        let dock = PersistedDockState {
            schema_version: 1,
            kit_layout_version: 1,
            dock: self.dock_area.read(cx).dump(cx),
        };
        if let Some(services) = &self.services {
            services.persist(crate::workbench::PersistChange::Dock(dock));
        }
    }
    fn dirty_path(&self, cx: &gpui_kit::App) -> Option<PathBuf> {
        self.services
            .as_ref()?
            .documents
            .borrow()
            .iter()
            .find(|(_, doc)| doc.read(cx).save_state().is_dirty())
            .map(|(path, _)| path.clone())
    }
    fn save(&mut self, path: PathBuf, close: bool, window_close: bool, cx: &mut Context<Self>) {
        let Some(services) = self.services.clone() else {
            return;
        };
        let Some(doc) = services.documents.borrow().get(&path).cloned() else {
            return;
        };
        let receiver = doc.update(cx, |doc, _| doc.request_save(&services.writes));
        match receiver {
            Ok(receiver) => {
                let task = cx.background_spawn(async move { receiver.recv() });
                cx.spawn(async move |this, cx| {
                    let result = task.await;
                    let _ = this.update(cx, |this, cx| {
                        match result {
                            Ok(result) => {
                                let saved = result.is_ok();
                                doc.update(cx, |doc, cx| {
                                    doc.apply_save_result(result);
                                    cx.notify();
                                });
                                if saved && close {
                                    services.emit(WorkbenchEvent::Close(path));
                                }
                                if saved && window_close {
                                    this.close_pending =
                                        this.dirty_path(cx).map(|path| (path, true));
                                }
                            }
                            Err(error) => this.status = error.to_string(),
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
            Err(error) => {
                self.status = error.to_string();
                cx.notify();
            }
        }
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
    fn open(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let root = self.tree.read(cx).root().to_path_buf();
        let task = cx.background_spawn(async move {
            let path = omw_local::normalize_document_path(path);
            let is_directory = path.is_dir();
            let logical_path = path
                .strip_prefix(omw_local::normalize_document_path(root))
                .ok()
                .map(PathBuf::from);
            (path, is_directory, logical_path)
        });
        cx.spawn_in(window, async move |this, cx| {
            let (path, is_directory, logical_path) = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if is_directory {
                    this.side = 0;
                    this.status = path.display().to_string();
                    if let Some(logical_path) = logical_path {
                        this.tree
                            .update(cx, |tree, cx| tree.open_directory(&logical_path, cx));
                    } else {
                        this.status = "目录不在当前 Workspace 内".into();
                    }
                    cx.notify();
                    return;
                }
                this.open_resolved(path, window, cx);
            });
        })
        .detach();
    }
    fn open_resolved(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if extension == "md" || extension == "markdown" {
            if let Some(panel) = self.panels.get(&path) {
                self.dock_area.update(cx, |area, cx| {
                    area.select_panel(PanelId::from(panel.entity_id()), window, cx)
                });
                return;
            }
            let panel = cx.new(|cx| MarkdownPanel::open(path.clone(), window, cx));
            self.dock_area.update(cx, |area, cx| {
                area.add_panel_view(
                    panel_handle(panel.clone()),
                    DockPlacement::Center,
                    None,
                    window,
                    cx,
                )
            });
            self.panels.insert(path.clone(), panel);
            if let Some(services) = &self.services {
                *services.active.borrow_mut() = Some(path.clone());
                let mut recent = services.recent.borrow_mut();
                recent.retain(|p| p != &path);
                recent.insert(0, path);
                recent.truncate(20);
            }
        } else if ["png", "jpg", "jpeg", "gif", "webp", "bmp", "pdf"].contains(&extension.as_str())
        {
            let panel = cx.new(|cx| PreviewPanel::new(TabRef::Preview { path }, window, cx));
            self.dock_area.update(cx, |area, cx| {
                area.add_panel_view(panel_handle(panel), DockPlacement::Center, None, window, cx)
            });
        } else {
            cx.background_spawn(async move {
                let _ = std::process::Command::new("explorer.exe").arg(path).spawn();
            })
            .detach();
        }
    }
    fn close_document(&mut self, path: &PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(panel) = self.panels.remove(path) {
            self.dock_area
                .update(cx, |area, cx| area.remove_panel(panel, window, cx));
        }
        if let Some(services) = &self.services {
            services.documents.borrow_mut().remove(path);
            services.panels.borrow_mut().remove(path);
        }
        self.close_pending = None;
    }
    fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(services) = self.services.clone() else {
            return;
        };
        self.panels = services.panels.borrow().clone();
        let events: Vec<_> = services.events.borrow_mut().drain(..).collect();
        let had_events = !events.is_empty();
        for event in events {
            match event {
                WorkbenchEvent::Open(path) => self.open(path, window, cx),
                WorkbenchEvent::Close(path) => {
                    let dirty = services
                        .documents
                        .borrow()
                        .get(&path)
                        .is_some_and(|d| d.read(cx).save_state().is_dirty());
                    if dirty {
                        self.close_pending = Some((path, false));
                    } else {
                        self.close_document(&path, window, cx);
                    }
                }
                WorkbenchEvent::Task(intent) => self.commit_task(intent, cx),
                WorkbenchEvent::ShowTasks => self.side = 2,
                WorkbenchEvent::ShowPi => self.side = 3,
            }
        }
        if !self.reading_disk {
            let paths: Vec<_> = services
                .documents
                .borrow()
                .iter()
                .filter(|(_, doc)| {
                    !doc.read(cx).task_pending()
                        && !matches!(
                            doc.read(cx).save_state().status(),
                            SaveStatus::Saving { .. }
                        )
                })
                .map(|(p, d)| {
                    let doc = d.read(cx);
                    (
                        p.clone(),
                        doc.revision(),
                        doc.save_state().baseline().cloned(),
                    )
                })
                .collect();
            if !paths.is_empty() {
                self.reading_disk = true;
                let task = cx.background_spawn(async move {
                    paths
                        .into_iter()
                        .filter_map(|(p, r, baseline)| {
                            let snapshot = omw_local::read_snapshot(&p);
                            if let Ok(snapshot) = &snapshot
                                && baseline
                                    .as_ref()
                                    .is_some_and(|b| b.same_content(&snapshot.version))
                            {
                                return None;
                            }
                            Some((p, r, snapshot))
                        })
                        .collect::<Vec<_>>()
                });
                cx.spawn(async move |this, cx| {
                    let results = task.await;
                    let _ = this.update(cx, |this, cx| {
                        this.reading_disk = false;
                        let changed = !results.is_empty();
                        for (path, revision, snapshot) in results {
                            let doc = this
                                .services
                                .as_ref()
                                .and_then(|s| s.documents.borrow().get(&path).cloned());
                            if let Some(doc) = doc {
                                doc.update(cx, |doc, cx| {
                                    if doc.revision() != revision
                                        || doc.task_pending()
                                        || matches!(
                                            doc.save_state().status(),
                                            SaveStatus::Saving { .. }
                                        )
                                    {
                                        return;
                                    }
                                    let event = match snapshot {
                                        Ok(s) => ExternalEvent::Snapshot(s),
                                        Err(error) => ExternalEvent::ReadError(error.to_string()),
                                    };
                                    if let Err(error) = doc.apply_external_event(event) {
                                        this.status = error;
                                    }
                                    cx.notify();
                                });
                            }
                        }
                        if changed {
                            cx.notify();
                        }
                    });
                })
                .detach();
            }
        }
        if self.closing_window
            && !self.flushing_close
            && self.close_pending.is_none()
            && self.dirty_path(cx).is_none()
            && services
                .documents
                .borrow()
                .values()
                .all(|doc| !doc.read(cx).task_pending())
        {
            if let Some(pi) = &self.pi {
                if pi.read(cx).is_connected() {
                    pi.update(cx, |pi, cx| pi.close(cx));
                    return;
                }
            }
            self.flushing_close = true;
            self.persist(cx);
            let (reply, result) = std::sync::mpsc::channel();
            services.persist(crate::workbench::PersistChange::Flush(reply));
            let task = cx.background_spawn(async move {
                result.recv().map_err(|e| e.to_string()).and_then(|r| r)
            });
            cx.spawn_in(window, async move |this, cx| {
                let result = task.await;
                let _ = this.update_in(cx, |this, window, cx| {
                    this.flushing_close = false;
                    if !this.closing_window {
                        return;
                    }
                    if let Err(error) = result {
                        this.closing_window = false;
                        this.status = error;
                        cx.notify();
                        return;
                    }
                    if let Some(path) = this.dirty_path(cx) {
                        this.close_pending = Some((path, true));
                        cx.notify();
                        return;
                    }
                    if this.close_pending.is_some()
                        || this.services.as_ref().is_some_and(|services| {
                            services
                                .documents
                                .borrow()
                                .values()
                                .any(|doc| doc.read(cx).task_pending())
                        })
                        || this
                            .pi
                            .as_ref()
                            .is_some_and(|pi| pi.read(cx).is_connected())
                    {
                        cx.notify();
                        return;
                    }
                    window.remove_window();
                });
            })
            .detach();
        }
        if had_events {
            cx.notify();
        }
    }
    fn commit_task(&mut self, intent: super::tasks::TaskIntent, cx: &mut Context<Self>) {
        let super::tasks::TaskIntent {
            path,
            version,
            anchor,
            update,
        } = intent;
        let Some(services) = self.services.clone() else {
            return;
        };
        let doc = services.documents.borrow().get(&path).cloned();
        if doc.as_ref().is_some_and(|d| {
            d.read(cx).task_pending()
                || matches!(d.read(cx).save_state().status(), SaveStatus::Saving { .. })
        }) {
            self.status = "请等待当前保存完成".into();
            return;
        }
        if let Some(doc) = &doc {
            doc.update(cx, |doc, cx| {
                doc.set_task_pending(true);
                cx.notify();
            });
        }
        let revision = doc.as_ref().map(|d| d.read(cx).revision());
        let working = doc.as_ref().map(|d| d.read(cx).session());
        let writes = services.writes.clone();
        let task_path = path.clone();
        let document_version = doc
            .as_ref()
            .and_then(|d| d.read(cx).save_state().baseline().cloned());
        let task = cx.background_spawn(async move {
            let snapshot = omw_local::read_snapshot(&task_path).map_err(|e| e.to_string())?;
            if !snapshot.version.same_content(&version)
                || document_version
                    .as_ref()
                    .is_some_and(|baseline| !snapshot.version.same_content(baseline))
            {
                return Err("任务来源已变化；请刷新后重试".into());
            }
            let parsed = omw_document::parse(&snapshot.bytes).map_err(|e| e.to_string())?;
            let item =
                omw_document::locate_task_by_anchor(&parsed, &anchor).map_err(|e| e.to_string())?;
            let merged = if let Some(working) = &working {
                Some(
                    working
                        .lock()
                        .map_err(|_| "document lock poisoned")?
                        .with_committed_task_field(item.node, &update)?,
                )
            } else {
                None
            };
            let bytes = omw_document::apply_task_update_to_baseline(&parsed, &anchor, &update)
                .map_err(|e| e.to_string())?;
            let receiver = writes
                .submit_task_save(
                    omw_local::DocumentKey::new(&task_path),
                    &task_path,
                    Some(snapshot.version),
                    bytes,
                    revision.unwrap_or(0),
                )
                .map_err(|e| format!("{e:?}"))?;
            let ack = receiver
                .recv()
                .map_err(|e| e.to_string())?
                .map_err(|e| e.to_string())?;
            if let (Some(working), Some(merged)) = (working, merged) {
                let mut session = working.lock().map_err(|_| "document lock poisoned")?;
                *session = merged;
            }
            Ok::<_, String>(ack)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(doc) = &doc {
                    doc.update(cx, |doc, cx| {
                        doc.set_task_pending(false);
                        cx.notify();
                    });
                }
                match result {
                    Ok(ack) => {
                        if let Some(doc) = doc {
                            doc.update(cx, |doc, cx| {
                                doc.apply_task_commit(ack);
                                cx.notify();
                            });
                        }
                        this.status = "任务字段已保存；其他正文仍未保存".into();
                        if let Some(tasks) = &this.tasks {
                            tasks.update(cx, |tasks, cx| tasks.reload(cx));
                        }
                    }
                    Err(error) => this.status = error,
                }
                cx.notify();
            });
        })
        .detach();
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
            1 => {
                if let Some(search) = &self.search {
                    sidebar.child(search.clone())
                } else {
                    sidebar.child("正在建立索引…")
                }
            }
            2 => {
                if let Some(tasks) = &self.tasks {
                    sidebar.child(tasks.clone())
                } else {
                    sidebar.child("正在加载任务…")
                }
            }
            3 => {
                if let Some(pi) = &self.pi {
                    sidebar.child(pi.clone())
                } else {
                    sidebar.child("Pi 服务不可用")
                }
            }
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
                let path = this
                    .services
                    .as_ref()
                    .and_then(|s| s.active.borrow().clone());
                if let Some(path) = path {
                    this.save(path, false, false, cx);
                }
            }))
            .on_action(cx.listener(|this, &CloseTab, _, cx| {
                if let Some(s) = &this.services {
                    if let Some(path) = s.active.borrow().clone() {
                        s.emit(WorkbenchEvent::Close(path));
                    }
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
                                    let path = this
                                        .services
                                        .as_ref()
                                        .and_then(|s| s.active.borrow().clone());
                                    if let Some(path) = path {
                                        this.save(path, false, false, cx);
                                    }
                                }
                                1 => {
                                    if let Some(s) = &this.services {
                                        if let Some(path) = s.active.borrow().clone() {
                                            s.emit(WorkbenchEvent::Close(path));
                                        }
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
        if let Some((path, window_close)) = self.close_pending.clone() {
            let save_path = path.clone();
            let discard_path = path.clone();
            body = body.child(
                div()
                    .p_3()
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(format!("{} 有未保存修改", path.display()))
                    .child(
                        Button::new("close-save")
                            .label("保存")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.save(save_path.clone(), !window_close, window_close, cx)
                            })),
                    )
                    .child(
                        Button::new("close-discard")
                            .label("放弃")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.close_document(&discard_path, window, cx);
                                if window_close {
                                    this.close_pending = this.dirty_path(cx).map(|p| (p, true));
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("close-cancel")
                            .label("取消")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.close_pending = None;
                                this.closing_window = false;
                                cx.notify();
                            })),
                    ),
            );
        }
        body
    }
}
