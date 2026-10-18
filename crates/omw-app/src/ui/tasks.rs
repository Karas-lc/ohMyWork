use crate::workbench::{Workbench, WorkbenchEvent};
use gpui_kit::component::{
    button::Button,
    checkbox::Checkbox,
    input::{Input, InputState},
};
use gpui_kit::{Context, Entity, IntoElement, Render, Window, div, prelude::*};
use std::{path::PathBuf, sync::Arc};
#[derive(Clone, Debug)]
pub struct TaskIntent {
    pub path: PathBuf,
    pub version: omw_local::DiskVersion,
    pub anchor: omw_document::TaskAnchor,
    pub update: omw_document::TaskFieldUpdate,
}
#[derive(Clone)]
pub struct TaskSummary {
    pub title: String,
    pub checked: bool,
    pub intent: TaskIntent,
}
#[derive(Clone)]
struct Row {
    path: PathBuf,
    version: omw_local::DiskVersion,
    anchor: omw_document::TaskAnchor,
    task: omw_document::task::Task,
    in_archive: bool,
}
pub struct TasksView {
    root: PathBuf,
    rows: Vec<Row>,
    include_archive: bool,
    show_completed: bool,
    selected: Option<usize>,
    value: Entity<InputState>,
    status: String,
    generation: u64,
}
impl TasksView {
    pub fn new(
        _index: Arc<omw_local::IndexStore>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let root = cx.global::<Workbench>().root.clone();
        let mut this = Self {
            root,
            rows: vec![],
            include_archive: false,
            show_completed: false,
            selected: None,
            value: cx
                .new(|cx| InputState::new(window, cx).placeholder("YYYY-MM-DD / 1..9；留空清除")),
            status: String::new(),
            generation: 0,
        };
        this.reload(cx);
        this
    }
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        let root = self.root.clone();
        let include = self.include_archive;
        self.status = "正在读取任务…".into();
        let task = cx.background_spawn(async move {
            let report = omw_local::scan_workspace(&root, omw_local::ScanOptions::default())
                .map_err(|e| e.to_string())?;
            let mut rows = Vec::new();
            let mut seen = std::collections::HashSet::new();
            for entry in report.entries() {
                if !entry.is_file()
                    || entry.is_excluded()
                    || (!include && entry.logical_path().starts_with("Archive"))
                {
                    continue;
                }
                if entry
                    .target_path()
                    .extension()
                    .and_then(|s| s.to_str())
                    .is_none_or(|s| !s.eq_ignore_ascii_case("md"))
                {
                    continue;
                }
                if !seen.insert(entry.target_path().to_path_buf()) {
                    continue;
                }
                let snapshot =
                    omw_local::read_snapshot(entry.target_path()).map_err(|e| e.to_string())?;
                let doc = omw_document::parse(&snapshot.bytes).map_err(|e| e.to_string())?;
                for item in omw_document::task::task_items(doc.tree()) {
                    if let (Some(task), Some(range)) = (
                        omw_document::task::parse_task(&doc, &item),
                        doc.tree().node(item.node).source,
                    ) {
                        rows.push(Row {
                            path: entry.target_path().to_path_buf(),
                            version: snapshot.version.clone(),
                            anchor: omw_document::TaskAnchor {
                                item_range: range,
                                checkbox_range: item.checkbox_range,
                            },
                            task,
                            in_archive: entry.logical_path().starts_with("Archive"),
                        });
                    }
                }
            }
            rows.sort_by(|a, b| {
                a.task
                    .item
                    .checked
                    .cmp(&b.task.item.checked)
                    .then_with(|| {
                        b.task
                            .priority
                            .as_ref()
                            .map(|p| p.0)
                            .cmp(&a.task.priority.as_ref().map(|p| p.0))
                    })
                    .then_with(|| {
                        a.task
                            .due
                            .as_ref()
                            .map(|d| d.0.as_str())
                            .unwrap_or("9999-12-31")
                            .cmp(
                                b.task
                                    .due
                                    .as_ref()
                                    .map(|d| d.0.as_str())
                                    .unwrap_or("9999-12-31"),
                            )
                    })
                    .then_with(|| a.path.cmp(&b.path))
                    .then_with(|| a.anchor.item_range.start.cmp(&b.anchor.item_range.start))
            });
            Ok::<_, String>(rows)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok(rows) => {
                        *cx.global_mut::<Workbench>().tasks.borrow_mut() = rows
                            .iter()
                            .filter(|r| !r.in_archive && !r.task.item.checked)
                            .map(|r| TaskSummary {
                                title: r.task.title.clone(),
                                checked: r.task.item.checked,
                                intent: TaskIntent {
                                    path: r.path.clone(),
                                    version: r.version.clone(),
                                    anchor: r.anchor,
                                    update: omw_document::TaskFieldUpdate::Complete {
                                        checked: true,
                                    },
                                },
                            })
                            .collect();
                        this.status = format!("{} 个任务", rows.len());
                        this.rows = rows;
                        this.selected = None;
                    }
                    Err(e) => this.status = e,
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn intent(&self, index: usize, update: omw_document::TaskFieldUpdate, cx: &mut Context<Self>) {
        if let Some(row) = self.rows.get(index) {
            cx.global::<Workbench>()
                .emit(WorkbenchEvent::Task(TaskIntent {
                    path: row.path.clone(),
                    version: row.version.clone(),
                    anchor: row.anchor,
                    update,
                }));
        }
    }
}
impl Render for TasksView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut rows = div().id("tasks-list").flex().flex_col().gap_2();
        for (i, row) in self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, r)| self.show_completed || !r.task.item.checked)
        {
            let path = row.path.clone();
            rows = rows.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .p_1()
                    .child(
                        Checkbox::new(("task", i))
                            .label(row.task.title.clone())
                            .checked(row.task.item.checked)
                            .on_change(cx.listener(move |this, checked, _, cx| {
                                this.intent(
                                    i,
                                    omw_document::TaskFieldUpdate::Complete { checked: *checked },
                                    cx,
                                )
                            })),
                    )
                    .child(
                        Button::new(("task-fields", i))
                            .label(format!(
                                "{} · {:?}",
                                row.path.file_name().unwrap_or_default().to_string_lossy(),
                                row.task.priority.as_ref().map(|p| p.0)
                            ))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.selected = Some(i);
                                cx.notify();
                            })),
                    )
                    .child(Button::new(("task-open", i)).label("定位任务").on_click(
                        move |_, _, cx| {
                            cx.global::<Workbench>()
                                .emit(WorkbenchEvent::Open(path.clone()))
                        },
                    )),
            );
        }
        let mut body = div()
            .size_full()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                Checkbox::new("show-completed")
                    .label("显示已完成")
                    .checked(self.show_completed)
                    .on_change(cx.listener(|this, checked, _, cx| {
                        this.show_completed = *checked;
                        cx.notify();
                    })),
            )
            .child(
                Checkbox::new("tasks-archive")
                    .label("包含 Archive")
                    .checked(self.include_archive)
                    .on_change(cx.listener(|this, checked, _, cx| {
                        this.include_archive = *checked;
                        this.reload(cx);
                    })),
            )
            .child(
                Button::new("reload-tasks")
                    .label("刷新任务")
                    .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
            )
            .child(
                div()
                    .id("task-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .child(rows),
            )
            .child(self.status.clone());
        if let Some(index) = self.selected {
            body = body
                .child(
                    Input::new(&self.value)
                        .id("task-field-value")
                        .aria_label("任务字段值"),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .child(Button::new("task-priority").label("设优先级").on_click(
                            cx.listener(move |this, _, _, cx| {
                                let value = this.value.read(cx).value().to_string();
                                let parsed = if value.trim().is_empty() {
                                    None
                                } else {
                                    match value.parse::<u8>() {
                                        Ok(p) if (1..=9).contains(&p) => Some(p),
                                        _ => {
                                            this.status = "优先级须为 1..9".into();
                                            cx.notify();
                                            return;
                                        }
                                    }
                                };
                                this.intent(
                                    index,
                                    omw_document::TaskFieldUpdate::SetPriority { value: parsed },
                                    cx,
                                );
                            }),
                        ))
                        .child(
                            Button::new("task-start")
                                .label("设开始日期")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let value = this.value.read(cx).value().to_string();
                                    this.intent(
                                        index,
                                        omw_document::TaskFieldUpdate::SetDate {
                                            which: omw_document::TaskDateField::Start,
                                            value: (!value.is_empty()).then_some(value),
                                        },
                                        cx,
                                    );
                                })),
                        )
                        .child(
                            Button::new("task-due")
                                .label("设截止日期")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let value = this.value.read(cx).value().to_string();
                                    this.intent(
                                        index,
                                        omw_document::TaskFieldUpdate::SetDate {
                                            which: omw_document::TaskDateField::Due,
                                            value: (!value.is_empty()).then_some(value),
                                        },
                                        cx,
                                    );
                                })),
                        ),
                );
        }
        body
    }
}
