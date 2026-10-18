use crate::workbench::Workbench;
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::{
    button::Button,
    input::{Input, InputState, Textarea, TextareaState},
};
use gpui_kit::{Context, Entity, IntoElement, Render, Window, div, prelude::*, px};
use omw_pi::{
    ContextItem, NewOrResume, PiClient, PiCommandSender, PiUpdate, UiRequest, UiResponse,
    wrap_context_for_prompt,
};
use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
pub struct PiPanel {
    status: String,
    cwd: Entity<InputState>,
    executable: Entity<InputState>,
    session: Entity<InputState>,
    draft: Entity<TextareaState>,
    answer: Entity<TextareaState>,
    rt: Option<Arc<tokio::runtime::Handle>>,
    client: Option<(PiClient, PiCommandSender)>,
    busy: bool,
    closing: bool,
    generation: u64,
    attachments: Vec<ContextItem>,
    messages: Vec<String>,
    pending: VecDeque<UiRequest>,
    prompts: HashMap<String, String>,
    accum: omw_pi::MessageAccum,
    state: omw_pi::SessionState,
    pending_requests: omw_pi::PendingRequests,
    pending_path: Option<(u8, PathBuf)>,
}
impl PiPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(30))
                    .await;
                if this
                    .update_in(cx, |this, window, cx| this.poll(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        Self {
            status: "未连接".into(),
            cwd: cx.new(|cx| {
                InputState::new(window, cx).placeholder("选择工作目录（可以在 Workspace 外）")
            }),
            executable: cx
                .new(|cx| InputState::new(window, cx).placeholder("Pi 程序或命令；留空使用 PATH")),
            session: cx.new(|cx| {
                InputState::new(window, cx).placeholder("留空新建，或选择已有 session.jsonl")
            }),
            draft: cx.new(|cx| TextareaState::new(window, cx).placeholder("输入消息")),
            answer: cx.new(|cx| TextareaState::new(window, cx)),
            rt: None,
            client: None,
            busy: false,
            closing: false,
            generation: 0,
            attachments: vec![],
            messages: vec![],
            pending: VecDeque::new(),
            prompts: HashMap::new(),
            accum: Default::default(),
            state: Default::default(),
            pending_requests: Default::default(),
            pending_path: None,
        }
    }
    pub fn set_runtime(&mut self, handle: Arc<tokio::runtime::Handle>) {
        self.rt = Some(handle);
    }
    pub fn status(&self) -> &str {
        &self.status
    }
    pub fn is_connected(&self) -> bool {
        self.client.is_some() || self.closing || self.busy
    }
    fn connect(&mut self, cx: &mut Context<Self>) {
        if self.is_connected() {
            return;
        }
        let Some(rt) = self.rt.clone() else {
            self.status = "Pi runtime unavailable".into();
            return;
        };
        let cwd = PathBuf::from(self.cwd.read(cx).value().as_ref());
        if !cwd.is_absolute() {
            self.status = "请选择绝对工作目录".into();
            cx.notify();
            return;
        }
        let exec = self.executable.read(cx).value().to_string();
        let session = self.session.read(cx).value().to_string();
        self.busy = true;
        self.generation += 1;
        let generation = self.generation;
        let task = cx.background_spawn(async move {
            let exec = omw_pi::discover_pi((!exec.trim().is_empty()).then(|| PathBuf::from(exec)))
                .map_err(|e| format!("{e:?}"))?;
            let choice = if session.is_empty() {
                NewOrResume::New
            } else {
                NewOrResume::Resume(PathBuf::from(session))
            };
            let launch = omw_pi::build_launch(exec, cwd, choice).map_err(|e| format!("{e:?}"))?;
            PiClient::spawn(launch, &rt).map_err(|e| format!("{e:?}"))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.busy = false;
                match result {
                    Ok((client, sender)) => {
                        let _ = sender.try_send(omw_pi::Command {
                            id: Some("verify-state".into()),
                            body: omw_pi::CommandBody::GetState,
                        });
                        this.client = Some((client, sender));
                        this.status = "正在核验 RPC 连接…".into();
                    }
                    Err(e) => this.status = e,
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.disconnect(false, cx);
    }
    fn disconnect(&mut self, force: bool, cx: &mut Context<Self>) {
        let Some((mut client, sender)) = self.client.take() else {
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
        self.status = "正在 abort 并关闭 Pi…".into();
        let Some(rt) = self.rt.clone() else {
            return;
        };
        let task = rt.spawn(async move {
            let result = client
                .shutdown(omw_pi::ShutdownRequest {
                    force_after_timeout: force,
                })
                .await;
            (client, result)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.closing = false;
                match result {
                    Ok((client, omw_pi::ShutdownOutcome::TimedOut)) => {
                        this.client = Some((client, sender));
                        this.status = "Pi 未退出，可重试关闭或显式终止".into();
                    }
                    Ok(_) => {
                        this.status = "Pi 已关闭".into();
                        this.prompts.clear();
                    }
                    Err(e) => this.status = format!("关闭失败: {e}"),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn poll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut updates = vec![];
        if let Some((client, _)) = &mut self.client {
            while let Some(update) = client.try_recv_update() {
                updates.push(update);
            }
        }
        let changed = !updates.is_empty();
        for update in updates {
            match update {
                PiUpdate::State { state, .. } => {
                    self.state = state;
                    self.status = format!("{:?}", self.state);
                }
                PiUpdate::Response { response, .. } => {
                    if let Some(id) = &response.id {
                        if let Some(draft) = self.prompts.remove(id) {
                            if response.success {
                                self.messages.push(format!("You: {draft}"));
                                self.attachments.clear();
                                self.draft.update(cx, |d, cx| d.set_value("", window, cx));
                            } else {
                                self.status = response
                                    .error
                                    .unwrap_or_else(|| "Pi rejected prompt".into());
                            }
                        } else if id == "verify-state" {
                            self.status = if response.success {
                                "已连接".into()
                            } else {
                                response
                                    .error
                                    .unwrap_or_else(|| "RPC verification failed".into())
                            };
                        }
                    }
                }
                PiUpdate::Event { event, .. } => {
                    if event.typ == "message_start" {
                        self.messages.push(String::new());
                        self.accum = Default::default();
                    }
                    self.state = omw_pi::reducer::reduce_event(
                        self.state.clone(),
                        &event,
                        &mut self.accum,
                        &mut self.pending_requests,
                    );
                    if event.typ == "message_update" || event.typ == "message_end" {
                        let text = self
                            .accum
                            .blocks
                            .values()
                            .map(|block| match block {
                                omw_pi::ContentBlock::Text(t) => t.clone(),
                                omw_pi::ContentBlock::Thinking(t) => format!("Thinking: {t}"),
                                omw_pi::ContentBlock::Tool {
                                    name, arguments, ..
                                } => format!("Tool {name}: {arguments}"),
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        if let Some(last) = self.messages.last_mut() {
                            *last = text;
                        } else {
                            self.messages.push(text);
                        }
                    }
                }
                PiUpdate::UiRequest { request, .. } => {
                    if ["select", "confirm", "input", "editor"].contains(&request.method.as_str()) {
                        if self.pending.is_empty() {
                            let initial = request
                                .payload
                                .get("prefill")
                                .or_else(|| request.payload.get("defaultValue"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            self.answer
                                .update(cx, |a, cx| a.set_value(initial, window, cx));
                        }
                        self.pending.push_back(request);
                    } else {
                        self.status = format!("不支持的 Extension UI: {}", request.method);
                        self.respond_request(request.id, None, None, true);
                    }
                }
                PiUpdate::Error { error, .. } => self.status = error,
                PiUpdate::Disconnected { .. } => {
                    self.status = "Pi 进程已退出".into();
                    self.client = None;
                    self.pending.clear();
                    self.prompts.clear();
                }
                PiUpdate::Stderr { .. } => {}
            }
        }
        if changed && cx.try_global::<Workbench>().is_some() {
            *cx.global_mut::<Workbench>().pi_status.borrow_mut() = self.status.clone();
        }
        if changed {
            cx.notify();
        }
    }
    fn respond_request(
        &self,
        id: String,
        value: Option<serde_json::Value>,
        confirmed: Option<bool>,
        cancelled: bool,
    ) {
        if let Some((_, sender)) = &self.client {
            let _ = sender.try_send_ui_response(UiResponse {
                typ: "extension_ui_response".into(),
                id,
                value,
                confirmed,
                cancelled: cancelled.then_some(true),
            });
        }
    }
    fn reply(
        &mut self,
        value: Option<serde_json::Value>,
        confirmed: Option<bool>,
        cancelled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(request) = self.pending.pop_front() {
            self.respond_request(request.id, value, confirmed, cancelled);
        }
        if let Some(next) = self.pending.front() {
            let initial = next
                .payload
                .get("prefill")
                .or_else(|| next.payload.get("defaultValue"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            self.answer
                .update(cx, |a, cx| a.set_value(initial, window, cx));
        }
        cx.notify();
    }
    fn pick(&mut self, kind: u8, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: kind != 0,
            directories: kind == 0,
            multiple: kind == 2,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            let result = paths.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(Ok(Some(paths))) => {
                        for path in paths {
                            match kind {
                                0 => {
                                    this.status = format!("工作目录: {}", path.display());
                                    this.pending_path = Some((0, path));
                                }
                                1 => this.pending_path = Some((1, path)),
                                _ => this.attachments.push(ContextItem::file_only(path)),
                            }
                        }
                    }
                    Ok(Ok(None)) => {}
                    _ => this.status = "文件选择失败".into(),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn send(&mut self, cx: &mut Context<Self>) {
        let Some((_, sender)) = &self.client else {
            return;
        };
        if !self.prompts.is_empty() {
            return;
        }
        let draft = self.draft.read(cx).value().to_string();
        if draft.is_empty() {
            return;
        }
        let id = uuid::Uuid::new_v4().to_string();
        let message = wrap_context_for_prompt(&draft, &self.attachments);
        match sender.try_send(omw_pi::Command::prompt(Some(id.clone()), message)) {
            Ok(()) => {
                self.prompts.insert(id, draft);
                self.status = "等待 Pi 接受消息…".into();
            }
            Err(e) => self.status = e,
        }
        cx.notify();
    }
}
impl Render for PiPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some((kind, path)) = self.pending_path.take() {
            let state = if kind == 0 { &self.cwd } else { &self.session };
            state.update(cx, |s, cx| {
                s.set_value(path.to_string_lossy().into_owned(), window, cx)
            });
        }
        let mut body = div()
            .id("pi-panel")
            .size_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .child(
                Input::new(&self.executable)
                    .id("pi-executable")
                    .aria_label("Pi 程序"),
            )
            .child(Input::new(&self.cwd).id("pi-cwd").aria_label("Pi 工作目录"))
            .child(
                Button::new("pi-choose-cwd")
                    .label("选择工作目录")
                    .on_click(cx.listener(|this, _, _, cx| this.pick(0, cx))),
            )
            .child(
                Input::new(&self.session)
                    .id("pi-session")
                    .aria_label("Pi 会话路径；留空新建"),
            )
            .child(
                Button::new("pi-choose-session")
                    .label("选择已有会话")
                    .on_click(cx.listener(|this, _, _, cx| this.pick(1, cx))),
            )
            .child(
                Button::new("pi-connect")
                    .label("启动 Pi")
                    .on_click(cx.listener(|this, _, _, cx| this.connect(cx))),
            )
            .child(
                Button::new("pi-disconnect")
                    .label("关闭 Pi")
                    .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
            )
            .child(
                Button::new("pi-force-stop")
                    .label("终止未退出的 Pi")
                    .on_click(cx.listener(|this, _, _, cx| this.disconnect(true, cx))),
            )
            .child(self.status.clone());
        let mut messages = div().flex().flex_col();
        for text in &self.messages {
            messages = messages.child(text.clone());
        }
        body = body.child(
            div()
                .id("pi-messages")
                .flex_1()
                .overflow_y_scroll()
                .child(messages),
        );
        for (i, item) in self.attachments.iter().enumerate() {
            body = body.child(
                div().child(omw_pi::display_path(item, None)).child(
                    Button::new(("remove-attachment", i))
                        .label("移除")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.attachments.remove(i);
                            cx.notify();
                        })),
                ),
            );
        }
        body = body
            .child(
                Button::new("pi-add-attach")
                    .label("附加文件路径")
                    .on_click(cx.listener(|this, _, _, cx| this.pick(2, cx))),
            )
            .child(
                div().id("pi-draft").test_support().child(
                    Textarea::new(&self.draft)
                        .aria_label("Pi 消息草稿")
                        .min_h(px(90.)),
                ),
            )
            .child(
                Button::new("pi-send")
                    .label("发送")
                    .on_click(cx.listener(|this, _, _, cx| this.send(cx))),
            );
        if let Some(request) = self.pending.front().cloned() {
            let mut dialog = div().border_1().p_2().child(
                request
                    .payload
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Pi Extension UI")
                    .to_string(),
            );
            if let Some(message) = request.payload.get("message").and_then(|v| v.as_str()) {
                dialog = dialog.child(message.to_string());
            }
            if request.method == "select" {
                if let Some(options) = request.payload.get("options").and_then(|v| v.as_array()) {
                    for (i, value) in options.iter().enumerate() {
                        let value = value.clone();
                        dialog = dialog.child(
                            Button::new(("pi-option", i))
                                .label(value.as_str().unwrap_or("").to_string())
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.reply(Some(value.clone()), None, false, window, cx)
                                })),
                        );
                    }
                }
            } else if request.method == "confirm" {
                dialog = dialog.child(Button::new("pi-confirm").label("确认").on_click(
                    cx.listener(|this, _, window, cx| {
                        this.reply(None, Some(true), false, window, cx)
                    }),
                ));
            } else {
                dialog = dialog
                    .child(
                        div()
                            .id("pi-extension-answer")
                            .child(Textarea::new(&self.answer).aria_label("Pi Extension UI 回复")),
                    )
                    .child(Button::new("pi-answer").label("提交").on_click(cx.listener(
                        |this, _, window, cx| {
                            let value = this.answer.read(cx).value().to_string();
                            this.reply(
                                Some(serde_json::Value::String(value)),
                                None,
                                false,
                                window,
                                cx,
                            );
                        },
                    )));
            }
            dialog = dialog.child(Button::new("pi-cancel-request").label("取消").on_click(
                cx.listener(|this, _, window, cx| this.reply(None, None, true, window, cx)),
            ));
            body = body.child(dialog);
        }
        body
    }
}
