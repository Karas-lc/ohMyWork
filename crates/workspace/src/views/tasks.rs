use editor::documents::TaskIntent;
use gpui_kit::component::{button::Button, checkbox::Checkbox, input::{Input, InputState}};
use gpui_kit::{Context, Entity, IntoElement, Render, Subscription, Window, div, prelude::*};

use crate::queries::tasks::{TaskRow, TasksModel};

pub struct TasksView {
    model: Entity<TasksModel>,
    session: Entity<crate::session::WorkspaceSession>,
    show_completed: bool,
    selected: Option<usize>,
    value: Entity<InputState>,
    action_status: Option<String>,
    _model_subscription: Subscription,
}

impl TasksView {
    pub fn new(model: Entity<TasksModel>, session: Entity<crate::session::WorkspaceSession>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let value = cx.new(|cx| InputState::new(window, cx).placeholder("YYYY-MM-DD / 1..9；留空清除"));
        let model_subscription = cx.observe(&model, |this, _, cx| {
            this.selected = None;
            this.action_status = None;
            cx.notify();
        });
        Self { model, session, show_completed: false, selected: None, value, action_status: None, _model_subscription: model_subscription }
    }


    fn intent(&mut self, row: &TaskRow, update: editor::TaskFieldUpdate, cx: &mut Context<Self>) {
        let intent = TaskIntent { path: row.path.clone(), version: row.version.clone(), anchor: row.anchor, update };
        match self.model.update(cx, |model, cx| model.commit_task(intent, cx)) {
            Ok(()) => self.action_status = Some("正在提交任务字段…".into()),
            Err(error) => self.action_status = Some(error.to_string()),
        }
        cx.notify();
    }
}

impl Render for TasksView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let model = self.model.read(cx);
        let rows_snapshot = model.rows().to_vec();
        let mut rows = div().id("tasks-list").flex().flex_col().gap_2();
        for (i, row) in rows_snapshot.iter().enumerate().filter(|(_, row)| self.show_completed || !row.task.item.checked) {
            let path = row.path.clone();
            let completion_row = row.clone();
            let title = row.task.title.clone();
            let checked = row.task.item.checked;
            rows = rows.child(div().flex().flex_col().gap_1().p_1()
                .child(Checkbox::new(("task", i)).label(title).checked(checked)
                    .on_change(cx.listener(move |this, checked, _, cx| this.intent(&completion_row, editor::TaskFieldUpdate::Complete { checked: *checked }, cx))))
                .child(Button::new(("task-fields", i)).label(format!("{} · {:?}", row.path.file_name().unwrap_or_default().to_string_lossy(), row.task.priority.as_ref().map(|p| p.0)))
                    .on_click(cx.listener(move |this, _, _, cx| { this.selected = Some(i); cx.notify(); })))
                .child(Button::new(("task-open", i)).label("定位任务").on_click(cx.listener(move |this, _, _, cx| {
                    this.session.update(cx, |session, cx| session.request_open(path.clone(), cx));
                }))));
        }
        let include_archive = model.include_archive();
        let status = self.action_status.clone().unwrap_or_else(|| model.status().to_owned());
        let mut body = div().size_full().flex().flex_col().gap_2()
            .child(Checkbox::new("show-completed").label("显示已完成").checked(self.show_completed)
                .on_change(cx.listener(|this, checked, _, cx| { this.show_completed = *checked; cx.notify(); })))
            .child(Checkbox::new("tasks-archive").label("包含 Archive").checked(include_archive)
                .on_change(cx.listener(|this, checked, _, cx| this.model.update(cx, |model, cx| model.set_include_archive(*checked, cx)))))
            .child(Button::new("reload-tasks").label("刷新任务").on_click(cx.listener(|this, _, _, cx| this.model.update(cx, |model, cx| model.reload(cx)))))
            .child(div().id("task-scroll").flex_1().overflow_y_scroll().child(rows))
            .child(status);
        if let Some(index) = self.selected {
            body = body.child(Input::new(&self.value).id("task-field-value").aria_label("任务字段值"))
                .child(div().flex().flex_wrap()
                    .child(Button::new("task-priority").label("设优先级").on_click(cx.listener(move |this, _, _, cx| {
                        let value = this.value.read(cx).value().to_string();
                        let parsed = if value.trim().is_empty() { None } else { match value.parse::<u8>() {
                            Ok(priority) if (1..=9).contains(&priority) => Some(priority),
                            _ => { this.action_status = Some("优先级须为 1..9".into()); cx.notify(); return; }
                        }};
                        let row = this.model.read(cx).rows().get(index).cloned();
                        if let Some(row) = row { this.intent(&row, editor::TaskFieldUpdate::SetPriority { value: parsed }, cx); }
                    })))
                    .child(Button::new("task-start").label("设开始日期").on_click(cx.listener(move |this, _, _, cx| {
                        let value = this.value.read(cx).value().to_string();
                        let row = this.model.read(cx).rows().get(index).cloned();
                        if let Some(row) = row { this.intent(&row, editor::TaskFieldUpdate::SetDate { which: editor::TaskDateField::Start, value: (!value.is_empty()).then_some(value) }, cx); }
                    })))
                    .child(Button::new("task-due").label("设截止日期").on_click(cx.listener(move |this, _, _, cx| {
                        let value = this.value.read(cx).value().to_string();
                        let row = this.model.read(cx).rows().get(index).cloned();
                        if let Some(row) = row { this.intent(&row, editor::TaskFieldUpdate::SetDate { which: editor::TaskDateField::Due, value: (!value.is_empty()).then_some(value) }, cx); }
                    }))));
        }
        body
    }
}
