use std::path::PathBuf;

use gpui_kit::{AppContext as _, Context};
use project::AppDataPaths;

use crate::home::{BuiltinWidgetKind, CanvasState, WorldPoint, Widget, WorldRect};
use crate::persistence::UiStateWriter;

#[derive(Clone, Debug)]
pub enum HomeCommand {
    Add(BuiltinWidgetKind),
    Remove(uuid::Uuid),
    Raise(uuid::Uuid),
    Move { id: uuid::Uuid, delta: WorldPoint },
    Resize { id: uuid::Uuid, delta: WorldPoint },
    Pan([f64; 2]),
    Zoom { factor: f32, anchor: crate::home::ScreenPoint },
    Fit { viewport: crate::home::WorldSize },
    Pin { widget: uuid::Uuid, path: PathBuf },
}

pub struct Homepage {
    state: CanvasState,
    writer: UiStateWriter,
    revision: u64,
    gesture: bool,
    status: Option<String>,
}

impl Homepage {
    pub fn new(paths: AppDataPaths, root: PathBuf, writer: UiStateWriter, cx: &mut Context<Self>) -> Self {
        let model = Self { state: crate::home::default_start_layout(), writer, revision: 0, gesture: false, status: None };
        let task = cx.background_spawn(async move { crate::persistence::load_ui_state(&paths, &root) });
        cx.spawn(async move |this, cx| {
            let loaded = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.revision == 0 {
                    match loaded {
                        Ok(snapshot) => if let Some(state) = snapshot.canvas { this.state = state; },
                        Err(error) => this.status = Some(format!("无法读取首页布局：{error}")),
                    }
                }
                cx.notify();
            });
        }).detach();
        model
    }

    pub fn state(&self) -> &CanvasState { &self.state }
    pub fn set_status(&mut self, status: String) { self.status = Some(status); }
    pub fn status(&self) -> Option<&str> { self.status.as_deref() }

    pub fn apply(&mut self, command: HomeCommand, cx: &mut Context<Self>) {
        let target = match &command {
            HomeCommand::Remove(id) | HomeCommand::Raise(id) => Some(*id),
            HomeCommand::Move { id, .. } | HomeCommand::Resize { id, .. } => Some(*id),
            HomeCommand::Pin { widget, .. } => Some(*widget),
            _ => None,
        };
        if target.is_some_and(|id| self.state.find(id).is_none()) { return; }
        match command {
            HomeCommand::Add(kind) => {
                if self.state.widgets.len() >= 200 { return; }
                let size = kind.min_size();
                let offset = self.state.widgets.len() as f64 * 16.0;
                self.state.add_widget(Widget::new(kind.as_str(), WorldRect::new(30.0 + offset, 30.0 + offset, size.w, size.h)));
            }
            HomeCommand::Remove(id) => self.state.remove_widget(id),
            HomeCommand::Raise(id) => self.state.raise(id),
            HomeCommand::Move { id, delta } => if let Some(widget) = self.state.find_mut(id) { widget.rect = crate::home::apply_world_delta(widget.rect, delta); },
            HomeCommand::Resize { id, delta } => if let Some(widget) = self.state.find_mut(id) {
                let minimum = BuiltinWidgetKind::from_str(&widget.kind).map(|kind| kind.min_size());
                widget.rect = crate::home::resize_clamped(widget.rect, delta, minimum);
            },
            HomeCommand::Pan(delta) => self.state.pan = crate::home::apply_pan(self.state.pan, delta),
            HomeCommand::Zoom { factor, anchor } => {
                let target = crate::home::clamp_zoom(self.state.zoom * factor);
                if let Some(pan) = crate::home::zoom_anchored(self.state.pan, self.state.zoom, target, anchor, [0.0, 0.0]) { self.state.pan = pan; }
                self.state.zoom = target;
            },
            HomeCommand::Fit { viewport } => fit_all(&mut self.state, viewport),
            HomeCommand::Pin { widget, path } => if let Some(item) = self.state.find_mut(widget) {
                let mut paths: Vec<PathBuf> = item.options.get("paths").cloned().and_then(|value| serde_json::from_value(value).ok()).unwrap_or_default();
                if !paths.contains(&path) { paths.push(path); }
                item.options["paths"] = serde_json::json!(paths);
            },
        }
        self.revision = self.revision.wrapping_add(1);
        if !self.gesture { let _ = self.checkpoint(cx); }
        cx.notify();
    }

    pub fn begin_gesture(&mut self) { self.gesture = true; }
    pub fn end_gesture(&mut self, cx: &mut Context<Self>) { if self.gesture { self.gesture = false; let _ = self.checkpoint(cx); } }
    pub fn checkpoint(&mut self, cx: &mut Context<Self>) -> Result<(), String> {
        let result = self.writer.write_canvas(self.state.clone());
        self.status = result.as_ref().err().map(|error| format!("保存首页布局失败：{error}"));
        cx.notify();
        result
    }
}

fn fit_all(state: &mut CanvasState, viewport: crate::home::WorldSize) {
    if state.widgets.is_empty() {
        state.pan = [0.0, 0.0];
        state.zoom = 1.0;
        return;
    }
    let mut min_x = f64::MAX;
    let mut min_y = f64::MAX;
    let mut max_x = f64::MIN;
    let mut max_y = f64::MIN;
    for widget in &state.widgets {
        min_x = min_x.min(widget.rect.origin.x);
        min_y = min_y.min(widget.rect.origin.y);
        max_x = max_x.max(widget.rect.origin.x + widget.rect.size.w);
        max_y = max_y.max(widget.rect.origin.y + widget.rect.size.h);
    }
    let content_width = (max_x - min_x).max(1.0);
    let content_height = (max_y - min_y).max(1.0);
    state.zoom = crate::home::clamp_zoom(
        (viewport.w / content_width)
            .min(viewport.h / content_height)
            .min(2.0)
            .max(0.25) as f32,
    );
    let zoom = state.zoom as f64;
    state.pan = [
        (viewport.w - content_width * zoom) * 0.5 - min_x * zoom,
        (viewport.h - content_height * zoom) * 0.5 - min_y * zoom,
    ];
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_all_centers_bounds_within_clamped_zoom() {
        let mut state = CanvasState::new();
        state.widgets.push(Widget::new("tasks", WorldRect::new(-100.0, -50.0, 100.0, 100.0)));
        state.widgets.push(Widget::new("tasks", WorldRect::new(100.0, 100.0, 100.0, 100.0)));
        fit_all(&mut state, crate::home::WorldSize { w: 600.0, h: 500.0 });
        assert_eq!(state.zoom, 2.0);
        assert_eq!(state.pan, [200.0, 100.0]);
    }
}
