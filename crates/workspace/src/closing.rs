use std::{path::PathBuf, time::Duration};

use editor::documents::{DocumentEvent, DocumentStore};
use gpui_kit::{AppContext as _, Context, Entity, EventEmitter, Subscription};

use crate::{homepage::Homepage, persistence::UiStateWriter, pi::PiSession};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseDecision {
    Save,
    Discard,
    Cancel,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CloseEvent {
    Prompt { path: PathBuf },
    RemoveTab { path: PathBuf },
    CaptureLayout,
    ReadyToCloseWindow,
    Cancelled,
    Failed(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PageCloseProgress {
    Prompt,
    Wait,
    Release,
}

fn page_close_progress(dirty: bool, pending: bool) -> PageCloseProgress {
    match (dirty, pending) {
        (true, false) => PageCloseProgress::Prompt,
        (false, false) => PageCloseProgress::Release,
        (_, true) => PageCloseProgress::Wait,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum CloseTarget {
    Tab(PathBuf),
    Window,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ClosePhase {
    Idle,
    AwaitingDecision { path: PathBuf },
    WaitingForWrite { path: PathBuf },
    WaitingForTasks,
    StoppingPi,
    Flushing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FlushStage {
    AwaitingLayout,
    WaitingForWriter,
}

/// Owns the single in-flight tab or window close operation.
pub struct CloseCoordinator {
    documents: Entity<DocumentStore>,
    pi: Entity<PiSession>,
    homepage: Entity<Homepage>,
    writer: UiStateWriter,
    target: Option<CloseTarget>,
    phase: ClosePhase,
    operation_generation: u64,
    flush_stage: Option<FlushStage>,
    discard_pending: bool,
    _documents_subscription: Subscription,
    _pi_subscription: Subscription,
}

impl CloseCoordinator {
    pub fn new(
        documents: Entity<DocumentStore>,
        pi: Entity<PiSession>,
        homepage: Entity<Homepage>,
        writer: UiStateWriter,
        cx: &mut Context<Self>,
    ) -> Self {
        let documents_subscription = cx.subscribe(&documents, |this, _, event: &DocumentEvent, cx| {
            this.on_document_event(event, cx);
        });
        let pi_subscription = cx.observe(&pi, |this, _, cx| this.on_pi_changed(cx));
        Self {
            documents,
            pi,
            homepage,
            writer,
            target: None,
            phase: ClosePhase::Idle,
            operation_generation: 0,
            flush_stage: None,
            discard_pending: false,
            _documents_subscription: documents_subscription,
            _pi_subscription: pi_subscription,
        }
    }

    pub fn request_tab(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if !matches!(self.phase, ClosePhase::Idle) { return; }
        let composing = self.documents.read(cx).get(&path)
            .is_some_and(|page| page.read(cx).has_composition());
        if composing {
            cx.emit(CloseEvent::Failed("请先确认或取消输入法组合文本".into()));
            return;
        }
        self.operation_generation = self.operation_generation.wrapping_add(1).max(1);
        self.target = Some(CloseTarget::Tab(path.clone()));
        self.inspect_tab(path, cx);
    }

    pub fn request_window(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.phase, ClosePhase::Idle) { return; }
        if self.documents.read(cx).has_composition(cx) {
            cx.emit(CloseEvent::Failed("请先确认或取消输入法组合文本".into()));
            return;
        }
        self.operation_generation = self.operation_generation.wrapping_add(1).max(1);
        self.target = Some(CloseTarget::Window);
        self.advance_window(cx);
    }

    pub fn decide(&mut self, decision: CloseDecision, cx: &mut Context<Self>) {
        let ClosePhase::AwaitingDecision { path } = self.phase.clone() else { return; };
        match decision {
            CloseDecision::Cancel => {
                self.finish();
                cx.emit(CloseEvent::Cancelled);
            }
            CloseDecision::Save => {
                let result = self.documents.update(cx, |store, cx| store.save(path.clone(), cx));
                match result {
                    Ok(()) => {
                        self.phase = ClosePhase::WaitingForWrite { path: path.clone() };
                        let store = self.documents.read(cx);
                        let dirty = store.get(&path).is_some_and(|page| page.read(cx).is_dirty());
                        let pending = store.has_pending_work(cx);
                        if !dirty && !pending {
                            match self.release_path(&path, false, cx) {
                                Ok(()) => self.after_tab_removed(path, cx),
                                Err(error) => self.fail(error, cx),
                            }
                        }
                    }
                    Err(error) => self.fail(error.to_string(), cx),
                }
            }
            CloseDecision::Discard => {
                if self.documents.read(cx).has_pending_work(cx) {
                    self.discard_pending = true;
                    self.phase = ClosePhase::WaitingForWrite { path };
                } else {
                    match self.release_path(&path, true, cx) {
                        Ok(()) => self.after_tab_removed(path, cx),
                        Err(error) => self.fail(error, cx),
                    }
                }
            }
        }
    }

    /// Acknowledges that the UI has enqueued the captured Dock layout.
    pub fn layout_captured(&mut self, cx: &mut Context<Self>) {
        if self.phase != ClosePhase::Flushing || self.flush_stage != Some(FlushStage::AwaitingLayout) {
            return;
        }
        let receiver = self.writer.flush();
        match receiver {
            Ok(receiver) => {
                self.flush_stage = Some(FlushStage::WaitingForWriter);
                let generation = self.operation_generation;
                cx.spawn(async move |this, cx| {
                    let result = cx.background_spawn(async move { receiver.recv() }).await;
                    let _ = this.update(cx, |this, cx| {
                        if this.operation_generation != generation { return; }
                        match result {
                            Ok(Ok(())) => this.finish_window_checks(cx),
                            Ok(Err(error)) => this.fail(error, cx),
                            Err(error) => this.fail(error.to_string(), cx),
                        }
                    });
                }).detach();
            }
            Err(error) => self.fail(error, cx),
        }
    }

    pub fn layout_failed(&mut self, message: String, cx: &mut Context<Self>) {
        if self.phase == ClosePhase::Flushing && self.flush_stage == Some(FlushStage::AwaitingLayout) {
            self.fail(message, cx);
        }
    }

    fn inspect_tab(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let store = self.documents.read(cx);
        let dirty = store.get(&path).is_some_and(|page| page.read(cx).is_dirty());
        let pending = store.has_pending_work(cx);
        match page_close_progress(dirty, pending) {
            PageCloseProgress::Prompt => {
                self.phase = ClosePhase::AwaitingDecision { path: path.clone() };
                cx.emit(CloseEvent::Prompt { path });
            }
            PageCloseProgress::Wait => self.phase = ClosePhase::WaitingForWrite { path },
            PageCloseProgress::Release => match self.release_path(&path, false, cx) {
                Ok(()) => self.after_tab_removed(path, cx),
                Err(error) => self.fail(error, cx),
            },
        }
    }

    fn on_document_event(&mut self, event: &DocumentEvent, cx: &mut Context<Self>) {
        if let DocumentEvent::Failed { path, message } = event {
            let relevant = match &self.target {
                Some(CloseTarget::Tab(target)) => target == path,
                Some(CloseTarget::Window) => true,
                None => false,
            };
            if relevant {
                self.fail(message.clone(), cx);
                return;
            }
        }
        self.recheck(cx);
    }

    fn recheck(&mut self, cx: &mut Context<Self>) {
        match self.phase.clone() {
            ClosePhase::WaitingForWrite { path } => match self.target.clone() {
                Some(CloseTarget::Tab(_)) => {
                    let (dirty, pending) = {
                        let store = self.documents.read(cx);
                        (
                            store.get(&path).is_some_and(|page| page.read(cx).is_dirty()),
                            store.has_pending_work(cx),
                        )
                    };
                    if self.discard_pending && !pending {
                        match self.release_path(&path, true, cx) {
                            Ok(()) => self.after_tab_removed(path, cx),
                            Err(error) => self.fail(error, cx),
                        }
                    } else {
                        match page_close_progress(dirty, pending) {
                            PageCloseProgress::Prompt => {
                                self.phase = ClosePhase::AwaitingDecision { path: path.clone() };
                                cx.emit(CloseEvent::Prompt { path });
                            }
                            PageCloseProgress::Wait => {}
                            PageCloseProgress::Release => match self.release_path(&path, false, cx) {
                                Ok(()) => self.after_tab_removed(path, cx),
                                Err(error) => self.fail(error, cx),
                            },
                        }
                    }
                }
                Some(CloseTarget::Window) => self.advance_window(cx),
                None => {}
            },
            ClosePhase::WaitingForTasks => self.advance_window(cx),
            _ => {}
        }
    }

    fn after_tab_removed(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        cx.emit(CloseEvent::RemoveTab { path });
        if matches!(self.target, Some(CloseTarget::Window)) {
            self.phase = ClosePhase::Idle;
            self.advance_window(cx);
        } else {
            self.finish();
        }
    }

    fn advance_window(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.target, Some(CloseTarget::Window)) { return; }
        if self.documents.read(cx).has_pending_work(cx) {
            self.phase = ClosePhase::WaitingForTasks;
            return;
        }
        if let Some(path) = self.documents.read(cx).first_dirty(cx) {
            self.phase = ClosePhase::AwaitingDecision { path: path.clone() };
            cx.emit(CloseEvent::Prompt { path });
            return;
        }
        if self.pi.read(cx).is_connected() {
            self.phase = ClosePhase::StoppingPi;
            self.pi.update(cx, |pi, cx| pi.close(false, cx));
            if !self.pi.read(cx).is_connected() {
                self.begin_layout_flush(cx);
                return;
            }
            let generation = self.operation_generation;
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_secs(6)).await;
                let _ = this.update(cx, |this, cx| {
                    if this.operation_generation == generation
                        && this.phase == ClosePhase::StoppingPi
                        && this.pi.read(cx).is_connected()
                    {
                        this.fail("Pi 关闭超时；应用保持打开，Pi 会话已保留".into(), cx);
                    }
                });
            }).detach();
            return;
        }
        self.begin_layout_flush(cx);
    }

    fn on_pi_changed(&mut self, cx: &mut Context<Self>) {
        if self.phase != ClosePhase::StoppingPi { return; }
        let status = self.pi.read(cx).status().to_owned();
        if status.starts_with("关闭失败:") {
            self.fail(status, cx);
        } else if !self.pi.read(cx).is_connected() {
            self.begin_layout_flush(cx);
        }
    }

    fn begin_layout_flush(&mut self, cx: &mut Context<Self>) {
        if self.phase == ClosePhase::Flushing { return; }
        match self.homepage.update(cx, |home, cx| home.checkpoint(cx)) {
            Ok(()) => {}
            Err(error) => {
                self.fail(error, cx);
                return;
            }
        }
        self.phase = ClosePhase::Flushing;
        self.flush_stage = Some(FlushStage::AwaitingLayout);
        cx.emit(CloseEvent::CaptureLayout);
    }

    fn finish_window_checks(&mut self, cx: &mut Context<Self>) {
        self.flush_stage = None;
        if self.documents.read(cx).has_pending_work(cx) {
            self.phase = ClosePhase::WaitingForTasks;
            return;
        }
        if let Some(path) = self.documents.read(cx).first_dirty(cx) {
            self.phase = ClosePhase::AwaitingDecision { path: path.clone() };
            cx.emit(CloseEvent::Prompt { path });
            return;
        }
        if self.pi.read(cx).is_connected() {
            self.fail("Pi close timed out while reconnected; keeping the application open".into(), cx);
        } else {
            cx.emit(CloseEvent::ReadyToCloseWindow);
            self.finish();
        }
    }

    fn release_path(&mut self, path: &PathBuf, discard: bool, cx: &mut Context<Self>) -> Result<(), String> {
        self.documents.update(cx, |store, cx| store.release(path.clone(), discard, cx))
            .map_err(|error| error.to_string())
    }

    fn fail(&mut self, message: String, cx: &mut Context<Self>) {
        self.finish();
        cx.emit(CloseEvent::Failed(message));
    }

    fn finish(&mut self) {
        self.operation_generation = self.operation_generation.wrapping_add(1).max(1);
        self.target = None;
        self.phase = ClosePhase::Idle;
        self.flush_stage = None;
        self.discard_pending = false;
    }
}
impl EventEmitter<CloseEvent> for CloseCoordinator {}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirty_documents_are_prompted_only_after_prior_work_finishes() {
        assert_eq!(page_close_progress(true, true), PageCloseProgress::Wait);
        assert_eq!(page_close_progress(true, false), PageCloseProgress::Prompt);
    }

    #[test]
    fn clean_documents_wait_for_work_then_release() {
        assert_eq!(page_close_progress(false, true), PageCloseProgress::Wait);
        assert_eq!(page_close_progress(false, false), PageCloseProgress::Release);
    }
}
