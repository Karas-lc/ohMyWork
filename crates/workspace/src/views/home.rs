
use gpui_kit::{App, Bounds, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, Pixels, Render, Subscription, Window, div, prelude::*, px};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::{ActiveTheme as _, button::Button, dock::{BasePanel, Panel, PanelEvent}};

use crate::{homepage::{HomeCommand, Homepage}, home::{BuiltinWidgetKind, ScreenPoint, WorldPoint, WorldSize, world_to_screen}, queries::TasksModel, session::WorkspaceSession};

use std::{cell::Cell, path::{Path, PathBuf}, rc::Rc};
type PointerDrag = (uuid::Uuid, gpui_kit::Point<Pixels>);

pub struct HomePanel {
    focus: FocusHandle,
    homepage: Entity<Homepage>,
    tasks: Entity<TasksModel>,
    session: Entity<WorkspaceSession>,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    selected: Option<uuid::Uuid>,
    resize_drag: Option<PointerDrag>,
    drag: Option<PointerDrag>,
    pan_drag: Option<gpui_kit::Point<Pixels>>,
    _subscriptions: Vec<Subscription>,
}

impl HomePanel {
    pub fn new(homepage: Entity<Homepage>, tasks: Entity<TasksModel>, session: Entity<WorkspaceSession>, _: &mut Window, cx: &mut Context<Self>) -> Self {
        let subs = vec![cx.observe(&homepage, |_, _, cx| cx.notify()), cx.observe(&tasks, |_, _, cx| cx.notify()), cx.observe(&session, |_, _, cx| cx.notify())];
        Self { focus: cx.focus_handle(), homepage, tasks, session, bounds: Rc::new(Cell::new(Bounds::default())), selected: None, drag: None, resize_drag: None, pan_drag: None, _subscriptions: subs }
    }

    fn command(&mut self, command: HomeCommand, cx: &mut Context<Self>) { self.homepage.update(cx, |home, cx| home.apply(command, cx)); }

    fn viewport_size(&self) -> WorldSize {
        let size = self.bounds.get().size;
        WorldSize { w: f32::from(size.width) as f64, h: f32::from(size.height) as f64 }
    }

    fn fit(&mut self, cx: &mut Context<Self>) { self.command(HomeCommand::Fit { viewport: self.viewport_size() }, cx); }

    fn key_down(&mut self, event: &gpui_kit::KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.is_focused(window) { return; }
        let step = if event.keystroke.modifiers.shift { 1.0 } else { 10.0 };
        let delta = match event.keystroke.key.as_str() { "left" => Some((-step, 0.0)), "right" => Some((step, 0.0)), "up" => Some((0.0, -step)), "down" => Some((0.0, step)), _ => None };
        if let Some((x, y)) = delta {
            if let Some(id) = self.selected { self.command(HomeCommand::Move { id, delta: WorldPoint { x, y } }, cx); }
            else { self.command(HomeCommand::Pan([x, y]), cx); }
        } else if event.keystroke.key.as_str() == "delete" {
            if let Some(id) = self.selected.take() { self.command(HomeCommand::Remove(id), cx); }
        } else if event.keystroke.key.as_str() == "escape" { self.selected = None; }
        else if event.keystroke.modifiers.control && event.keystroke.key.as_str() == "f" { self.fit(cx); }
        else { return; }
        cx.stop_propagation();
        cx.notify();
    }
}

impl BasePanel for HomePanel {
    fn panel_name(&self) -> &'static str { "HomePanel" }
    fn closable(&self, _: &App) -> bool { false }
}
impl Panel for HomePanel { fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement { "Homepage" } }
impl EventEmitter<PanelEvent> for HomePanel {}
impl Focusable for HomePanel { fn focus_handle(&self, _: &App) -> FocusHandle { self.focus.clone() } }

