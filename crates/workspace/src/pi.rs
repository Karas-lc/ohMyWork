use gpui_kit::{AppContext as _, Context};
use project::pi::{ContextItem, ContentBlock, Event, MessageAccum, NewOrResume, PiClient, PiCommandSender, PiUpdate, UiRequest, UiResponse};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::oneshot;
type ShutdownCompletion = (
    PiClient,
    PiCommandSender,
    Result<project::pi::ShutdownOutcome, String>,
    bool,
);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtensionKind {
    Select { labels: Vec<String> },
    Confirm,
    Text { prefill: String, multiline: bool },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionPrompt { pub id: String, pub title: String, pub message: Option<String>, pub kind: ExtensionKind }
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtensionAnswer { Select(usize), Confirm(bool), Text(String), Cancel }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PiSessionEvent {
    PromptChanged,
    PromptAccepted,
}

/// The sole owner of a Pi process session and its protocol-derived presentation state.
pub struct PiSession {
    runtime: Arc<tokio::runtime::Runtime>,
    client: Option<(PiClient, PiCommandSender)>,
    generation: u64,
    busy: bool,
    closing: bool,
    shutdown_rx: Option<oneshot::Receiver<ShutdownCompletion>>,
    status: String,
    messages: Vec<String>,
    attachments: Vec<ContextItem>,
    prompts: std::collections::HashMap<String, String>,
    pending: std::collections::VecDeque<UiRequest>,
    accum: MessageAccum,
}
impl PiSession {
    pub fn new(runtime: Arc<tokio::runtime::Runtime>, _cx: &mut Context<Self>) -> Self {
        _cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(std::time::Duration::from_millis(30)).await;
                let keep_going = this.update(cx, |this, cx| {
                    this.poll_shutdown(cx);
                    let updates = this.client.as_mut().map(|(client, _)| {
                        let mut updates = Vec::new();
                        while let Some(update) = client.try_recv_update() { updates.push(update); }
                        updates
                    }).unwrap_or_default();
                    for update in updates { this.apply_update(update, cx); }
                }).is_ok();
                if !keep_going { break; }
            }
        }).detach();
        Self { runtime, client: None, generation: 0, busy: false, closing: false, shutdown_rx: None, status: "未连接".into(), messages: vec![], attachments: vec![], prompts: Default::default(), pending: Default::default(), accum: Default::default() }
    }
    pub fn status(&self) -> &str { &self.status }
    pub fn is_connected(&self) -> bool { self.client.is_some() || self.closing || self.busy }
    pub fn messages(&self) -> &[String] { &self.messages }
    pub fn attachments(&self) -> &[ContextItem] { &self.attachments }
    pub fn current_prompt(&self) -> Option<ExtensionPrompt> { self.pending.front().map(extension_prompt) }
    pub fn connect(&mut self, executable: Option<PathBuf>, cwd: PathBuf, choice: NewOrResume, cx: &mut Context<Self>) {
        if self.is_connected() { return; }
        if !cwd.is_absolute() { self.status = "请选择绝对工作目录".into(); cx.notify(); return; }
        self.generation = self.generation.wrapping_add(1);
        self.busy = true;
        self.status = "正在连接 Pi…".into();
        let generation = self.generation;
        let runtime = self.runtime.clone();
        let task = cx.background_spawn(async move {
            let executable = project::pi::discover_pi(executable).map_err(|e| format!("{e:?}"))?;
            let launch = project::pi::build_launch(executable, cwd, choice).map_err(|e| format!("{e:?}"))?;
            let (client, sender) = PiClient::spawn(launch, runtime.handle()).map_err(|e| format!("{e:?}"))?;
            client.verify_connection(&sender).await?;
            Ok::<_, String>((client, sender))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if generation != this.generation {
                    if let Ok((mut client, _)) = result { this.runtime.spawn(async move { let _ = client.shutdown(project::pi::ShutdownRequest { force_after_timeout: true }).await; }); }
                    return;
                }
                this.busy = false;
                match result {
                    Ok((client, sender)) => { this.client = Some((client, sender)); this.status = "已连接".into(); }
                    Err(error) => this.status = error,
                }
                cx.notify();
            });
        }).detach();
    }
    pub fn send(&mut self, draft: String, cx: &mut Context<Self>) {
        let Some((_, sender)) = &self.client else { return; };
        if draft.is_empty() || !self.prompts.is_empty() { return; }
        let id = uuid::Uuid::new_v4().to_string();
        let message = project::pi::wrap_context_for_prompt(&draft, &self.attachments);
        match sender.try_send(project::pi::Command::prompt(Some(id.clone()), message)) {
            Ok(()) => { self.prompts.insert(id, draft); self.status = "等待 Pi 接受消息…".into(); }
            Err(error) => self.status = error,
        }
        cx.notify();
    }
    pub fn attach(&mut self, item: ContextItem, cx: &mut Context<Self>) { self.attachments.push(item); cx.notify(); }
    pub fn remove_attachment(&mut self, index: usize, cx: &mut Context<Self>) { if index < self.attachments.len() { self.attachments.remove(index); cx.notify(); } }
    pub fn reply(&mut self, id: String, answer: ExtensionAnswer, cx: &mut Context<Self>) {
        let Some(request) = self.pending.front() else { return; };
        if request.id != id { return; }
        let (value, confirmed, cancelled) = match answer {
            ExtensionAnswer::Select(index) => {
                let Some(value) = request.payload.get("options").and_then(serde_json::Value::as_array).and_then(|values| values.get(index)).cloned() else {
                    self.status = "Pi Extension UI 选项已失效".into();
                    cx.notify();
                    return;
                };
                (Some(value), None, false)
            }
            ExtensionAnswer::Confirm(value) => (None, Some(value), false),
            ExtensionAnswer::Text(value) => (Some(serde_json::Value::String(value)), None, false),
            ExtensionAnswer::Cancel => (None, None, true),
        };
        let Some((_, sender)) = &self.client else { return; };
        match sender.try_send_ui_response(UiResponse { typ: "extension_ui_response".into(), id, value, confirmed, cancelled: cancelled.then_some(true) }) {
            Ok(()) => { self.pending.pop_front(); cx.notify(); }
            Err(error) => { self.status = error; cx.notify(); }
        }
    }
    pub fn close(&mut self, force: bool, cx: &mut Context<Self>) {
        if self.closing { return; }
        self.generation = self.generation.wrapping_add(1);
        let Some((mut client, sender)) = self.client.take() else {
            self.busy = false;
            return;
        };
        for request in self.pending.drain(..) {
            let _ = sender.try_send_ui_response(UiResponse {
                typ: "extension_ui_response".into(),
                id: request.id,
                value: None,
                confirmed: None,
                cancelled: Some(true),
            });
        }
        self.closing = true;
        let (tx, rx) = oneshot::channel();
        self.shutdown_rx = Some(rx);
        self.runtime.spawn(async move {
            let result = client
                .shutdown(project::pi::ShutdownRequest {
                    force_after_timeout: force,
                })
                .await;
            let _ = tx.send((client, sender, result, force));
        });
        cx.notify();
    }
    fn poll_shutdown(&mut self, cx: &mut Context<Self>) {
        let result = self.shutdown_rx.as_mut().and_then(|receiver| match receiver.try_recv() {
            Ok(result) => Some(Some(result)),
            Err(oneshot::error::TryRecvError::Closed) => Some(None),
            Err(oneshot::error::TryRecvError::Empty) => None,
        });
        let Some(result) = result else { return; };
        self.shutdown_rx = None;
        self.closing = false;
        match result {
            None => self.status = "Pi 关闭任务中断".into(),
            Some((client, sender, Err(error), _)) => {
                self.client = Some((client, sender));
                self.status = format!("关闭失败: {error}");
            }
            Some((client, sender, Ok(project::pi::ShutdownOutcome::TimedOut), false)) => {
                self.client = Some((client, sender));
                self.status = "Pi 未退出，可重试关闭或显式终止".into();
            }
            Some((_, _, Ok(project::pi::ShutdownOutcome::TimedOut), true)) => {
                self.status = "Pi 已终止".into();
                self.prompts.clear();
            }
            Some((_, _, Ok(_), _)) => {
                self.status = "Pi 已关闭".into();
                self.prompts.clear();
            }
        }
        cx.notify();
    }
    /// Apply a protocol update only when it belongs to the currently live client generation.
    pub fn apply_update(&mut self, update: PiUpdate, cx: &mut Context<Self>) {
        match update {
            PiUpdate::State { generation, state } if self.current_generation(generation) => { self.status = format!("{state:?}"); }
            PiUpdate::Event { generation, event } if self.current_generation(generation) => self.apply_event(event),
            PiUpdate::Response { generation, response } if self.current_generation(generation) => {
                if let Some(id) = response.id {
                    if let Some(draft) = self.prompts.remove(&id) {
                        if response.success {
                            self.messages.push(format!("You: {draft}"));
                            self.attachments.clear();
                            cx.emit(PiSessionEvent::PromptAccepted);
                        } else {
                            self.status = response.error.unwrap_or_else(|| "Pi rejected prompt".into());
                        }
                    }
                }
            }
            PiUpdate::UiRequest { generation, request } if self.current_generation(generation) => {
                if ["select", "confirm", "input", "editor"].contains(&request.method.as_str()) {
                    self.pending.push_back(request);
                    cx.emit(PiSessionEvent::PromptChanged);
                } else {
                    self.status = format!("不支持的 Extension UI: {}", request.method);
                    if let Some((_, sender)) = &self.client {
                        let _ = sender.try_send_ui_response(UiResponse {
                            typ: "extension_ui_response".into(),
                            id: request.id,
                            value: None,
                            confirmed: None,
                            cancelled: Some(true),
                        });
                    }
                }
            }
            PiUpdate::Error { generation, error } if self.current_generation(generation) => self.status = error,
            PiUpdate::Disconnected { generation } if self.current_generation(generation) => { self.status = "Pi 进程已退出".into(); self.client = None; self.pending.clear(); self.prompts.clear(); }
            _ => return,
        }
        cx.notify();
    }
    fn current_generation(&self, generation: u64) -> bool { self.client.as_ref().is_some_and(|(_, sender)| sender.current_gen() == generation) }
    fn apply_event(&mut self, event: Event) {
        if event.typ == "message_start" { self.messages.push(String::new()); }
        self.accum.apply_event(&event);
        if matches!(event.typ.as_str(), "message_update" | "message_end") {
            let text = self.accum.blocks.values().map(|block| match block { ContentBlock::Text(t) => t.clone(), ContentBlock::Thinking(t) => format!("Thinking: {t}"), ContentBlock::Tool { name, arguments, .. } => format!("Tool {name}: {arguments}") }).collect::<Vec<_>>().join("\n");
            if let Some(last) = self.messages.last_mut() { *last = text; }
        }
    }
}
impl gpui_kit::EventEmitter<PiSessionEvent> for PiSession {}
fn extension_prompt(request: &UiRequest) -> ExtensionPrompt {
    let payload = &request.payload;
    let kind = match request.method.as_str() {
        "select" => ExtensionKind::Select { labels: payload.get("options").and_then(serde_json::Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect()).unwrap_or_default() },
        "confirm" => ExtensionKind::Confirm,
        _ => ExtensionKind::Text { prefill: payload.get("prefill").or_else(|| payload.get("defaultValue")).and_then(serde_json::Value::as_str).unwrap_or("").to_owned(), multiline: request.method == "editor" },
    };
    ExtensionPrompt { id: request.id.clone(), title: payload.get("title").and_then(serde_json::Value::as_str).unwrap_or("").to_owned(), message: payload.get("message").and_then(serde_json::Value::as_str).map(str::to_owned), kind }
}
