use super::toolbar::EditorToolbarCommand;
use gpui_kit::component::input::{Redo, Undo};
use gpui_kit::{
    Context, Entity, EntityInputHandler as _, Focusable, FocusHandle, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement, Styled as _, Subscription, Window, div,
};
use crate::documents::{DocumentStore, PageCommand, PageDocument, PageSnapshot, SourceSelection};
use crate::parse::parse_for_view;
mod projection;

use self::projection::{Bias, LeafId, LeafKind, ParsedProjection, ProjectedLeaf};
mod surface;

use self::surface::MarkdownElement;
mod input;
mod tables;

/// GPUI presentation for a shared PageDocument.
pub struct EditorSurface {
    document: Entity<PageDocument>,
    documents: Entity<DocumentStore>,
    _document_subscription: Subscription,
    focus: FocusHandle,
    input_error: Option<String>,
    // new projection state (derived, cancellable, not in PageSnapshot)
    projection: Option<std::sync::Arc<ParsedProjection>>,
    projection_generation: u64,
    active_leaf: Option<LeafId>,
    composition: Option<Composition>,
    // retained for observer / external / save
    document_revision: Option<u64>,
    external_epoch: Option<u64>,
    selection: SourceSelection,
    // old editor state fields removed (no dual path)
}

// Placeholder Composition (will flesh in input.rs)
#[derive(Clone, Debug)]
pub(crate) struct Composition {
    pub base_revision: u64,
    pub base_epoch: u64,
    pub source_range: std::ops::Range<usize>,
    pub preedit: String,
    pub ime_selection: std::ops::Range<usize>,
    pub pre_selection: SourceSelection,
}

impl EditorSurface {
    pub fn new(document: Entity<PageDocument>, documents: Entity<DocumentStore>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&document, |_, _, cx| cx.notify());
        Self {
            document,
            documents,
            _document_subscription: subscription,
            focus: cx.focus_handle(),
            input_error: None,
            projection: None,
            projection_generation: 0,
            active_leaf: None,
            composition: None,
            document_revision: None,
            external_epoch: None,
            selection: SourceSelection::default(),
        }
    }

    fn snapshot(&self, cx: &gpui_kit::App) -> Option<std::rc::Rc<PageSnapshot>> {
        self.document.read(cx).snapshot().ok()
    }

    pub fn request_save(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.document.read(cx).path().map(ToOwned::to_owned) else {
            self.input_error = Some("文档没有可保存的路径".to_string()); cx.notify(); return;
        };
        match self.documents.update(cx, |store, cx| store.save(path, cx)) {
            Ok(()) => self.input_error = None,
            Err(error) => self.input_error = Some(error.to_string()),
        }
        cx.notify();
    }

    pub fn input_error(&self) -> Option<&str> { self.input_error.as_deref() }

    pub fn handle_toolbar_command(&mut self, command: EditorToolbarCommand, window: &mut Window, cx: &mut Context<Self>) {
        if self.document.read(cx).is_busy() { return; }
        if matches!(command, EditorToolbarCommand::Save) {
            self.request_save(cx);
            return;
        }
        match command {
            EditorToolbarCommand::Save => return,
            EditorToolbarCommand::Undo | EditorToolbarCommand::Redo => {
                let page_command = if matches!(command, EditorToolbarCommand::Undo) { PageCommand::Undo } else { PageCommand::Redo };
                match self.document.update(cx, |doc, cx| doc.apply(page_command, cx)) {
                    Ok(outcome) => {
                        self.document_revision = Some(outcome.revision);
                        self.selection = outcome.selection.unwrap_or_default();
                        self.projection = None; // force rebuild on next render
                    }
                    Err(error) => self.input_error = Some(error.to_string()),
                }
            }
            EditorToolbarCommand::Bold | EditorToolbarCommand::Italic | EditorToolbarCommand::Strikethrough | EditorToolbarCommand::PasteMarkdown => {
                // stub: real impl will compute from active leaf geometry + call replace_source_range
                // for now just notify (no crash)
                self.input_error = Some("格式命令待原生输入路径实现".to_string());
            }
        }
        cx.notify();
    }
}

impl Render for EditorSurface {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // ensure projection (sync stub; later spawn Task + cancel token + compare epochs/generations)
        if let Some(snapshot) = self.snapshot(cx) {
            let rev = snapshot.revision();
            if self.external_epoch != Some(snapshot.external_epoch()) {
                self.projection = None;
                self.active_leaf = None;
                self.composition = None;
                self.selection = SourceSelection::default();
                self.external_epoch = Some(snapshot.external_epoch());
            }
            if self.projection.as_ref().map_or(true, |p| p.version != rev) {
                if let Ok((_, _meta)) = parse_for_view(snapshot.source().to_string().as_bytes()) {
                    let src_str = snapshot.source().to_string();
                    let rope_for_proj = ropey::Rope::from(&src_str[..]);
                    let proj = projection::build_projection_from_tree(
                        rev,
                        None,
                        rope_for_proj,
                        snapshot.source().to_string().as_bytes(),
                        &crate::source_map::SourceMap::new(&[]),
                    );
                    self.projection = Some(std::sync::Arc::new(proj));
                    self.projection_generation = rev;
                    self.document_revision = Some(rev);
                    self.external_epoch = Some(snapshot.external_epoch());
                }
            }
        }

        let task_pending = self.document.read(cx).is_busy();
        let toolbar = super::toolbar::editor_toolbar(cx.entity());
        let input_error = self.input_error.clone();

        let mut shell = div().id("markdown-editor-shell").key_context("Editor").w_full().h_full().flex().flex_col()
            .child(toolbar);
        if let Some(error) = &input_error {
            shell = shell.child(div().id("editor-input-error").child(error.clone()));
        }

        if let Some(proj) = self.projection.clone() {
            let active = self.active_leaf;
            let elem = MarkdownElement::new(proj, active);
            shell = shell.child(
                div().id("markdown-viewport").flex_1().min_h_0().min_w_0().w_full()
                    .child(elem)
            );
        } else {
            shell = shell.child(div().id("markdown-loading").child("Loading..."));
        }

        // Native input via EntityInputHandler on EditorSurface (installed in MarkdownElement paint or via ElementInputHandler in gpui)
        // keyboard via host + replace_source_range
        shell = shell.capture_action(cx.listener(|this, _: &Undo, window, cx| {
            this.handle_toolbar_command(EditorToolbarCommand::Undo, window, cx);
            cx.stop_propagation();
        }));
        shell = shell.capture_action(cx.listener(|this, _: &Redo, window, cx| {
            this.handle_toolbar_command(EditorToolbarCommand::Redo, window, cx);
            cx.stop_propagation();
        }));
        shell.aria_description(input_error.unwrap_or_default()).track_focus(&self.focus).into_any_element()
    }
}

// Note: request_save, document observer (via _document_subscription), external_epoch handling retained as required.
// Dual EditorState paths fully excised. No tests added.
