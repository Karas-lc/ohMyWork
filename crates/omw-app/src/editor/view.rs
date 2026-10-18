use super::toolbar::EditorToolbarCommand;
use gpui_kit::AppContext as _;

use crate::document_session::{
    DocumentSaveState, ExternalEvent, ExternalEventOutcome, SaveRequestError, SaveResult,
    apply_save_result, reduce_external_event, submit_save,
};
use gpui_kit::{
    Bounds, ClipboardItem, Context, ElementInputHandler, Entity, EntityInputHandler, FocusHandle,
    InteractiveElement as _, IntoElement, KeyDownEvent, ListAlignment, ListState,
    ParentElement as _, Render, Role, StatefulInteractiveElement as _, Styled as _, Subscription,
    TestSupportExt as _, UTF16Selection, Window, canvas, div, fill, hsla, list, px,
};
use omw_document::{
    ByteRange, DocumentSession, EditSession, MarkKind, NodeId, NodeKind, ParseError, VisualPos,
    VisualRange, VisualSelection, parse,
};
use omw_local::{DiskSnapshot, WriteQueue};
use std::{
    cell::RefCell,
    collections::HashMap,
    ops::Range,
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc, Mutex, TryLockError,
        mpsc::{self, Receiver, Sender},
    },
    thread,
};
use unicode_segmentation::UnicodeSegmentation as _;

/// One shared in-memory editing model may be referenced by multiple independent views.
pub struct EditorDocument {
    session: Arc<Mutex<EditSession>>,
    revision: u64,
    path: Option<PathBuf>,
    save_state: DocumentSaveState,
    task_pending: bool,
}
impl EditorDocument {
    pub fn from_document(document: &DocumentSession) -> Self {
        Self {
            session: Arc::new(Mutex::new(EditSession::from_document(document))),
            revision: 0,
            path: None,
            save_state: DocumentSaveState::new(None),
            task_pending: false,
        }
    }
    pub fn from_snapshot(
        document: &DocumentSession,
        path: impl Into<PathBuf>,
        snapshot: &DiskSnapshot,
    ) -> Self {
        Self {
            session: Arc::new(Mutex::new(EditSession::from_document(document))),
            revision: 0,
            path: Some(path.into()),
            save_state: DocumentSaveState::from_snapshot(snapshot),
            task_pending: false,
        }
    }
    pub fn session(&self) -> Arc<Mutex<EditSession>> {
        self.session.clone()
    }
    pub fn task_pending(&self) -> bool {
        self.task_pending
    }
    pub fn set_task_pending(&mut self, pending: bool) {
        self.task_pending = pending;
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn save_state(&self) -> &DocumentSaveState {
        &self.save_state
    }
    pub fn mark_changed(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.save_state.mark_edited(self.revision);
    }
    pub fn request_save(
        &mut self,
        queue: &WriteQueue,
    ) -> Result<Receiver<SaveResult>, SaveRequestError> {
        if self.task_pending {
            return Err(SaveRequestError::DocumentBusy);
        }
        let bytes = match self.session.try_lock() {
            Ok(session) => session.bytes(),
            Err(TryLockError::WouldBlock) => return Err(SaveRequestError::DocumentBusy),
            Err(TryLockError::Poisoned(_)) => {
                return Err(SaveRequestError::DocumentLockPoisoned);
            }
        };
        submit_save(queue, &mut self.save_state, self.path.as_ref(), bytes)
    }
    pub fn apply_save_result(&mut self, result: SaveResult) {
        apply_save_result(&mut self.save_state, result);
    }
    pub fn apply_task_commit(&mut self, ack: omw_local::CommitAck) {
        self.revision = self.revision.wrapping_add(1);
        let dirty = self
            .session
            .lock()
            .map(|s| {
                let bytes = s.bytes();
                bytes.len() as u64 != ack.new_version.len
                    || *blake3::hash(&bytes).as_bytes() != ack.new_version.blake3
            })
            .unwrap_or(true);
        self.save_state
            .accept_task_commit(ack.new_version, self.revision, dirty);
    }
    pub fn mark_external_changed(&mut self) {
        self.save_state.apply_external_change();
    }
    /// Apply a verified disk event. A content change replaces the edit
    /// session, which also drops its undo and redo history.
    pub fn apply_external_event(
        &mut self,
        event: ExternalEvent,
    ) -> Result<ExternalEventOutcome, String> {
        if let ExternalEvent::Snapshot(snapshot) = &event {
            if self
                .save_state
                .baseline()
                .is_some_and(|baseline| baseline.same_content(&snapshot.version))
            {
                return Ok(reduce_external_event(&mut self.save_state, event));
            }
            let parsed = parse(&snapshot.bytes).map_err(|error| error.to_string())?;
            let outcome = reduce_external_event(&mut self.save_state, event);
            if matches!(outcome, ExternalEventOutcome::Reloaded(_)) {
                let mut session = self
                    .session
                    .lock()
                    .map_err(|_| "文档状态锁已失效".to_string())?;
                *session = EditSession::from_document(&parsed);
                self.revision = self.revision.wrapping_add(1);
            }
            Ok(outcome)
        } else {
            Ok(reduce_external_event(&mut self.save_state, event))
        }
    }
}
/// Independent transient state for each open presentation of the same document.
pub struct EditorViewState {
    pub scroll_y: f32,
    pub caret: VisualPos,
    pub marked_range: Option<(usize, usize)>,
    pub focused: bool,
}
impl Default for EditorViewState {
    fn default() -> Self {
        Self {
            scroll_y: 0.0,
            caret: VisualPos(0),
            marked_range: None,
            focused: false,
        }
    }
}

struct ParseJob {
    document: u64,
    generation: u64,
    bytes: Vec<u8>,
}
pub struct ParseResult {
    pub document: u64,
    pub generation: u64,
    pub parsed: Result<DocumentSession, String>,
}
/// Fixed-size parser pool. Consumers must still compare result generation/revision before applying it.
pub struct ParsePool {
    workers: Vec<Sender<ParseJob>>,
    next_worker: usize,
    results: Receiver<ParseResult>,
    generations: HashMap<u64, u64>,
}
impl ParsePool {
    pub fn new() -> Self {
        let (result_tx, result_rx) = mpsc::channel();
        let mut workers = Vec::with_capacity(2);
        for _ in 0..2 {
            let (job_tx, job_rx) = mpsc::channel::<ParseJob>();
            let tx = result_tx.clone();
            let _ = thread::Builder::new()
                .name("omw-markdown-parse".into())
                .spawn(move || worker_loop(job_rx, tx));
            workers.push(job_tx);
        }
        Self {
            workers,
            next_worker: 0,
            results: result_rx,
            generations: HashMap::new(),
        }
    }
    pub fn request(&mut self, document: u64, bytes: Vec<u8>) -> Result<u64, &'static str> {
        let generation = self
            .generations
            .get(&document)
            .copied()
            .unwrap_or(0)
            .wrapping_add(1);
        self.generations.insert(document, generation);
        let worker = self.next_worker % self.workers.len();
        self.next_worker = self.next_worker.wrapping_add(1);
        self.workers[worker]
            .send(ParseJob {
                document,
                generation,
                bytes,
            })
            .map_err(|_| "parse workers have stopped")?;
        Ok(generation)
    }
    pub fn take_latest(&self) -> Vec<ParseResult> {
        self.results
            .try_iter()
            .filter(|result| self.generations.get(&result.document) == Some(&result.generation))
            .collect()
    }
}
fn worker_loop(jobs: Receiver<ParseJob>, results: Sender<ParseResult>) {
    while let Ok(job) = jobs.recv() {
        let parsed = parse(&job.bytes).map_err(|error: ParseError| error.to_string());
        if results
            .send(ParseResult {
                document: job.document,
                generation: job.generation,
                parsed,
            })
            .is_err()
        {
            return;
        }
    }
}
impl Default for ParsePool {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
struct EditorBlock {
    visible: crate::editor::VisibleBlock,
    visual_range: Option<VisualRange>,
    node: NodeId,
}

struct EditorSnapshot {
    source: ropey::Rope,
    text: gpui_kit::SharedString,
    blocks: Rc<Vec<EditorBlock>>,
    offsets: Vec<(usize, usize)>,
    utf16_len: usize,
}
impl EditorSnapshot {
    fn new(session: &EditSession) -> Self {
        let projection = crate::editor::VisibleProjection::from_session(session);
        let mut utf16_len = 0;
        let mut offsets = Vec::new();
        for (byte, grapheme) in projection.text.grapheme_indices(true) {
            offsets.push((byte, utf16_len));
            utf16_len += grapheme.encode_utf16().count();
        }
        offsets.push((projection.text.len(), utf16_len));
        let blocks = build_editor_blocks(session, &projection, &offsets);
        Self {
            source: session.text().clone(),
            text: projection.text.into(),
            blocks: Rc::new(blocks),
            offsets,
            utf16_len,
        }
    }
}

/// Focusable GPUI editor surface. It binds platform text input to source-safe domain edits;
/// visual selection is local to this view while the document session can be shared.
pub struct EditorSurface {
    session: Arc<Mutex<EditSession>>,
    document: Option<Entity<EditorDocument>>,
    _document_subscription: Option<Subscription>,
    focus: FocusHandle,
    selection: VisualSelection,
    marked: Option<(VisualRange, String)>,
    input_error: Option<String>,
    hit_test: Arc<Mutex<Option<HitTestLayout>>>,
    list_state: ListState,
    save_queue: Option<WriteQueue>,
    save_results: Vec<Receiver<SaveResult>>,
    snapshot: RefCell<Option<Rc<EditorSnapshot>>>,
    rendered_source: Option<ropey::Rope>,
}
impl EditorSurface {
    pub fn new(session: Arc<Mutex<EditSession>>, cx: &mut Context<Self>) -> Self {
        Self {
            session,
            document: None,
            _document_subscription: None,
            focus: cx.focus_handle(),
            selection: VisualSelection::collapsed(VisualPos(0)),
            marked: None,
            input_error: None,
            hit_test: Arc::new(Mutex::new(None)),
            list_state: ListState::new(0, ListAlignment::Top, px(240.)),
            save_queue: None,
            save_results: Vec::new(),
            snapshot: RefCell::new(None),
            rendered_source: None,
        }
    }
    pub fn from_document(document: Entity<EditorDocument>, cx: &mut Context<Self>) -> Self {
        let session = document.read(cx).session();
        let subscription = cx.observe(&document, |_, _, cx| cx.notify());
        let mut surface = Self::new(session, cx);
        surface.document = Some(document);
        surface._document_subscription = Some(subscription);
        surface
    }
    pub fn with_write_queue(mut self, queue: WriteQueue) -> Self {
        self.save_queue = Some(queue);
        self
    }

