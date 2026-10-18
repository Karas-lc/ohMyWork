use super::{
    BuiltinWidgetKind, CanvasController, CanvasPersist, ScreenPoint, Widget, WorldPoint, WorldRect,
    WorldSize,
};
use crate::workbench::{PersistChange, Workbench, WorkbenchEvent};
use gpui_kit::component::{
    ActiveTheme, Disableable,
    button::Button,
    checkbox::Checkbox,
    dock::{BasePanel, Panel, PanelEvent},
};
use gpui_kit::{
    App, Bounds, Context, EventEmitter, FocusHandle, Focusable, IntoElement, MouseButton, Pixels,
    Render, Subscription, TestSupportExt as _, Window, div, prelude::*, px,
};
use std::{cell::Cell, path::PathBuf, rc::Rc};

type PointerDrag = (uuid::Uuid, gpui_kit::Point<Pixels>);
pub struct HomePanel {
    focus: FocusHandle,
    controller: CanvasController,
    root: PathBuf,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    drag: Option<PointerDrag>,
    resize_drag: Option<PointerDrag>,
    pan_drag: Option<gpui_kit::Point<Pixels>>,
    layout_revision: u64,
    status: String,
    _services: Subscription,
}
impl HomePanel {
    pub fn new(root: PathBuf, _: &mut Window, cx: &mut Context<Self>) -> Self {
        let read_root = root.clone();
        let paths = cx.try_global::<Workbench>().map(|s| s.app_data.clone());
        let task =
            cx.background_spawn(async move { paths.map(|p| CanvasPersist::load(&p, &read_root)) });
        cx.spawn(async move |this, cx| {
            let state = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.layout_revision == 0 {
                    if let Some(canvas) = state.and_then(|s| s.canvas) {
                        this.controller.state = canvas;
                    }
                }
                cx.notify();
            });
        })
        .detach();
        let subscription = cx.observe_global::<Workbench>(|_, cx| cx.notify());
        let mut controller = CanvasController::new();
        controller.state = super::default_start_layout();
        Self {
            focus: cx.focus_handle(),
            controller,
            root,
            bounds: Rc::new(Cell::new(Bounds::default())),
            drag: None,
            resize_drag: None,
            pan_drag: None,
            layout_revision: 0,
            status: String::new(),
            _services: subscription,
        }
    }
    fn save(&mut self, cx: &App) {
        self.layout_revision += 1;
        if let Some(s) = cx.try_global::<Workbench>() {
            s.persist(PersistChange::Canvas(self.controller.state.clone()));
        }
    }
    fn add(&mut self, kind: BuiltinWidgetKind, cx: &mut Context<Self>) {
        if self.controller.state.widgets.len() >= 200 {
            self.status = "最多 200 个组件".into();
            cx.notify();
            return;
        }
        let size = kind.min_size();
        let offset = self.controller.state.widgets.len() as f64 * 16.;
        self.controller.state.add_widget(Widget::new(
            kind.as_str(),
            WorldRect::new(30. + offset, 30. + offset, size.w, size.h),
        ));
        self.save(cx);
        cx.notify();
    }
    fn viewport_size(&self) -> WorldSize {
        let size = self.bounds.get().size;
        WorldSize {
            w: f32::from(size.width) as f64,
            h: f32::from(size.height) as f64,
        }
    }
    fn fit(&mut self, cx: &mut Context<Self>) {
        self.controller.fit_all(self.viewport_size(), [0., 0.]);
        self.save(cx);
        cx.notify();
    }
    fn zoom(&mut self, factor: f32, cx: &mut Context<Self>) {
        let size = self.viewport_size();
        self.controller.zoom_at(
            super::clamp_zoom(self.controller.state.zoom * factor),
            ScreenPoint {
                x: size.w / 2.,
                y: size.h / 2.,
            },
            [0., 0.],
        );
        self.save(cx);
        cx.notify();
    }
    fn resize(&mut self, id: uuid::Uuid, delta: WorldPoint) {
        if let Some(w) = self.controller.state.find_mut(id) {
            let min = BuiltinWidgetKind::from_str(&w.kind).map(|k| k.min_size());
            w.rect = super::resize_clamped(w.rect, delta, min);
        }
    }
    fn key_down(
        &mut self,
        event: &gpui_kit::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.focus.is_focused(window) {
            return;
        }
        let step = if event.keystroke.modifiers.shift {
            1.
        } else {
            10.
        };
        let key = event.keystroke.key.as_str();
        let delta = match key {
            "left" => Some((-step, 0.)),
            "right" => Some((step, 0.)),
            "up" => Some((0., -step)),
            "down" => Some((0., step)),
            _ => None,
        };
        if let Some((x, y)) = delta {
            if let Some(id) = self.controller.selected {
                if event.keystroke.modifiers.alt {
                    self.resize(id, WorldPoint { x, y });
                } else {
                    self.controller.keyboard_move(x, y);
                }
            } else {
                self.controller.pan_by([x, y]);
            }
            self.save(cx);
        } else if key == "tab" && !self.controller.state.widgets.is_empty() {
            let widgets = &self.controller.state.widgets;
            let index = widgets
                .iter()
                .position(|w| Some(w.id) == self.controller.selected);
            let next = if event.keystroke.modifiers.shift {
                index
                    .map(|i| (i + widgets.len() - 1) % widgets.len())
                    .unwrap_or(widgets.len() - 1)
            } else {
                index.map(|i| (i + 1) % widgets.len()).unwrap_or(0)
            };
            self.controller.selected = Some(widgets[next].id);
        } else if key == "delete" {
            if let Some(id) = self.controller.selected.take() {
                self.controller.state.remove_widget(id);
                self.save(cx);
            }
        } else if key == "escape" {
            self.controller.selected = None;
        } else if event.keystroke.modifiers.control && key == "f" {
            self.fit(cx);
        } else if event.keystroke.modifiers.control && (key == "+" || key == "=") {
            self.zoom(1.1, cx);
        } else if event.keystroke.modifiers.control && key == "-" {
            self.zoom(1. / 1.1, cx);
        } else {
            return;
        }
        cx.stop_propagation();
        cx.notify();
    }
}
impl BasePanel for HomePanel {
    fn panel_name(&self) -> &'static str {
        "HomePanel"
    }
}
impl Panel for HomePanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        "Homepage"
    }
}
impl EventEmitter<PanelEvent> for HomePanel {}
impl Focusable for HomePanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for HomePanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bounds = self.bounds.clone();
        let measure = gpui_kit::canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {})
            .absolute()
            .size_full();
        let zoom = self.controller.state.zoom;
        let pan = self.controller.state.pan;
        let mut widgets = self.controller.state.widgets.iter().collect::<Vec<_>>();
        widgets.sort_by_key(|w| w.z);
        let viewport = self.bounds.get();
        let service = cx.try_global::<Workbench>().cloned();
        let mut content = div()
            .id("home-canvas")
            .test_support()
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .overflow_hidden()
            .child(measure);
        for w in widgets {
            let Some(position) = super::world_to_screen(w.rect.origin, pan, zoom, [0., 0.]) else {
                continue;
            };
            let width = w.rect.size.w * zoom as f64;
            let height = w.rect.size.h * zoom as f64;
            if position.x + width < 0.
                || position.y + height < 0.
                || position.x > f32::from(viewport.size.width) as f64
                || position.y > f32::from(viewport.size.height) as f64
            {
                continue;
            }
            let id = w.id;
            let key = id.as_u128() as usize;
            let kind = BuiltinWidgetKind::from_str(&w.kind);
            let title = kind
                .map(|k| k.default_title())
                .unwrap_or("Unknown component");
            let mut card = div()
                .id(("widget", key))
                .test_support()
                .absolute()
                .left(px(position.x as f32))
                .top(px(position.y as f32))
                .w(px(width as f32))
                .h(px(height as f32))
                .border_1()
                .border_color(if self.controller.selected == Some(id) {
                    cx.theme().primary
                } else {
                    cx.theme().border
                })
                .bg(cx.theme().background)
                .flex()
                .flex_col()
                .overflow_hidden()
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .p_1()
                        .child(
                            div()
                                .id(("move-widget", key))
                                .test_support()
                                .flex_1()
                                .child(title)
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(
                                        move |this,
                                              event: &gpui_kit::MouseDownEvent,
                                              window,
                                              cx| {
                                            this.controller.selected = Some(id);
                                            this.controller.state.raise(id);
                                            this.drag = Some((id, event.position));
                                            window.focus(&this.focus, cx);
                                            cx.stop_propagation();
                                            cx.notify();
                                        },
                                    ),
                                ),
                        )
                        .child(Button::new(("remove-widget", key)).label("×").on_click(
                            cx.listener(move |this, _, _, cx| {
                                this.controller.state.remove_widget(id);
                                this.save(cx);
                                cx.notify();
                            }),
                        )),
                );
            match kind {
                Some(
                    BuiltinWidgetKind::Atlas
                    | BuiltinWidgetKind::Efforts
                    | BuiltinWidgetKind::Archive,
                ) => {
                    let path = self.root.join(title);
                    card = card.child(
                        Button::new(("entry", key))
                            .label(format!("打开 {title}"))
                            .disabled(service.is_none())
                            .on_click(move |_, _, cx| {
                                cx.global::<Workbench>()
                                    .emit(WorkbenchEvent::Open(path.clone()))
                            }),
                    );
                }
                Some(BuiltinWidgetKind::Tasks) => {
                    if let Some(s) = &service {
                        for (index, task) in s.tasks.borrow().iter().take(5).enumerate() {
                            let intent = task.intent.clone();
                            card = card.child(
                                Checkbox::new(("home-task", index))
                                    .label(task.title.clone())
                                    .checked(task.checked)
                                    .on_change(move |checked, _, cx| {
                                        let mut intent = intent.clone();
                                        intent.update = omw_document::TaskFieldUpdate::Complete {
                                            checked: *checked,
                                        };
                                        cx.global::<Workbench>().emit(WorkbenchEvent::Task(intent));
                                    }),
                            );
                        }
                    }
                    card = card.child(
                        Button::new(("tasks", key))
                            .label("查看和修改任务")
                            .disabled(service.is_none())
                            .on_click(|_, _, cx| {
                                cx.global::<Workbench>().emit(WorkbenchEvent::ShowTasks)
                            }),
                    );
                }
                Some(BuiltinWidgetKind::Pi) => {
                    if let Some(s) = &service {
                        card = card.child(s.pi_status.borrow().clone());
                    }
                    card = card.child(
                        Button::new(("pi", key))
                            .label("打开 Pi")
                            .disabled(service.is_none())
                            .on_click(|_, _, cx| {
                                cx.global::<Workbench>().emit(WorkbenchEvent::ShowPi)
                            }),
                    );
                }
                Some(BuiltinWidgetKind::Recent | BuiltinWidgetKind::Pinned) => {
                    let paths: Vec<PathBuf> = if kind == Some(BuiltinWidgetKind::Pinned) {
                        w.options
                            .get("paths")
                            .cloned()
                            .and_then(|p| serde_json::from_value(p).ok())
                            .unwrap_or_default()
                    } else {
                        service
                            .as_ref()
                            .map(|s| s.recent.borrow().clone())
                            .unwrap_or_default()
                    };
                    for (index, path) in paths.into_iter().take(5).enumerate() {
                        let label = path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned();
                        card = card.child(
                            Button::new(("page", index))
                                .label(label)
                                .disabled(service.is_none())
                                .on_click(move |_, _, cx| {
                                    cx.global::<Workbench>()
                                        .emit(WorkbenchEvent::Open(path.clone()))
                                }),
                        );
                    }
                    if kind == Some(BuiltinWidgetKind::Pinned) {
                        card = card.child(
                            Button::new(("pin-current", key))
                                .label("置顶当前页")
                                .disabled(service.is_none())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let path = cx.global::<Workbench>().active.borrow().clone();
                                    if let Some(path) = path {
                                        if let Some(w) = this.controller.state.find_mut(id) {
                                            let mut paths: Vec<PathBuf> = w
                                                .options
                                                .get("paths")
                                                .cloned()
                                                .and_then(|p| serde_json::from_value(p).ok())
                                                .unwrap_or_default();
                                            if !paths.contains(&path) {
                                                paths.push(path);
                                            }
                                            w.options = serde_json::json!({"paths": paths});
                                        }
                                        this.save(cx);
                                        cx.notify();
                                    }
                                })),
                        );
                    }
                }
                Some(BuiltinWidgetKind::Actions) => {
                    card = card.child(
                        Button::new(("fit", key))
                            .label("适合全部")
                            .on_click(cx.listener(|this, _, _, cx| this.fit(cx))),
                    );
                }
                None => {}
            }
            card = card.child(div().flex_1()).child(
                div().flex().justify_end().child(
                    div()
                        .id(("resize", key))
                        .test_support()
                        .w(px(24.))
                        .h(px(24.))
                        .child("↘")
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(
                                move |this, event: &gpui_kit::MouseDownEvent, window, cx| {
                                    this.controller.selected = Some(id);
                                    this.resize_drag = Some((id, event.position));
                                    window.focus(&this.focus, cx);
                                    cx.stop_propagation();
                                    cx.notify();
                                },
                            ),
                        ),
                ),
            );
            content = content.child(card);
        }
        content = content
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.controller.selected = None;
                    window.focus(&this.focus, cx);
                    cx.notify();
                }),
            )
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(|this, event: &gpui_kit::MouseDownEvent, _, _| {
                    this.pan_drag = Some(event.position)
                }),
            )
            .on_mouse_move(
                cx.listener(|this, event: &gpui_kit::MouseMoveEvent, _, cx| {
                    if let Some((id, last)) = this.resize_drag.or(this.drag) {
                        let from = ScreenPoint {
                            x: f32::from(last.x) as f64,
                            y: f32::from(last.y) as f64,
                        };
                        let to = ScreenPoint {
                            x: f32::from(event.position.x) as f64,
                            y: f32::from(event.position.y) as f64,
                        };
                        if let Some(delta) = super::world_delta(
                            from,
                            to,
                            this.controller.state.pan,
                            this.controller.state.zoom,
                            [0., 0.],
                        ) {
                            if this.resize_drag.is_some() {
                                this.resize(id, delta);
                                this.resize_drag = Some((id, event.position));
                            } else {
                                if let Some(w) = this.controller.state.find_mut(id) {
                                    w.rect = super::apply_world_delta(w.rect, delta);
                                }
                                this.drag = Some((id, event.position));
                            }
                            cx.notify();
                        }
                    } else if let Some(last) = this.pan_drag {
                        this.controller.pan_by([
                            f32::from(event.position.x - last.x) as f64,
                            f32::from(event.position.y - last.y) as f64,
                        ]);
                        this.pan_drag = Some(event.position);
                        cx.notify();
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    let moved = this.drag.take().is_some() | this.resize_drag.take().is_some();
                    if moved {
                        this.save(cx);
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Middle,
                cx.listener(|this, _, _, cx| {
                    if this.pan_drag.take().is_some() {
                        this.save(cx);
                    }
                }),
            )
            .on_scroll_wheel(
                cx.listener(|this, event: &gpui_kit::ScrollWheelEvent, _, cx| {
                    let delta = event.delta.pixel_delta(px(16.));
                    if event.modifiers.control {
                        let origin = this.bounds.get().origin;
                        let point = ScreenPoint {
                            x: f32::from(event.position.x - origin.x) as f64,
                            y: f32::from(event.position.y - origin.y) as f64,
                        };
                        let factor = if f32::from(delta.y) > 0. {
                            1.1
                        } else {
                            1. / 1.1
                        };
                        this.controller.zoom_at(
                            super::clamp_zoom(this.controller.state.zoom * factor),
                            point,
                            [0., 0.],
                        );
                    } else {
                        this.controller
                            .pan_by([f32::from(delta.x) as f64, f32::from(delta.y) as f64]);
                    }
                    this.save(cx);
                    cx.notify();
                }),
            );
        let mut toolbar = div()
            .flex()
            .flex_wrap()
            .gap_1()
            .child(
                Button::new("home-fit")
                    .label("适合全部")
                    .on_click(cx.listener(|this, _, _, cx| this.fit(cx))),
            )
            .child(
                Button::new("home-zoom-out")
                    .label("−")
                    .on_click(cx.listener(|this, _, _, cx| this.zoom(1. / 1.1, cx))),
            )
            .child(
                Button::new("home-zoom-in")
                    .label("+")
                    .on_click(cx.listener(|this, _, _, cx| this.zoom(1.1, cx))),
            );
        for kind in [
            BuiltinWidgetKind::Atlas,
            BuiltinWidgetKind::Efforts,
            BuiltinWidgetKind::Archive,
            BuiltinWidgetKind::Recent,
            BuiltinWidgetKind::Pinned,
            BuiltinWidgetKind::Tasks,
            BuiltinWidgetKind::Pi,
            BuiltinWidgetKind::Actions,
        ] {
            toolbar = toolbar.child(
                Button::new(format!("add-{}", kind.as_str()))
                    .label(format!("+{}", kind.default_title()))
                    .on_click(cx.listener(move |this, _, _, cx| this.add(kind, cx))),
            );
        }
        div()
            .id("home-pane")
            .test_support()
            .size_full()
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .child(toolbar)
            .child(content)
            .child("方向键移动；Shift 精细移动；Alt+方向键调整尺寸；Ctrl +/- 缩放；Ctrl+F 适合全部")
            .child(self.status.clone())
            .on_key_down(cx.listener(Self::key_down))
    }
}
