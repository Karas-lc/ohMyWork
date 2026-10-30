use super::tabs::TabRef;
pub use crate::views::home::HomePanel;
use crate::context::WorkspaceContext;
use editor::documents::{DocumentEvent, DocumentStore, PageDocument};
use editor::{EditorSurface, EditorToolbarCommand};
use gpui_kit::component::{
    button::Button,
    dock::{BasePanel, Panel, PanelEvent, PanelInfo, PanelState, panel_handle},
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    Render, Subscription, Window, div, prelude::*,
};
use gpui_kit::TestSupportExt as _;
use std::{collections::HashMap, path::{Path, PathBuf}};

#[derive(Default)]
pub struct PanelRegistry {
    markdown: HashMap<PathBuf, Entity<MarkdownPanel>>,
}

impl PanelRegistry {
    pub fn markdown(&self, path: &Path) -> Option<Entity<MarkdownPanel>> {
        self.markdown.get(path).cloned()
    }

    pub fn remove(&mut self, path: &Path) -> Option<Entity<MarkdownPanel>> {
        self.markdown.remove(path)
    }
}

pub(crate) fn persisted(name: &str, tab: &TabRef) -> PanelState {
    PanelState {
        panel_name: name.into(),
        children: vec![],
        info: PanelInfo::Panel(serde_json::to_value(tab).unwrap_or_default()),
    }
}
pub struct MarkdownPanel {
    focus: FocusHandle,
    documents: Entity<DocumentStore>,
    session: Entity<crate::session::WorkspaceSession>,
    document: Option<Entity<PageDocument>>,
    surface: Option<Entity<EditorSurface>>,
    tab_ref: TabRef,
    error: Option<String>,
    _document_subscription: Option<Subscription>,
}

impl MarkdownPanel {
    fn surface(
        document: Entity<PageDocument>,
        documents: Entity<DocumentStore>,
        cx: &mut Context<Self>,
    ) -> Entity<EditorSurface> {
        cx.new(|cx| EditorSurface::new(document, documents, cx))
    }

    fn forward_surface_command(
        &mut self,
        command: EditorToolbarCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(surface) = &self.surface {
            surface.update(cx, |editor, cx| editor.handle_toolbar_command(command, window, cx));
        }
    }

    pub fn open(
        path: PathBuf,
        session: Entity<crate::session::WorkspaceSession>,
        documents: Entity<DocumentStore>,
        registry: Entity<PanelRegistry>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let panel = cx.entity();
        registry.update(cx, |registry, _| {
            registry.markdown.insert(path.clone(), panel);
        });
        let mut this = Self {
            focus: cx.focus_handle(),
            documents: documents.clone(),
            session,
            document: None,
            surface: None,
            tab_ref: TabRef::Markdown { path: path.clone() },
            error: None,
            _document_subscription: None,
        };
        let observed_path = path.clone();
        this._document_subscription = Some(cx.subscribe(
            &documents,
            move |this, store, event: &DocumentEvent, cx| {
                if event_path(event) == Some(&observed_path) {
                    match event {
                        DocumentEvent::Failed { message, .. } => {
                            this.error = Some(message.clone());
                        }
                        DocumentEvent::Opened(_)
                        | DocumentEvent::Changed(_)
                        | DocumentEvent::Saved(_)
                        | DocumentEvent::TaskCommitted(_) => {
                            if this.document.is_none() {
                                if let Some(document) = store.read(cx).get(&observed_path) {
                                    this.surface = Some(Self::surface(
                                        document.clone(),
                                        this.documents.clone(),
                                        cx,
                                    ));
                                    this.document = Some(document);
                                    this.error = None;
                                }
                            }
                        }
                        DocumentEvent::Released(_) => {}
                    }
                    cx.notify();
                }
            },
        ));
        if let Some(document) = documents.read(cx).get(&path) {
            this.surface = Some(Self::surface(document.clone(), documents.clone(), cx));
            this.document = Some(document);
        } else {
            let _ = documents.update(cx, |store, cx| store.open(path, cx));
        }
        this
    }

}