    pub fn request_save(&mut self, cx: &mut Context<Self>) {
        let Some(queue) = self.save_queue.clone() else {
            self.input_error = Some("save queue is not configured".to_string());
            return;
        };
        let Some(document) = self.document.clone() else {
            self.input_error = Some("editor is not attached to a document".to_string());
            return;
        };
        match document.update(cx, |document, _| document.request_save(&queue)) {
            Ok(receiver) => {
                let task = cx.background_spawn(async move { receiver.recv() });
                cx.spawn(async move |this, cx| {
                    let result = task.await;
                    let _ = this.update(cx, |this, cx| {
                        match result {
                            Ok(result) => {
                                if let Some(document) = &this.document {
                                    document.update(cx, |document, cx| {
                                        document.apply_save_result(result);
                                        cx.notify();
                                    });
                                }
                            }
                            Err(error) => this.input_error = Some(error.to_string()),
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
            Err(error) => self.input_error = Some(error.to_string()),
        }
        cx.notify();
    }

    fn poll_save_results(&mut self, cx: &mut Context<Self>) {
        let Some(document) = self.document.clone() else {
            return;
        };
        let mut pending = Vec::new();
        for receiver in self.save_results.drain(..) {
            match receiver.try_recv() {
                Ok(result) => {
                    document.update(cx, |document, _| document.apply_save_result(result));
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => pending.push(receiver),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.input_error = Some("save worker stopped".to_string());
                }
            }
        }
        self.save_results = pending;
    }

    fn notify_document(&mut self, cx: &mut Context<Self>) {
        if let Some(document) = &self.document {
            document.update(cx, |document, cx| {
                document.mark_changed();
                cx.notify();
            });
        }
    }
    pub fn input_error(&self) -> Option<&str> {
        self.input_error.as_deref()
    }
    fn snapshot(&self) -> Option<Rc<EditorSnapshot>> {
        let session = self.session.lock().ok()?;
        if let Some(cached) = self.snapshot.borrow().as_ref()
            && cached.source.is_instance(session.text())
        {
            return Some(cached.clone());
        }
        let cached = Rc::new(EditorSnapshot::new(&session));
        self.snapshot.replace(Some(cached.clone()));
        Some(cached)
    }
    fn projection(&self) -> gpui_kit::SharedString {
        let base = self
            .snapshot()
            .map(|snapshot| snapshot.text.clone())
            .unwrap_or_default();
        let Some((range, marked)) = &self.marked else {
            return base;
        };
        let mut text = base.to_string();
        let start = projection_byte_offset(&text, range.start.to_usize());
        let end = projection_byte_offset(&text, range.end.to_usize());
        text.replace_range(start..end, marked);
        text.into()
    }
    fn commit_visible(&mut self, visible: VisualRange, text: &str, cx: &mut Context<Self>) {
        let result = self
            .session
            .lock()
            .map_err(|_| "document lock poisoned".to_string())
            .and_then(|mut session| {
                crate::editor::commit_text(&mut session, visible, text)
                    .map_err(|error| error.to_string())
            });
        match result {
            Ok(()) => {
                let end = visible.start + text.graphemes(true).count() as u32;
                self.selection = VisualSelection::collapsed(end);
                self.input_error = None;
                self.notify_document(cx);
            }
            Err(error) => self.input_error = Some(error),
        }
        self.marked = None;
        cx.notify();
    }
    fn vertical_target(&self, caret: usize, down: bool) -> Option<u32> {
        let cache = self.hit_test.lock().ok()?;
        let layout = cache.as_ref()?;
        let current_line = layout
            .lines
            .iter()
            .rev()
            .find(|line| line.start_grapheme <= caret)?;
        let local = caret
            .saturating_sub(current_line.start_grapheme)
            .min(current_line.boundaries.len().saturating_sub(1));
        let current = current_line.boundaries.get(local)?.1;
        let target_x = current_line.origin_x + current.x.as_f32();
        let target_y = current_line.start_y
            + current.y.as_f32()
            + if down {
                layout.line_height.as_f32()
            } else {
                -layout.line_height.as_f32()
            };
        layout
            .lines
            .iter()
            .flat_map(|line| {
                line.boundaries
                    .iter()
                    .enumerate()
                    .map(move |(index, (_, position))| {
                        (
                            line.start_grapheme + index,
                            line.origin_x + position.x.as_f32(),
                            line.start_y + position.y.as_f32(),
                        )
                    })
            })
            .min_by(|(_, ax, ay), (_, bx, by)| {
                let da = (ax - target_x).powi(2) + (ay - target_y).powi(2);
                let db = (bx - target_x).powi(2) + (by - target_y).powi(2);
                da.total_cmp(&db)
            })
            .map(|(index, _, _)| index as u32)
    }
    fn visible_range_for_utf16(&self, range: Range<usize>) -> Option<VisualRange> {
        let projection = self.projection();
        crate::editor::utf16_range_to_visual(
            &projection,
            crate::editor::Utf16Range {
                start: range.start,
                end: range.end,
            },
        )
        .ok()
    }
    pub(crate) fn handle_toolbar_command(
        &mut self,
        command: EditorToolbarCommand,
        cx: &mut Context<Self>,
    ) {
        if self
            .document
            .as_ref()
            .is_some_and(|d| d.read(cx).task_pending())
        {
            return;
        }
        if matches!(command, EditorToolbarCommand::Save) {
            self.request_save(cx);
            return;
        }
        let result = match command {
            EditorToolbarCommand::Save => Ok(()),
            EditorToolbarCommand::Undo => self
                .session
                .lock()
                .map_err(|_| "document lock poisoned".to_string())
                .and_then(|mut session| {
                    session
                        .undo()
                        .map(|_| ())
                        .map_err(|error| error.to_string())
                }),
            EditorToolbarCommand::Redo => self
                .session
                .lock()
                .map_err(|_| "document lock poisoned".to_string())
                .and_then(|mut session| {
                    session
                        .redo()
                        .map(|_| ())
                        .map_err(|error| error.to_string())
                }),
            EditorToolbarCommand::Bold
            | EditorToolbarCommand::Italic
            | EditorToolbarCommand::Strikethrough => {
                let mark = match command {
                    EditorToolbarCommand::Bold => MarkKind::Bold,
                    EditorToolbarCommand::Italic => MarkKind::Italic,
                    EditorToolbarCommand::Strikethrough => MarkKind::Strikethrough,
                    _ => unreachable!(),
                };
                self.session
                    .lock()
                    .map_err(|_| "document lock poisoned".to_string())
                    .and_then(|mut session| {
                        let projection = crate::editor::VisibleProjection::from_session(&session);
                        let range = projection
                            .source_range(self.selection.range())
                            .map_err(|error| error.to_string())?;
                        session
                            .toggle_mark(mark, range)
                            .map(|_| ())
                            .map_err(|error| error.to_string())
                    })
            }
            EditorToolbarCommand::PasteMarkdown => {
                let Some(item) = cx.read_from_clipboard() else {
                    return;
                };
                let Some(text) = item.text() else {
                    return;
                };
                self.session
                    .lock()
                    .map_err(|_| "document lock poisoned".to_string())
                    .and_then(|mut session| {
                        let projection = crate::editor::VisibleProjection::from_session(&session);
                        let range = projection
                            .source_range(self.selection.range())
                            .map_err(|error| error.to_string())?;
                        session
                            .paste_markdown(range, &text)
                            .map(|_| ())
                            .map_err(|error| error.to_string())
                    })
            }
        };
        match result {
            Ok(()) => {
                self.input_error = None;
                self.notify_document(cx);
            }
            Err(error) => self.input_error = Some(error),
        }
        cx.notify();
    }

    fn handle_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        if self
            .document
            .as_ref()
            .is_some_and(|d| d.read(cx).task_pending())
        {
            return;
        }
        if self.marked.is_some() {
            return;
        }
        let key = event.keystroke.key.as_str();
        let modifiers = &event.keystroke.modifiers;
        if modifiers.control && key.eq_ignore_ascii_case("s") {
            self.request_save(cx);
            return;
        }
        if modifiers.control && key.eq_ignore_ascii_case("a") {
            self.selection = VisualSelection::new(
                VisualPos(0),
                VisualPos(self.projection().graphemes(true).count() as u32),
            );
            cx.notify();
            return;
        }
        if modifiers.control && (key.eq_ignore_ascii_case("c") || key.eq_ignore_ascii_case("x")) {
            let text = self.projection();
            let range = self.selection.range();
            let selected = text
                .grapheme_indices(true)
                .enumerate()
                .filter(|(index, _)| {
                    *index >= range.start.to_usize() && *index < range.end.to_usize()
                })
                .map(|(_, (_, grapheme))| grapheme)
                .collect::<String>();
            cx.write_to_clipboard(ClipboardItem::new_string(selected));
            if key.eq_ignore_ascii_case("x") && !self.selection.is_collapsed() {
                let mut changed = false;
                if let Ok(mut session) = self.session.lock() {
                    let projection = crate::editor::VisibleProjection::from_session(&session);
                    match crate::editor::delete_visible_range(&mut session, &projection, range) {
                        Ok(()) => {
                            self.selection = VisualSelection::collapsed(range.start);
                            self.input_error = None;
                            changed = true;
                        }
                        Err(error) => self.input_error = Some(error.to_string()),
                    }
                }
                if changed {
                    self.notify_document(cx);
                }
                cx.notify();
            }
            return;
        }
        if modifiers.control && key.eq_ignore_ascii_case("z") {
            let changed = if let Ok(mut session) = self.session.lock() {
                let result = if modifiers.shift {
                    session.redo()
                } else {
                    session.undo()
                };
                if let Err(error) = result {
                    self.input_error = Some(error.to_string());
                    false
                } else {
                    true
                }
            } else {
                false
            };
            if changed {
                self.notify_document(cx);
            }
            cx.notify();
            return;
        }
        let len = self.projection().graphemes(true).count() as u32;
        let head = self.selection.head.0;
        match key {
            "left" | "right" | "up" | "down" | "home" | "end" => {
                let selected = self.selection.range();
                let next = if !modifiers.shift
                    && selected.start != selected.end
                    && (key == "left" || key == "right")
                {
                    if key == "left" {
                        selected.start.0
                    } else {
                        selected.end.0
                    }
                } else if key == "home" || key == "end" {
                    visual_line_edge(&self.projection(), head as usize, key == "end") as u32
                } else if key == "up" || key == "down" {
                    self.vertical_target(head as usize, key == "down")
                        .unwrap_or(head)
                } else if modifiers.control && (key == "left" || key == "right") {
                    word_boundary(&self.projection(), head as usize, key == "left") as u32
                } else if key == "left" {
                    head.saturating_sub(1)
                } else if key == "right" {
                    head.saturating_add(1).min(len)
                } else {
                    head
                };
                self.selection = if modifiers.shift {
                    VisualSelection::new(self.selection.anchor, VisualPos(next))
                } else {
                    VisualSelection::collapsed(VisualPos(next))
                };
            }
            "backspace" | "delete" => {
                let range = if !self.selection.is_collapsed() {
                    self.selection.range()
                } else if key == "backspace" && head > 0 {
                    VisualRange::new(VisualPos(head - 1), VisualPos(head))
                } else if key == "delete" && head < len {
                    VisualRange::new(VisualPos(head), VisualPos(head + 1))
                } else {
                    return;
                };
                let mut changed = false;
                if let Ok(mut session) = self.session.lock() {
                    let projection = crate::editor::VisibleProjection::from_session(&session);
                    let result =
                        crate::editor::delete_visible_range(&mut session, &projection, range)
                            .map_err(|error| error.to_string());
                    if let Err(error) = result {
                        self.input_error = Some(error);
                    } else {
                        self.selection = VisualSelection::collapsed(range.start);
                        self.input_error = None;
                        changed = true;
                    }
                }
                if changed {
                    self.notify_document(cx);
                }
            }
            "enter" => {
                let mut changed = false;
                if let Ok(mut session) = self.session.lock() {
                    let projection = crate::editor::VisibleProjection::from_session(&session);
                    let source = projection.source_boundaries.get(head as usize).copied();
                    if let Some(source) = source {
                        if let Err(error) = session.split_block(source) {
                            self.input_error = Some(error.to_string());
                        } else {
                            self.selection =
                                VisualSelection::collapsed(VisualPos(head.saturating_add(1)));
                            self.input_error = None;
                            changed = true;
                        }
                    }
                }
                if changed {
                    self.notify_document(cx);
                }
            }
            _ => return,
        }
        cx.notify();
    }
}
fn contains_raw_descendant(session: &EditSession, node: NodeId) -> bool {
    let Some(kind) = session.node_kind(node) else {
        return true;
    };
    kind.is_raw()
        || matches!(kind, NodeKind::CodeBlock { .. } | NodeKind::Code { .. })
        || session
            .children(node)
            .unwrap_or_default()
            .into_iter()
            .any(|child| contains_raw_descendant(session, child))
}
fn contains_html_raw(session: &EditSession, node: NodeId) -> bool {
    session.node_kind(node).is_some_and(|kind| {
        kind.is_raw()
            || session
                .children(node)
                .unwrap_or_default()
                .into_iter()
                .any(|child| contains_html_raw(session, child))
    })
}

fn build_editor_blocks(
    session: &EditSession,
    projection: &crate::editor::VisibleProjection,
    offsets: &[(usize, usize)],
) -> Vec<EditorBlock> {
    let root_children = session.children(session.root_id()).unwrap_or_default();
    let mut blocks = Vec::with_capacity(root_children.len().max(1));
    for node in root_children {
        let Some(kind) = session.node_kind(node) else {
            continue;
        };
        let Some(source) = session.source_range(node) else {
            continue;
        };
        if !contains_raw_descendant(session, node) {
            let visual_range = projection_range_for_source(projection, source);
            let text = visual_range
                .and_then(|range| {
                    let start = offsets.get(range.start.to_usize())?.0;
                    let end = offsets.get(range.end.to_usize())?.0;
                    projection.text.get(start..end)
                })
                .unwrap_or_default()
                .to_owned();
            blocks.push(EditorBlock {
                visible: crate::editor::VisibleBlock::new(format!("node-{}", node.0), text),
                visual_range,
                node,
            });
        } else {
            let text = if kind.is_raw() || contains_html_raw(session, node) {
                format!("[Raw content preserved: {} bytes]", source.len())
            } else {
                session
                    .byte_slice(source)
                    .map(|slice| slice.into_owned())
                    .unwrap_or_default()
            };
            blocks.push(EditorBlock {
                visible: crate::editor::VisibleBlock::raw(format!("node-{}", node.0), text),
                visual_range: None,
                node,
            });
        }
    }
    if blocks.is_empty() {
        let length = projection.text.graphemes(true).count() as u32;
        blocks.push(EditorBlock {
            visible: crate::editor::VisibleBlock::new("document", projection.text.clone()),
            visual_range: Some(VisualRange::new(VisualPos(0), VisualPos(length))),
            node: session.root_id(),
        });
    }
    blocks
}

fn projection_range_for_source(
    projection: &crate::editor::VisibleProjection,
    source: ByteRange,
) -> Option<VisualRange> {
    let start = projection.source_position(source.start)?;
    let end = projection.source_position(source.end)?;
    let boundaries = &projection.source_boundaries;
    let first = boundaries.partition_point(|at| *at < start);
    let last = boundaries
        .partition_point(|at| *at <= end)
        .saturating_sub(1);
    if first >= last {
        return Some(VisualRange::new(VisualPos(0), VisualPos(0)));
    }
    Some(VisualRange::new(
        VisualPos(first as u32),
        VisualPos(last as u32),
    ))
}

impl Render for EditorSurface {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.poll_save_results(cx);
        let view = cx.entity();
        let toolbar_view = view.clone();
        let focus = self.focus.clone();
        let caret = self.selection.head.to_usize();
        let Some(snapshot) = self.snapshot() else {
            return div()
                .child("文档状态锁已失效，无法显示编辑器")
                .into_any_element();
        };
        let text = self.projection();
        let total_utf16 = snapshot.utf16_len;
        let blocks = snapshot.blocks.clone();
        if let Ok(mut cached) = self.hit_test.lock() {
            *cached = None;
        }
        if self.list_state.item_count() != blocks.len()
            || self
                .rendered_source
                .as_ref()
                .is_none_or(|source| !source.is_instance(&snapshot.source))
        {
            self.list_state.reset(blocks.len());
            self.rendered_source = Some(snapshot.source.clone());
        }
        let list_state = self.list_state.clone();
        let hit_test = self.hit_test.clone();
        let selection = self.selection;
        let input_error = self.input_error.clone().unwrap_or_default();
        let a11y_blocks = blocks.clone();
        let a11y_selection = selection;
        let list_element = list(list_state, move |index, window, _cx| {
            let Some(block) = blocks.get(index).cloned() else {
                return div().into_any_element();
            };
            let block_range = block.visual_range;
            let local_selection = block_range.and_then(|range| {
                let selected = selection.range();
                let start = selected.start.max(range.start);
                let end = selected.end.min(range.end);
                (start < end).then(|| {
                    VisualSelection::new(
                        VisualPos(start.0.saturating_sub(range.start.0)),
                        VisualPos(end.0.saturating_sub(range.start.0)),
                    )
                })
            });
            let block_text = block.visible.text.clone();
            let block_id = block.visible.id.clone();
            let block_start = block_range.map_or(0, |range| range.start.to_usize());
            let (block_byte_start, block_utf16_start) = snapshot
                .offsets
                .get(block_start)
                .copied()
                .unwrap_or_default();
            let block_node = block.node;
            let block_text_len_utf16 = total_utf16;
            let hit_test = hit_test.clone();
            let focus = focus.clone();
            let view = view.clone();
            div()
                .id(block_id.clone())
                .relative()
                .w_full()
                .min_h(px(24.))
                .child(
                    div()
                        .id(format!("{block_id}-text"))
                        .role(if block.visible.editable {
                            Role::Paragraph
                        } else {
                            Role::Label
                        })
                        .child(
                            gpui_kit::StyledText::new(block_text.clone()).with_runs(
                                local_selection
                                    .map(|selection| {
                                        selection_text_runs(&block_text, selection, window)
                                    })
                                    .unwrap_or_else(|| {
                                        vec![window.text_style().to_run(block_text.len())]
                                    }),
                            ),
                        ),
                )
                .child(
                    canvas(
                        move |bounds, window, _| {
                            shape_hit_test_block(
                                block_text,
                                bounds,
                                window,
                                block_start,
                                block_byte_start,
                                block_utf16_start,
                                block_node,
                                block_text_len_utf16,
                            )
                        },
                        move |bounds, layout, window, cx| {
                            if let Some(layout) = layout.clone() {
                                if let Ok(mut cached) = hit_test.lock() {
                                    merge_hit_test_layout(&mut cached, layout.clone());
                                }
                                window.handle_input(
                                    &focus,
                                    ElementInputHandler::new(bounds, view.clone()),
                                    cx,
                                );
                                if focus.is_focused(window)
                                    && let Some(origin) = caret_origin(&layout, caret)
                                {
                                    window.paint_quad(fill(
                                        Bounds {
                                            origin,
                                            size: gpui_kit::size(px(1.), px(18.)),
                                        },
                                        hsla(0.58, 0.85, 0.55, 1.0),
                                    ));
                                }
                            }
                        },
                    )
                    .absolute()
                    .inset_0(),
                )
                .into_any_element()
        })
        .size_full();
        let toolbar = super::toolbar::editor_toolbar(toolbar_view);
        let editor_surface = div()
            .id("paragraph-editor")
            .role(Role::TextInput)
            .aria_label("Markdown editor")
            .aria_value(text)
            .aria_description(input_error)
            .on_key_down(cx.listener(|this, event, _window, cx| this.handle_key(event, cx)))
            .track_focus(&self.focus)
            .test_support()
            .w_full()
            .h_full()
            .a11y_synthetic_children(move |builder| {
                let mut ids = Vec::with_capacity(a11y_blocks.len());
                for block in a11y_blocks.iter() {
                    let id = builder.synthetic_node_id(&block.visible.id);
                    let mut node = gpui_kit::accesskit::Node::new(Role::TextRun);
                    node.set_value(block.visible.text.clone());
                    node.set_character_lengths(
                        block
                            .visible
                            .text
                            .graphemes(true)
                            .map(|grapheme| grapheme.len() as u8)
                            .collect::<Vec<_>>(),
                    );
                    builder.push_child(id, node);
                    ids.push((id, block));
                }
                if let Some((anchor, focus)) = a11y_selection_positions(&ids, a11y_selection) {
                    builder
                        .parent_node()
                        .set_text_selection(gpui_kit::accesskit::TextSelection { anchor, focus });
                }
            })
            .child(list_element);
        div()
            .id("markdown-editor-shell")
            .w_full()
            .h_full()
            .flex()
            .flex_col()
            .child(toolbar)
            .child(div().flex_1().min_h_0().child(editor_surface))
            .into_any_element()
    }
}

fn selection_text_runs(
    text: &str,
    selection: VisualSelection,
    window: &Window,
) -> Vec<gpui_kit::TextRun> {
    let mut base = window.text_style().to_run(text.len());
    let range = selection.range();
    if range.start == range.end || text.is_empty() {
        return vec![base];
    }
    let offsets: Vec<usize> = text
        .grapheme_indices(true)
        .map(|(byte, _)| byte)
        .chain([text.len()])
        .collect();
    let start = offsets
        .get(range.start.to_usize())
        .copied()
        .unwrap_or(text.len());
    let end = offsets
        .get(range.end.to_usize())
        .copied()
        .unwrap_or(text.len());
    if start >= end {
        return vec![base];
    }
    let mut runs = Vec::with_capacity(3);
    if start > 0 {
        let mut run = base.clone();
        run.len = start;
        runs.push(run);
    }
    base.len = end - start;
    base.background_color = Some(hsla(0.61, 0.72, 0.48, 0.48));
    runs.push(base);
    if end < text.len() {
        let mut run = window.text_style().to_run(text.len() - end);
        run.len = text.len() - end;
        runs.push(run);
    }
    runs
}
#[derive(Clone)]
struct HitTestLine {
    node: NodeId,
    start_byte: usize,
    start_utf16: usize,
    start_grapheme: usize,
    start_y: f32,
    origin_x: f32,
    text: String,
    boundaries: Vec<(usize, gpui_kit::Point<gpui_kit::Pixels>)>,
}
#[derive(Clone)]
struct HitTestLayout {
    bounds: Bounds<gpui_kit::Pixels>,
    lines: Vec<HitTestLine>,
    line_height: gpui_kit::Pixels,
    text_len_utf16: usize,
}
fn shape_hit_test_layout(
    text: String,
    bounds: Bounds<gpui_kit::Pixels>,
    window: &mut Window,
) -> Option<HitTestLayout> {
    let style = window.text_style();
    let font_size = style.font_size.to_pixels(window.rem_size());
    let line_height = style
        .line_height
        .to_pixels(font_size.into(), window.rem_size());
    let run = style.to_run(text.len());
    let wrapped = window
        .text_system()
        .shape_text(
            text.clone().into(),
            font_size,
            &[run],
            Some(bounds.size.width),
            None,
        )
        .ok()?;
    let mut byte_start = 0usize;
    let mut utf16_start = 0usize;
    let mut top = 0.0f32;
    let mut grapheme_start = 0usize;
    let mut lines = Vec::with_capacity(wrapped.len());
    for line in wrapped {
        let line_text = line.text.to_string();
        let line_utf16 = line_text.encode_utf16().count();
        let line_height_count = line.wrap_boundaries.len() + 1;
        let mut boundaries = vec![(0, line.position_for_index(0, line_height)?)];
        for (byte, grapheme) in line_text.grapheme_indices(true) {
            let end = byte + grapheme.len();
            boundaries.push((end, line.position_for_index(end, line_height)?));
        }
        lines.push(HitTestLine {
            node: NodeId(0),
            start_byte: byte_start,
            start_utf16: utf16_start,
            start_grapheme: grapheme_start,
            start_y: bounds.origin.y.as_f32() + top,
            origin_x: bounds.origin.x.as_f32(),
            text: line_text.clone(),
            boundaries,
        });
        byte_start += line_text.len();
        utf16_start += line_utf16;
        grapheme_start += line_text.graphemes(true).count();
        if byte_start < text.len() && text.as_bytes().get(byte_start) == Some(&10) {
            byte_start += 1;
            utf16_start += 1;
            grapheme_start += 1;
        }
        top += line_height.as_f32() * line_height_count as f32;
    }
    Some(HitTestLayout {
        bounds,
        lines,

        line_height,
        text_len_utf16: text.encode_utf16().count(),
    })
}
fn projection_byte_offset(text: &str, grapheme: usize) -> usize {
    text.grapheme_indices(true)
        .nth(grapheme)
        .map(|(byte, _)| byte)
        .unwrap_or(text.len())
}

#[allow(clippy::too_many_arguments)]
fn shape_hit_test_block(
    text: String,
    bounds: Bounds<gpui_kit::Pixels>,
    window: &mut Window,
    start_grapheme: usize,
    start_byte: usize,
    start_utf16: usize,
    node: NodeId,
    text_len_utf16: usize,
) -> Option<HitTestLayout> {
    let mut layout = shape_hit_test_layout(text, bounds, window)?;
    for line in &mut layout.lines {
        line.node = node;
        line.start_grapheme += start_grapheme;
        line.start_byte += start_byte;
        line.start_utf16 += start_utf16;
    }
    layout.text_len_utf16 = text_len_utf16;
    Some(layout)
}

fn merge_hit_test_layout(cached: &mut Option<HitTestLayout>, incoming: HitTestLayout) {
    let Some(existing) = cached.as_mut() else {
        *cached = Some(incoming);
        return;
    };
    existing.lines.retain(|line| {
        !incoming
            .lines
            .iter()
            .any(|new_line| new_line.node == line.node)
    });
    existing.lines.extend(incoming.lines);
    existing.lines.sort_by(|a, b| {
        a.start_grapheme
            .cmp(&b.start_grapheme)
            .then_with(|| a.start_y.total_cmp(&b.start_y))
    });
    existing.bounds = incoming.bounds;
    existing.line_height = incoming.line_height;
    existing.text_len_utf16 = existing.text_len_utf16.max(incoming.text_len_utf16);
}

fn caret_origin(layout: &HitTestLayout, caret: usize) -> Option<gpui_kit::Point<gpui_kit::Pixels>> {
    let line = layout
        .lines
        .iter()
        .rev()
        .find(|line| line.start_grapheme <= caret)?;
    let local = caret
        .saturating_sub(line.start_grapheme)
        .min(line.boundaries.len().saturating_sub(1));
    let position = line.boundaries.get(local)?.1;
    Some(gpui_kit::point(
        px(line.origin_x) + position.x,
        px(line.start_y) + position.y,
    ))
}

fn a11y_selection_positions(
    blocks: &[(gpui_kit::accesskit::NodeId, &EditorBlock)],
    selection: VisualSelection,
) -> Option<(
    gpui_kit::accesskit::TextPosition,
    gpui_kit::accesskit::TextPosition,
)> {
    fn position(
        blocks: &[(gpui_kit::accesskit::NodeId, &EditorBlock)],
        visual: VisualPos,
    ) -> Option<gpui_kit::accesskit::TextPosition> {
        let (_, block) = blocks.iter().find(|(_, block)| {
            block
                .visual_range
                .is_some_and(|range| range.start <= visual && visual <= range.end)
        })?;
        let range = block.visual_range?;
        let node = blocks
            .iter()
            .find(|(_, candidate)| candidate.node == block.node)?
            .0;
        Some(gpui_kit::accesskit::TextPosition {
            node,
            character_index: visual
                .0
                .saturating_sub(range.start.0)
                .min(block.visible.text.graphemes(true).count() as u32)
                as usize,
        })
    }
    Some((
        position(blocks, selection.anchor)?,
        position(blocks, selection.head)?,
    ))
}
impl EntityInputHandler for EditorSurface {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let text = self.projection();
        let (start, end) = utf16_to_byte_range(&text, range)?;
        *adjusted = None;
        text.get(start..end).map(str::to_owned)
    }
    fn selected_text_range(
        &mut self,
        _ignore_disabled: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let text = self.projection();
        let start = grapheme_to_utf16(&text, self.selection.anchor.to_usize())?;
        let end = grapheme_to_utf16(&text, self.selection.head.to_usize())?;
        Some(UTF16Selection {
            range: start.min(end)..start.max(end),
            reversed: start > end,
        })
    }
    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        let (range, text) = self.marked.as_ref()?;
        let projection = self.projection();
        let start = grapheme_to_utf16(&projection, range.start.to_usize())?;
        Some(start..start + text.encode_utf16().count())
    }
    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some((range, text)) = self.marked.clone() {
            self.commit_visible(range, &text, cx);
        }
    }
    fn paste(&mut self, item: ClipboardItem, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = item.text() {
            let range = self.selection.range();
            self.commit_visible(range, &text, cx);
        }
    }
    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .document
            .as_ref()
            .is_some_and(|d| d.read(cx).task_pending())
        {
            return;
        }
        let range = self
            .marked
            .as_ref()
            .map(|(r, _)| *r)
            .or_else(|| range.and_then(|r| self.visible_range_for_utf16(r)))
            .unwrap_or_else(|| self.selection.range());
        self.commit_visible(range, text, cx);
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .document
            .as_ref()
            .is_some_and(|d| d.read(cx).task_pending())
        {
            return;
        }
        let range = self
            .marked
            .as_ref()
            .map(|(r, _)| *r)
            .or_else(|| range.and_then(|r| self.visible_range_for_utf16(r)))
            .unwrap_or_else(|| self.selection.range());
        if text.is_empty() {
            self.marked = None;
            self.selection = VisualSelection::collapsed(range.start);
            cx.notify();
            return;
        }
        self.marked = Some((range, text.to_owned()));
        if let Some(selected) = selected
            && let Ok(relative) = crate::editor::utf16_range_to_visual(
                text,
                crate::editor::Utf16Range {
                    start: selected.start,
                    end: selected.end,
                },
            )
        {
            self.selection =
                VisualSelection::new(range.start + relative.start.0, range.start + relative.end.0);
        }
        cx.notify();
    }
    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        _bounds: Bounds<gpui_kit::Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<gpui_kit::Pixels>> {
        let cache = self.hit_test.lock().ok()?;
        let layout = cache.as_ref()?;
        let (start, _) = utf16_to_byte_range(&self.projection(), range)?;
        let line = layout
            .lines
            .iter()
            .rev()
            .find(|line| line.start_byte <= start)?;
        let local = start.saturating_sub(line.start_byte).min(line.text.len());
        let position = line.boundaries.iter().find(|(byte, _)| *byte == local)?.1;
        Some(Bounds {
            origin: gpui_kit::point(
                px(line.origin_x) + position.x,
                px(line.start_y) + position.y,
            ),
            size: gpui_kit::size(px(1.), layout.line_height),
        })
    }
    fn character_index_for_point(
        &mut self,
        point: gpui_kit::Point<gpui_kit::Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let cache = self.hit_test.lock().ok()?;
        let layout = cache.as_ref()?;
        let line = layout.lines.iter().min_by(|a, b| {
            (a.start_y - point.y.as_f32())
                .abs()
                .total_cmp(&(b.start_y - point.y.as_f32()).abs())
        })?;
        let local_x = point.x.as_f32() - line.origin_x;
        let local_y = point.y.as_f32() - line.start_y;
        let (index, _) = line.boundaries.iter().min_by(|(_, a), (_, b)| {
            let distance = |position: &gpui_kit::Point<gpui_kit::Pixels>| {
                (position.x.as_f32() - local_x).powi(2) + (position.y.as_f32() - local_y).powi(2)
            };
            distance(a).total_cmp(&distance(b))
        })?;
        let prefix = line.text.get(..*index)?;
        Some(line.start_utf16 + prefix.encode_utf16().count())
            .filter(|&index| index <= layout.text_len_utf16)
    }
    fn set_selected_text_range(
        &mut self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(visual) = self.visible_range_for_utf16(range) {
            self.selection = VisualSelection::new(visual.start, visual.end);
            cx.notify();
        }
    }
    fn text_length_utf16(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.projection().encode_utf16().count())
    }
}
fn utf16_to_byte_range(text: &str, range: Range<usize>) -> Option<(usize, usize)> {
    if range.start > range.end {
        return None;
    }
    let mut offsets = vec![0usize];
    let mut count = 0;
    for (index, character) in text.char_indices() {
        if index != 0 {
            offsets.push(count);
        }
        count += character.len_utf16();
    }
    offsets.push(count);
    let byte = |target| {
        text.char_indices()
            .map(|(at, _)| at)
            .chain(std::iter::once(text.len()))
            .nth(offsets.iter().position(|&n| n == target)?)
    };
    Some((byte(range.start)?, byte(range.end)?))
}
fn visual_line_edge(text: &str, position: usize, end: bool) -> usize {
    let offsets: Vec<usize> = text
        .grapheme_indices(true)
        .map(|(byte, _)| byte)
        .chain([text.len()])
        .collect();
    let byte = offsets.get(position).copied().unwrap_or(text.len());
    if end {
        text[byte..]
            .find('\n')
            .map_or(text.graphemes(true).count(), |offset| {
                text[..byte + offset].graphemes(true).count()
            })
    } else {
        text[..byte]
            .rfind('\n')
            .map_or(0, |offset| text[..=offset].graphemes(true).count())
    }
}
fn word_boundary(text: &str, position: usize, backwards: bool) -> usize {
    let mut boundaries: Vec<usize> = text
        .split_word_bound_indices()
        .map(|(byte, _)| text[..byte].graphemes(true).count())
        .collect();
    boundaries.push(text.graphemes(true).count());
    if backwards {
        boundaries
            .into_iter()
            .filter(|&boundary| boundary < position)
            .max()
            .unwrap_or(0)
    } else {
        boundaries
            .into_iter()
            .find(|&boundary| boundary > position)
            .unwrap_or_else(|| text.graphemes(true).count())
    }
}
fn grapheme_to_utf16(text: &str, target: usize) -> Option<usize> {
    use unicode_segmentation::UnicodeSegmentation as _;
    let mut count = 0;
    for (index, grapheme) in text.graphemes(true).enumerate() {
        if index == target {
            return Some(count);
        }
        count += grapheme.encode_utf16().count();
    }
    (target == text.graphemes(true).count()).then_some(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    #[test]
    fn home_end_positions_use_grapheme_indices() {
        let text = "猫🧑‍💻\nsecond";
        assert_eq!(visual_line_edge(text, 2, false), 0);
        assert_eq!(visual_line_edge(text, 2, true), 2);
        assert_eq!(visual_line_edge(text, 3, false), 3);
        assert_eq!(visual_line_edge(text, 3, true), 9);
    }

    #[test]
    fn parser_returns_only_current_generation() {
        let mut pool = ParsePool::new();
        let stale = pool
            .request(7, b"old".to_vec())
            .expect("worker channel open");
        let latest = pool
            .request(7, b"new".to_vec())
            .expect("worker channel open");
        assert!(latest > stale);
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let results = pool.take_latest();
            if !results.is_empty() {
                assert_eq!(results.len(), 1);
                assert_eq!(results[0].generation, latest);
                assert_eq!(
                    results[0]
                        .parsed
                        .as_ref()
                        .map(DocumentSession::serialize)
                        .as_deref(),
                    Ok("new")
                );
                break;
            }
            assert!(Instant::now() < deadline, "parser result timed out");
            thread::sleep(Duration::from_millis(5));
        }
    }
    #[test]
    fn external_reload_discards_dirty_text_and_undo_history() {
        let path = std::env::temp_dir().join(format!(
            "ohmywork-external-reload-{}.md",
            std::process::id()
        ));
        std::fs::write(&path, b"baseline").expect("write baseline");
        let baseline = omw_local::read_snapshot(&path).expect("read baseline");
        let document = parse(&baseline.bytes).expect("parse baseline");
        let mut editor = EditorDocument::from_snapshot(&document, &path, &baseline);
        editor
            .session()
            .lock()
            .expect("session lock")
            .insert_text(VisualPos(0), "local")
            .expect("local edit");
        editor.mark_changed();
        assert!(editor.save_state().is_dirty());

        std::fs::write(&path, b"external").expect("external edit");
        let external = omw_local::read_snapshot(&path).expect("read external");
        let outcome = editor
            .apply_external_event(ExternalEvent::Snapshot(external))
            .expect("reload external");

        assert!(matches!(outcome, ExternalEventOutcome::Reloaded(_)));
        let session = editor.session();
        let session = session.lock().expect("session lock");
        assert_eq!(session.bytes(), b"external");
        assert_eq!(session.history().undo_depth(), 0);
        assert_eq!(session.history().redo_depth(), 0);
        drop(session);
        assert_eq!(
            editor.save_state().status(),
            &crate::document_session::SaveStatus::Clean
        );
        let _ = std::fs::remove_file(path);
    }
    #[test]
    fn raw_blocks_are_kept_as_non_editable_visual_blocks() {
        let document = parse(b"before\n\n<div>raw</div>\n\nafter").expect("fixture parses");
        let session = EditSession::from_document(&document);
        let blocks = EditorSnapshot::new(&session).blocks;
        assert!(blocks.iter().any(|block| !block.visible.editable));
        assert!(
            blocks
                .iter()
                .filter(|block| block.visible.editable)
                .any(|block| block.visible.text == "before")
        );
    }
    #[test]
    fn inline_raw_content_uses_safe_summary() {
        let document = parse(b"before <span>raw</span> after").expect("fixture parses");
        let session = EditSession::from_document(&document);
        let blocks = EditorSnapshot::new(&session).blocks;
        assert_eq!(blocks.len(), 1);
        assert!(!blocks[0].visible.editable);
    }
    #[test]
    fn fenced_raw_blocks_keep_their_source_fence() {
        let document = parse(b"```raw\n<not-editable>\n```\n").expect("fixture parses");
        let session = EditSession::from_document(&document);
        let blocks = EditorSnapshot::new(&session).blocks;
        assert_eq!(blocks.len(), 1);
        assert!(!blocks[0].visible.editable);
        assert_eq!(blocks[0].visible.text, "```raw\n<not-editable>\n```");
    }
}
