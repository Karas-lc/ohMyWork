use crate::pi::{ExtensionAnswer, ExtensionKind, PiSession, PiSessionEvent};
use gpui_kit::component::{
    button::Button,
    input::{Input, InputState, Textarea, TextareaState},
};
use gpui_kit::{Context, Entity, IntoElement, Render, Subscription, Window, div, prelude::*, px};
use gpui_kit::TestSupportExt as _;
use project::pi::{ContextItem, NewOrResume};
use std::path::PathBuf;

/// UI-only Pi controls; process, protocol, messages, and prompts belong to PiSession.
pub struct PiView {
    session: Entity<PiSession>,
    cwd: Entity<InputState>,
    executable: Entity<InputState>,
    session_path: Entity<InputState>,
    draft: Entity<TextareaState>,
    answer: Entity<TextareaState>,
    _subscriptions: Vec<Subscription>,
}
impl PiView {
    pub fn new(session: Entity<PiSession>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&session, |_this, _, cx| cx.notify());
        let events = cx.subscribe_in(&session, window, |this, _, event: &PiSessionEvent, window, cx| {
            match event {
                PiSessionEvent::PromptChanged => {
                    let prefill = this.session.read(cx).current_prompt().map(|prompt| match prompt.kind {
                        ExtensionKind::Text { prefill, .. } => prefill,
                        _ => String::new(),
                    }).unwrap_or_default();
                    this.answer.update(cx, |answer, cx| answer.set_value(&prefill, window, cx));
                }
                PiSessionEvent::PromptAccepted => this.draft.update(cx, |draft, cx| draft.set_value("", window, cx)),
            }
        });
        Self {
            session,
            cwd: cx.new(|cx| InputState::new(window, cx).placeholder("选择工作目录（可在 Workspace 外）")),
            executable: cx.new(|cx| InputState::new(window, cx).placeholder("Pi 程序或命令；留空使用 PATH")),
            session_path: cx.new(|cx| InputState::new(window, cx).placeholder("留空新建，或选择已有 session.jsonl")),
            draft: cx.new(|cx| TextareaState::new(window, cx).placeholder("输入消息")),
            answer: cx.new(|cx| TextareaState::new(window, cx)),
            _subscriptions: vec![observe, events],
        }
    }
    fn connect(&mut self, cx: &mut Context<Self>) {
        let cwd = PathBuf::from(self.cwd.read(cx).value().as_ref());
        let executable = self.executable.read(cx).value().to_string();
        let session = self.session_path.read(cx).value().to_string();
        let executable = (!executable.trim().is_empty()).then(|| PathBuf::from(executable));
        let choice = if session.trim().is_empty() {
            NewOrResume::New
        } else {
            NewOrResume::Resume(PathBuf::from(session))
        };
        self.session.update(cx, |session, cx| session.connect(executable, cwd, choice, cx));
    }
    fn pick(&mut self, kind: u8, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: kind != 0,
            directories: kind == 0,
            multiple: kind == 2,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = paths.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if let Ok(Ok(Some(paths))) = result {
                    for path in paths {
                        let value = path.to_string_lossy().into_owned();
                        match kind {
                            0 => this.cwd.update(cx, |state, cx| state.set_value(&value, window, cx)),
                            1 => this.session_path.update(cx, |state, cx| state.set_value(&value, window, cx)),
                            _ => this.session.update(cx, |session, cx| session.attach(ContextItem::file_only(path), cx)),
                        }
                    }
                }
                cx.notify();
            });
        }).detach();
    }
    fn send(&mut self, cx: &mut Context<Self>) {
        let draft = self.draft.read(cx).value().to_string();
        self.session.update(cx, |session, cx| session.send(draft, cx));
    }
}
impl Render for PiView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session.read(cx);
        let mut body = div()
            .id("pi-panel")
            .size_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .child(Input::new(&self.executable).id("pi-executable").aria_label("Pi 程序"))
            .child(Input::new(&self.cwd).id("pi-cwd").aria_label("Pi 工作目录"))
            .child(Button::new("pi-choose-cwd").label("选择工作目录").on_click(cx.listener(|this, _, window, cx| this.pick(0, window, cx))))
            .child(Input::new(&self.session_path).id("pi-session").aria_label("Pi 会话路径；留空新建"))
            .child(Button::new("pi-choose-session").label("选择已有会话").on_click(cx.listener(|this, _, window, cx| this.pick(1, window, cx))))
            .child(Button::new("pi-connect").label("启动 Pi").on_click(cx.listener(|this, _, _, cx| this.connect(cx))))
            .child(Button::new("pi-disconnect").label("关闭 Pi").on_click(cx.listener(|this, _, _, cx| this.session.update(cx, |session, cx| session.close(false, cx)))))
            .child(Button::new("pi-force-stop").label("终止未退出的 Pi").on_click(cx.listener(|this, _, _, cx| this.session.update(cx, |session, cx| session.close(true, cx)))))
            .child(session.status().to_owned());
        let mut messages = div().flex().flex_col();
        for message in session.messages() {
            messages = messages.child(message.clone());
        }
        body = body.child(div().id("pi-messages").flex_1().overflow_y_scroll().child(messages));
        for (index, item) in session.attachments().iter().enumerate() {
            let label = project::pi::display_path(item, None);
            body = body.child(div().child(label).child(
                Button::new(("remove-attachment", index)).label("移除").on_click(cx.listener(move |this, _, _, cx| {
                    this.session.update(cx, |session, cx| session.remove_attachment(index, cx));
                })),
            ));
        }
        body = body
            .child(Button::new("pi-add-attach").label("附加文件路径").on_click(cx.listener(|this, _, window, cx| this.pick(2, window, cx))))
            .child(div().id("pi-draft").test_support().child(Textarea::new(&self.draft).aria_label("Pi 消息草稿").min_h(px(90.))))
            .child(Button::new("pi-send").label("发送").on_click(cx.listener(|this, _, _, cx| this.send(cx))));
        if let Some(prompt) = session.current_prompt() {
            let id = prompt.id.clone();
            let mut dialog = div().border_1().p_2().child(prompt.title);
            if let Some(message) = prompt.message { dialog = dialog.child(message); }
            match prompt.kind {
                ExtensionKind::Select { labels } => {
                    for (index, label) in labels.into_iter().enumerate() {
                        let option_id = id.clone();
                        dialog = dialog.child(Button::new(("pi-option", index)).label(label).on_click(cx.listener(move |this, _, _, cx| {
                            this.session.update(cx, |session, cx| session.reply(option_id.clone(), ExtensionAnswer::Select(index), cx));
                        })));
                    }
                }
                ExtensionKind::Confirm => {
                    let yes_id = id.clone();
                    let no_id = id.clone();
                    dialog = dialog
                        .child(Button::new("pi-confirm").label("确认").on_click(cx.listener(move |this, _, _, cx| {
                            this.session.update(cx, |session, cx| session.reply(yes_id.clone(), ExtensionAnswer::Confirm(true), cx));
                        })))
                        .child(Button::new("pi-reject").label("拒绝").on_click(cx.listener(move |this, _, _, cx| {
                            this.session.update(cx, |session, cx| session.reply(no_id.clone(), ExtensionAnswer::Confirm(false), cx));
                        })));
                }
                ExtensionKind::Text { .. } => {
                    let text_id = id.clone();
                    dialog = dialog
                        .child(div().id("pi-extension-answer").child(Textarea::new(&self.answer).aria_label("Pi Extension UI 回复")))
                        .child(Button::new("pi-answer").label("提交").on_click(cx.listener(move |this, _, _, cx| {
                            let value = this.answer.read(cx).value().to_string();
                            this.session.update(cx, |session, cx| session.reply(text_id.clone(), ExtensionAnswer::Text(value), cx));
                        })));
                }
            }
            let cancel_id = id.clone();
            dialog = dialog.child(Button::new("pi-cancel-request").label("取消").on_click(cx.listener(move |this, _, _, cx| {
                this.session.update(cx, |session, cx| session.reply(cancel_id.clone(), ExtensionAnswer::Cancel, cx));
            })));
            body = body.child(dialog);
        }
        body
    }
}