impl Render for HomePanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bounds_cell = self.bounds.clone();
        let measure = gpui_kit::canvas(move |bounds, _, _| bounds_cell.set(bounds), |_, _, _, _| {}).absolute().size_full();
        let state = self.homepage.read(cx).state().clone();
        let zoom = state.zoom;
        let pan = state.pan;
        let mut widgets = state.widgets.iter().collect::<Vec<_>>();
        widgets.sort_by_key(|widget| widget.z);
        let viewport = self.bounds.get();
        let mut canvas = div().id("home-canvas").relative().flex_1().min_h_0().w_full().overflow_hidden().child(measure);
        for widget in widgets {
            let Some(point) = world_to_screen(widget.rect.origin, pan, zoom, [0.0, 0.0]) else { continue };
            let id = widget.id;
            let key = id.as_u128() as usize;
            let kind = BuiltinWidgetKind::from_str(&widget.kind);
            let title = kind.map(|kind| kind.default_title()).unwrap_or("Unknown component");
            let width = widget.rect.size.w * zoom as f64;
            let height = widget.rect.size.h * zoom as f64;
            if point.x + width < 0.0 || point.y + height < 0.0 || point.x > f32::from(viewport.size.width) as f64 || point.y > f32::from(viewport.size.height) as f64 { continue; }
            let mut card = div().id(("widget", key)).absolute().left(px(point.x as f32)).top(px(point.y as f32)).w(px(width as f32)).h(px(height as f32)).border_1().bg(cx.theme().background).flex().flex_col().overflow_hidden()
                .child(div().flex().justify_between().p_1().child(div().id(("move-widget", key)).flex_1().child(title).on_mouse_down(gpui_kit::MouseButton::Left, cx.listener(move |this, event: &gpui_kit::MouseDownEvent, window, cx| {
                    this.selected = Some(id); this.homepage.update(cx, |home, _cx| home.begin_gesture()); this.homepage.update(cx, |home, cx| home.apply(HomeCommand::Raise(id), cx)); this.drag = Some((id, event.position)); window.focus(&this.focus, cx); cx.stop_propagation();
                }))).child(Button::new(("remove-widget", key)).label("×").on_click(cx.listener(move |this, _, _, cx| { this.selected = Some(id); this.command(HomeCommand::Remove(id), cx); }))));
            match kind {
                Some(BuiltinWidgetKind::Tasks) => {
                    let summaries = self.tasks.read(cx).summaries().iter().take(5).cloned().collect::<Vec<_>>();
                    for (index, summary) in summaries.into_iter().enumerate() {
                        let intent = summary.intent;
                        card = card.child(gpui_kit::component::checkbox::Checkbox::new(("home-task", index)).label(summary.title).checked(summary.checked).on_change(cx.listener(move |this, checked, _, cx| {
                            let mut intent = intent.clone();
                            intent.update = editor::TaskFieldUpdate::Complete { checked: *checked };
                            if let Err(error) = this.tasks.update(cx, |tasks, cx| tasks.commit_task(intent, cx)) {
                                this.homepage.update(cx, |home, cx| { home.set_status(error.to_string()); cx.notify(); });
                            }
                            cx.notify();
                        })));
                    }
                    card = card.child(Button::new(("tasks", key)).label("查看任务").on_click(cx.listener(|this, _, _, cx| { this.session.update(cx, |session, cx| session.show_tasks(cx)); })));
                }
                Some(BuiltinWidgetKind::Pi) => card = card.child(Button::new(("pi", key)).label("打开 Pi").on_click(cx.listener(|this, _, _, cx| { this.session.update(cx, |session, cx| session.show_pi(cx)); }))),
                Some(BuiltinWidgetKind::Recent) => {
                    let recent = self.session.read(cx).recent().to_vec();
                    for (index, path) in recent.into_iter().take(5).enumerate() {
                        let label = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                        card = card.child(Button::new(("recent", index)).label(label).on_click(cx.listener(move |this, _, _, cx| { this.session.update(cx, |session, cx| session.request_open(path.clone(), cx)); })));
                    }
                }
                Some(BuiltinWidgetKind::Pinned) => {
                    let paths: Vec<PathBuf> = widget.options.get("paths").cloned().and_then(|value| serde_json::from_value(value).ok()).unwrap_or_default();
                    for (index, path) in paths.into_iter().take(5).enumerate() {
                        let label = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                        card = card.child(Button::new(("pinned", index)).label(label).on_click(cx.listener(move |this, _, _, cx| { this.session.update(cx, |session, cx| session.request_open(path.clone(), cx)); })));
                    }
                    card = card.child(Button::new(("pin-current", key)).label("置顶当前页").on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(path) = this.session.read(cx).active().map(Path::to_path_buf) { this.command(HomeCommand::Pin { widget: id, path }, cx); }
                    })));
                }
                Some(BuiltinWidgetKind::Atlas | BuiltinWidgetKind::Efforts | BuiltinWidgetKind::Archive) => {
                    let path = self.session.read(cx).root().map(|root| root.join(title));
                    if let Some(path) = path { card = card.child(Button::new(("entry", key)).label(format!("打开 {title}")).on_click(cx.listener(move |this, _, _, cx| { this.session.update(cx, |session, cx| session.request_open(path.clone(), cx)); }))); }
                }
                Some(BuiltinWidgetKind::Actions) => card = card.child(Button::new(("fit", key)).label("适合全部").on_click(cx.listener(|this, _, _, cx| this.fit(cx)))),
                None => {}
            }
            card = card.child(
                div()
                    .id(("resize-widget", key))
                    .w_full()
                    .h(px(12.))
                    .flex()
                    .justify_end()
                    .child("↘")
                    .on_mouse_down(gpui_kit::MouseButton::Left, cx.listener(move |this, event: &gpui_kit::MouseDownEvent, _, cx| {
                        this.selected = Some(id);
                        this.homepage.update(cx, |home, _| home.begin_gesture());
                        this.resize_drag = Some((id, event.position));
                        cx.stop_propagation();
                    })),
            );
            canvas = canvas.child(card);
        }
        let mut toolbar = div().flex().flex_wrap().gap_1().child(Button::new("home-fit").label("适合全部").on_click(cx.listener(|this, _, _, cx| this.fit(cx))));
        for kind in [BuiltinWidgetKind::Atlas, BuiltinWidgetKind::Efforts, BuiltinWidgetKind::Archive, BuiltinWidgetKind::Recent, BuiltinWidgetKind::Pinned, BuiltinWidgetKind::Tasks, BuiltinWidgetKind::Pi, BuiltinWidgetKind::Actions] {
            toolbar = toolbar.child(Button::new(format!("add-{}", kind.as_str())).label(format!("+{}", kind.default_title())).on_click(cx.listener(move |this, _, _, cx| this.command(HomeCommand::Add(kind), cx))));
        }
        canvas = canvas.on_mouse_down(gpui_kit::MouseButton::Left, cx.listener(|this, event: &gpui_kit::MouseDownEvent, _, cx| {
            let origin = this.bounds.get().origin;
            let point = ScreenPoint {
                x: f32::from(event.position.x - origin.x) as f64,
                y: f32::from(event.position.y - origin.y) as f64,
            };
            let state = this.homepage.read(cx).state();
            this.selected = crate::home::hit_test(&state.widgets, point, state.pan, state.zoom, [0.0, 0.0]);
            if let Some(id) = this.selected {
                this.homepage.update(cx, |home, cx| home.apply(HomeCommand::Raise(id), cx));
            }
            cx.notify();
        }));
        canvas = canvas.on_mouse_down(gpui_kit::MouseButton::Middle, cx.listener(|this, event: &gpui_kit::MouseDownEvent, _, cx| {
            this.pan_drag = Some(event.position);
            this.homepage.update(cx, |home, _| home.begin_gesture());
        })).on_mouse_up(gpui_kit::MouseButton::Left, cx.listener(|this, _, _, cx| {
            let mut ended = this.drag.take().is_some();
            ended |= this.resize_drag.take().is_some();
            if ended { this.homepage.update(cx, |home, cx| home.end_gesture(cx)); }
        })).on_mouse_up(gpui_kit::MouseButton::Middle, cx.listener(|this, _, _, cx| {
            if this.pan_drag.take().is_some() { this.homepage.update(cx, |home, cx| home.end_gesture(cx)); }
        })).on_mouse_move(cx.listener(|this, event: &gpui_kit::MouseMoveEvent, _, cx| {
            if let Some((id, last)) = this.resize_drag {
                let from = ScreenPoint { x: f32::from(last.x) as f64, y: f32::from(last.y) as f64 };
                let to = ScreenPoint { x: f32::from(event.position.x) as f64, y: f32::from(event.position.y) as f64 };
                let snapshot = this.homepage.read(cx).state().clone();
                if let Some(delta) = crate::home::world_delta(from, to, snapshot.pan, snapshot.zoom, [0.0, 0.0]) {
                    this.command(HomeCommand::Resize { id, delta }, cx);
                    this.resize_drag = Some((id, event.position));
                }
            } else if let Some((id, last)) = this.drag {
                let from = ScreenPoint { x: f32::from(last.x) as f64, y: f32::from(last.y) as f64 };
                let to = ScreenPoint { x: f32::from(event.position.x) as f64, y: f32::from(event.position.y) as f64 };
                let snapshot = this.homepage.read(cx).state().clone();
                if let Some(delta) = crate::home::world_delta(from, to, snapshot.pan, snapshot.zoom, [0.0, 0.0]) {
                    this.command(HomeCommand::Move { id, delta }, cx);
                    this.drag = Some((id, event.position));
                }
            } else if let Some(last) = this.pan_drag {
                this.command(HomeCommand::Pan([f32::from(event.position.x - last.x) as f64, f32::from(event.position.y - last.y) as f64]), cx);
                this.pan_drag = Some(event.position);
            }
        })).on_scroll_wheel(cx.listener(|this, event: &gpui_kit::ScrollWheelEvent, _, cx| {
            let delta = event.delta.pixel_delta(px(16.0));
            if event.modifiers.control {
                let origin = this.bounds.get().origin;
                let anchor = ScreenPoint { x: f32::from(event.position.x - origin.x) as f64, y: f32::from(event.position.y - origin.y) as f64 };
                let factor = if f32::from(delta.y) > 0.0 { 1.1 } else { 1.0 / 1.1 };
                this.command(HomeCommand::Zoom { factor, anchor }, cx);
            } else { this.command(HomeCommand::Pan([f32::from(delta.x) as f64, f32::from(delta.y) as f64]), cx); }
        }));
        let status = self.homepage.read(cx).status().unwrap_or_default().to_owned();
        div().id("home-pane").test_support().size_full().track_focus(&self.focus).flex().flex_col().child(toolbar).child(canvas).child("方向键移动；Shift 精细移动；Ctrl +/- 缩放；Ctrl+F 适合全部").child(status).on_key_down(cx.listener(Self::key_down))
    }
}

