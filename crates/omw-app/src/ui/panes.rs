use super::{
    preview::{PreviewClient, PreviewHandle, PreviewUpdate},
    tabs::TabRef,
};
pub use crate::home::view::HomePanel;
use crate::{
    document_session::{CloseAction, CloseDecision, DocumentSaveState, close_action},
    editor::{EditorDocument, EditorSurface},
    workbench::{Workbench, WorkbenchEvent},
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::{
    button::Button,
    dock::{BasePanel, DockAreaState, Panel, PanelEvent, PanelInfo, PanelState, panel_handle},
};
use gpui_kit::{
    AnyView, App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, Render,
    Window, div, prelude::*,
};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PersistedDockState {
    pub schema_version: u32,
    pub kit_layout_version: u32,
    pub dock: DockAreaState,
}
impl Default for PersistedDockState {
    fn default() -> Self {
        Self {
            schema_version: 1,
            kit_layout_version: 1,
            dock: DockAreaState::default(),
        }
    }
}
fn persisted(name: &str, tab: &TabRef) -> PanelState {
    PanelState {
        panel_name: name.into(),
        children: vec![],
        info: PanelInfo::Panel(serde_json::to_value(tab).unwrap_or_default()),
    }
}
pub struct MarkdownPanel {
    focus: FocusHandle,
    document: Option<Entity<EditorDocument>>,
    surface: Option<Entity<EditorSurface>>,
    tab_ref: TabRef,
    error: Option<String>,
}
impl MarkdownPanel {
    pub fn new(
        document: Entity<EditorDocument>,
        tab_ref: TabRef,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let surface = Self::surface(document.clone(), cx);
        Self {
            focus: cx.focus_handle(),
            document: Some(document),
            surface: Some(surface),
            tab_ref,
            error: None,
        }
    }
    fn surface(document: Entity<EditorDocument>, cx: &mut Context<Self>) -> Entity<EditorSurface> {
        let queue = cx.try_global::<Workbench>().map(|s| s.writes.clone());
        cx.new(|cx| {
            let surface = EditorSurface::from_document(document, cx);
            if let Some(queue) = queue {
                surface.with_write_queue(queue)
            } else {
                surface
            }
        })
    }
    pub fn open(path: PathBuf, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let services = cx.global::<Workbench>().clone();
        services
            .panels
            .borrow_mut()
            .insert(path.clone(), cx.entity());
        let mut this = Self {
            focus: cx.focus_handle(),
            document: None,
            surface: None,
            tab_ref: TabRef::Markdown { path: path.clone() },
            error: None,
        };
        if let Some(doc) = services.documents.borrow().get(&path).cloned() {
            this.surface = Some(Self::surface(doc.clone(), cx));
            this.document = Some(doc);
            return this;
        }
        let task_path = path.clone();
        let task = cx.background_spawn(async move {
            let snapshot = omw_local::read_snapshot(&task_path).map_err(|e| e.to_string())?;
            let parsed = omw_document::parse(&snapshot.bytes).map_err(|e| e.to_string())?;
            Ok::<_, String>((snapshot, parsed))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok((snapshot, parsed)) => {
                        let existing = services.documents.borrow().get(&path).cloned();
                        let doc = existing.unwrap_or_else(|| {
                            cx.new(|_| {
                                EditorDocument::from_snapshot(&parsed, path.clone(), &snapshot)
                            })
                        });
                        services.documents.borrow_mut().insert(path, doc.clone());
                        this.surface = Some(Self::surface(doc.clone(), cx));
                        this.document = Some(doc);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        this
    }
    pub fn tab_ref(&self) -> &TabRef {
        &self.tab_ref
    }
    pub fn save_state(&self, cx: &App) -> Option<DocumentSaveState> {
        self.document
            .as_ref()
            .map(|d| d.read(cx).save_state().clone())
    }
    pub fn close_action(&self, decision: CloseDecision, cx: &App) -> CloseAction {
        self.save_state(cx)
            .map(|s| close_action(&s, decision))
            .unwrap_or(CloseAction::Close)
    }
}
impl BasePanel for MarkdownPanel {
    fn panel_name(&self) -> &'static str {
        "MarkdownPanel"
    }
    // The explicit close action offers save/discard/cancel before Dock mutation.
    fn closable(&self, _: &App) -> bool {
        false
    }
    fn set_active(&mut self, active: bool, _: &mut Window, cx: &mut Context<Self>) {
        if active {
            if let TabRef::Markdown { path } = &self.tab_ref {
                *cx.global::<Workbench>().active.borrow_mut() = Some(path.clone());
            }
        }
    }
    fn dump(&self, _: &App) -> PanelState {
        persisted(self.panel_name(), &self.tab_ref)
    }
}
impl Panel for MarkdownPanel {
    fn title(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dirty = self
            .document
            .as_ref()
            .is_some_and(|d| d.read(cx).save_state().is_dirty());
        format!(
            "{}{}",
            self.tab_ref.display_name(),
            if dirty { " *" } else { "" }
        )
    }
}
impl EventEmitter<PanelEvent> for MarkdownPanel {}
impl Focusable for MarkdownPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for MarkdownPanel {
    fn render(&mut self, _: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let path = if let TabRef::Markdown { path } = &self.tab_ref {
            path.clone()
        } else {
            PathBuf::new()
        };
        let mut body = div()
            .id("markdown-pane")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .child(
                Button::new("close-document")
                    .label("关闭 / Close")
                    .on_click(move |_, _, cx| {
                        cx.global::<Workbench>()
                            .emit(WorkbenchEvent::Close(path.clone()))
                    }),
            );
        if let Some(surface) = &self.surface {
            body = body.child(surface.clone());
        } else {
            body = body.child(self.error.clone().unwrap_or_else(|| "正在打开文件…".into()));
        }
        body
    }
}

pub struct PreviewPanel {
    focus: FocusHandle,
    tab_ref: TabRef,
    client: Option<PreviewClient>,
    handle: Option<PreviewHandle>,
    document_id: Option<u64>,
    page: u32,
    pages: u32,
    zoom: f32,
    generation: u64,
    bitmap: Option<Arc<gpui_kit::RenderImage>>,
    status: String,
}
impl PreviewPanel {
    pub fn new(tab_ref: TabRef, _: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            focus: cx.focus_handle(),
            tab_ref,
            client: None,
            handle: None,
            document_id: None,
            page: 0,
            pages: 0,
            zoom: 1.,
            generation: 1,
            bitmap: None,
            status: "正在加载…".into(),
        };
        let Some(services) = cx.try_global::<Workbench>().cloned() else {
            this.status = "预览服务未启动".into();
            return this;
        };
        let worker = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.join("omw-preview-worker.exe")));
        let Some(worker) = worker else {
            this.status = "无法定位预览进程".into();
            return this;
        };
        let _enter = services.runtime.enter();
        match PreviewClient::spawn(worker) {
            Ok((mut client, handle)) => {
                if let Some(mut updates) = client.update_rx.take() {
                    cx.spawn(async move |this, cx| {
                        while let Some(update) = updates.recv().await {
                            if this
                                .update(cx, |this, cx| {
                                    this.apply(update);
                                    cx.notify();
                                })
                                .is_err()
                            {
                                break;
                            }
                        }
                    })
                    .detach();
                }
                let path = match &this.tab_ref {
                    TabRef::Preview { path } => path.clone(),
                    _ => PathBuf::new(),
                };
                let h = handle.clone();
                cx.spawn(async move |this, cx| {
                    let result = h.open(path).await;
                    let _ = this.update(cx, |this, cx| {
                        match result {
                            Ok(omw_preview_worker::PreviewResponse::Opened {
                                document_id,
                                page_count,
                                ..
                            }) => {
                                this.document_id = Some(document_id);
                                this.pages = page_count;
                                this.render_page(cx);
                            }
                            Ok(omw_preview_worker::PreviewResponse::Error {
                                code,
                                message,
                                ..
                            }) => this.status = format!("{code}: {message}"),
                            Ok(_) => this.status = "预览协议返回错误响应".into(),
                            Err(error) => this.status = error,
                        }
                        cx.notify();
                    });
                })
                .detach();
                this.handle = Some(handle);
                this.client = Some(client);
            }
            Err(error) => this.status = error,
        }
        this
    }
    fn apply(&mut self, update: PreviewUpdate) {
        match update {
            PreviewUpdate::Bitmap {
                document_id,
                generation_tag,
                width,
                height,
                mut data,
                ..
            } if Some(document_id) == self.document_id && generation_tag == self.generation => {
                if data.len()
                    != (width as usize)
                        .saturating_mul(height as usize)
                        .saturating_mul(4)
                {
                    self.status = "无效图片长度".into();
                    return;
                }
                for pixel in data.chunks_exact_mut(4) {
                    pixel.swap(0, 2);
                }
                if let Some(buffer) = image::RgbaImage::from_raw(width, height, data) {
                    self.bitmap = Some(Arc::new(gpui_kit::RenderImage::new(vec![
                        image::Frame::new(buffer),
                    ])));
                    self.status = format!(
                        "{} / {} · {}%",
                        self.page + 1,
                        self.pages,
                        (self.zoom * 100.) as u32
                    );
                }
            }
            PreviewUpdate::Error { code, message, .. } => {
                self.status = format!("{code}: {message}")
            }
            PreviewUpdate::Disconnected { .. } => {
                self.status = "预览进程已退出；关闭并重新打开文件可重试".into()
            }
            _ => {}
        }
    }
    fn render_page(&mut self, cx: &mut Context<Self>) {
        let (Some(handle), Some(document)) = (self.handle.clone(), self.document_id) else {
            return;
        };
        self.generation += 1;
        let generation = self.generation;
        let page = self.page;
        let size = [(800. * self.zoom) as u32, (1000. * self.zoom) as u32];
        self.status = "正在渲染…".into();
        cx.spawn(async move |this, cx| {
            if let Err(error) = handle.render(document, page, size, generation).await {
                let _ = this.update(cx, |this, cx| {
                    this.status = error;
                    cx.notify();
                });
            }
        })
        .detach();
    }
}
impl BasePanel for PreviewPanel {
    fn panel_name(&self) -> &'static str {
        "PreviewPanel"
    }
    fn dump(&self, _: &App) -> PanelState {
        persisted(self.panel_name(), &self.tab_ref)
    }
}
impl Panel for PreviewPanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.tab_ref.display_name()
    }
}
impl EventEmitter<PanelEvent> for PreviewPanel {}
impl Focusable for PreviewPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for PreviewPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut body = div()
            .id("preview-pane")
            .test_support()
            .size_full()
            .flex()
            .flex_col()
            .track_focus(&self.focus)
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("previous-page")
                            .label("上一页")
                            .on_click(cx.listener(|this, _, _, cx| {
                                if this.page > 0 {
                                    this.page -= 1;
                                    this.render_page(cx);
                                }
                            })),
                    )
                    .child(
                        Button::new("next-page")
                            .label("下一页")
                            .on_click(cx.listener(|this, _, _, cx| {
                                if this.page + 1 < this.pages {
                                    this.page += 1;
                                    this.render_page(cx);
                                }
                            })),
                    )
                    .child(Button::new("zoom-out").label("−").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.zoom = (this.zoom / 1.25).max(0.25);
                            this.render_page(cx);
                        },
                    )))
                    .child(Button::new("zoom-in").label("+").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.zoom = (this.zoom * 1.25).min(2.);
                            this.render_page(cx);
                        },
                    )))
                    .child(
                        div()
                            .id("preview-status")
                            .role(gpui_kit::Role::Status)
                            .test_support()
                            .aria_label(self.status.clone())
                            .child(self.status.clone()),
                    ),
            );
        if let Some(image) = &self.bitmap {
            body = body.child(
                div()
                    .id("preview-image")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .child(
                        gpui_kit::img(image.clone())
                            .w_full()
                            .h_full()
                            .object_fit(gpui_kit::ObjectFit::Contain),
                    ),
            );
        }
        body
    }
}
pub fn register_pane_factories(cx: &mut App, root: PathBuf) {
    gpui_kit::component::dock::register_panel(cx, "MarkdownPanel", |state, window, cx| {
        let tab = match state.info() {
            PanelInfo::Panel(value) => serde_json::from_value::<TabRef>(value.clone()).ok(),
            _ => None,
        };
        let path = match tab {
            Some(TabRef::Markdown { path }) => path,
            _ => PathBuf::new(),
        };
        panel_handle(cx.new(|cx| MarkdownPanel::open(path, window, cx)))
    });
    gpui_kit::component::dock::register_panel(cx, "PreviewPanel", |state, window, cx| {
        let tab = match state.info() {
            PanelInfo::Panel(value) => serde_json::from_value::<TabRef>(value.clone()).ok(),
            _ => None,
        };
        panel_handle(cx.new(|cx| {
            PreviewPanel::new(
                tab.unwrap_or(TabRef::Preview {
                    path: PathBuf::new(),
                }),
                window,
                cx,
            )
        }))
    });
    gpui_kit::component::dock::register_panel(cx, "HomePanel", move |_, window, cx| {
        panel_handle(cx.new(|cx| HomePanel::new(root.clone(), window, cx)))
    });
}
pub fn panel_for_tab(
    tab: TabRef,
    document: Option<Entity<EditorDocument>>,
    window: &mut Window,
    cx: &mut Context<impl Render>,
    root: PathBuf,
) -> AnyView {
    match tab {
        TabRef::Markdown { path } => {
            let path = path.clone();
            cx.new(|cx| {
                if let Some(document) = document {
                    MarkdownPanel::new(
                        document,
                        TabRef::Markdown { path: path.clone() },
                        window,
                        cx,
                    )
                } else {
                    MarkdownPanel::open(path, window, cx)
                }
            })
            .into()
        }
        TabRef::Preview { .. } => cx.new(|cx| PreviewPanel::new(tab, window, cx)).into(),
        TabRef::Home => cx.new(|cx| HomePanel::new(root, window, cx)).into(),
    }
}