fn event_path(event: &DocumentEvent) -> Option<&PathBuf> {
    match event {
        DocumentEvent::Opened(path)
        | DocumentEvent::Saved(path)
        | DocumentEvent::TaskCommitted(path)
        | DocumentEvent::Changed(path)
        | DocumentEvent::Released(path) => Some(path),
        DocumentEvent::Failed { path, .. } => Some(path),
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
                self.session.update(cx, |session, cx| session.activate(path.clone(), cx));
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
            .is_some_and(|document| document.read(cx).is_dirty());
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use crate::keymap::{Bold, Italic, PasteMarkdown, Redo, Strikethrough, Undo};

        let path = if let TabRef::Markdown { path } = &self.tab_ref {
            path.clone()
        } else {
            PathBuf::new()
        };
        let closing = cx.global::<WorkspaceContext>().closing();
        let mut body = div()
            .id("markdown-pane")
            .test_support()
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .child(
                Button::new("close-document")
                    .label("关闭 / Close")
                    .on_click(move |_, _, cx| {
                        closing.update(cx, |closing, cx| closing.request_tab(path.clone(), cx));
                    }),
            );
        if let Some(surface) = &self.surface {
            let surface_element = div()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .w_full()
                .child(surface.clone())
                .capture_action(cx.listener(|this, _: &Undo, window, cx| {
                    this.forward_surface_command(EditorToolbarCommand::Undo, window, cx);
                    cx.stop_propagation();
                }))
                .capture_action(cx.listener(|this, _: &Redo, window, cx| {
                    this.forward_surface_command(EditorToolbarCommand::Redo, window, cx);
                    cx.stop_propagation();
                }))
                .capture_action(cx.listener(|this, _: &Bold, window, cx| {
                    this.forward_surface_command(EditorToolbarCommand::Bold, window, cx);
                    cx.stop_propagation();
                }))
                .capture_action(cx.listener(|this, _: &Italic, window, cx| {
                    this.forward_surface_command(EditorToolbarCommand::Italic, window, cx);
                    cx.stop_propagation();
                }))
                .capture_action(cx.listener(|this, _: &Strikethrough, window, cx| {
                    this.forward_surface_command(EditorToolbarCommand::Strikethrough, window, cx);
                    cx.stop_propagation();
                }))
                .capture_action(cx.listener(|this, _: &PasteMarkdown, window, cx| {
                    this.forward_surface_command(EditorToolbarCommand::PasteMarkdown, window, cx);
                    cx.stop_propagation();
                }));
            body = body.child(surface_element);
        } else {
            body = body.child(self.error.clone().unwrap_or_else(|| "正在打开文件…".into()));
        }
        body
    }
}

pub fn register_pane_factories(
    cx: &mut App,
    registry: Entity<PanelRegistry>,
    documents: Entity<DocumentStore>,
    homepage: Entity<crate::homepage::Homepage>,
    tasks: Entity<crate::queries::TasksModel>,
    session: Entity<crate::session::WorkspaceSession>,
) {
    let markdown_session = session.clone();
    let markdown_registry = registry.clone();
    let markdown_documents = documents.clone();
    gpui_kit::component::dock::register_panel(cx, "MarkdownPanel", move |state, window, cx| {
        let tab = match state.info() {
            PanelInfo::Panel(value) => serde_json::from_value::<TabRef>(value.clone()).ok(),
            _ => None,
        };
        let path = match tab {
            Some(TabRef::Markdown { path }) => path,
            _ => PathBuf::new(),
        };
        panel_handle(cx.new(|cx| MarkdownPanel::open(
            path,
            markdown_session.clone(),
            markdown_documents.clone(),
            markdown_registry.clone(),
            window,
            cx,
        )))
    });
    gpui_kit::component::dock::register_panel(cx, "PreviewPanel", move |state, _window, cx| {
        let tab = match state.info() {
            PanelInfo::Panel(value) => serde_json::from_value::<TabRef>(value.clone()).ok(),
            _ => None,
        };
        let path = match tab {
            Some(TabRef::Preview { path }) => path,
            _ => PathBuf::new(),
        };
        let context = cx.global::<WorkspaceContext>().clone();
        let session = context.new_preview(path.clone(), cx);
        panel_handle(cx.new(|cx| crate::views::preview::PreviewPanel::new(
            TabRef::Preview { path },
            session,
            cx,
        )))
    });
    gpui_kit::component::dock::register_panel(cx, "HomePanel", move |_, window, cx| {
        panel_handle(cx.new(|cx| HomePanel::new(homepage.clone(), tasks.clone(), session.clone(), window, cx)))
    });
}
