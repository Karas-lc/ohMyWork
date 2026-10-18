//! Plan 19: hit test + manipulation (z reverse, world deltas, resize clamp).
//! No UI here; returns indices/ids/deltas for controller + Element to apply to model.
//! Linear scan ok for V1 100 widgets.

use crate::home::model::{CanvasState, Widget};
use crate::home::transform::{
    apply_pan, clamp_zoom, screen_to_world, zoom_anchored, ScreenPoint, WorldPoint, WorldRect, WorldSize,
};
/// Hit test: convert screen to world, scan widgets high-z to low, return first (top) id whose rect contains point.
/// Returns None on empty or no hit or non-finite.
pub fn hit_test(
    widgets: &[Widget],
    screen_pos: ScreenPoint,
    pan: [f64; 2],
    zoom: f32,
    viewport_origin: [f64; 2],
) -> Option<uuid::Uuid> {
    if widgets.is_empty() {
        return None;
    }
    let world = screen_to_world(screen_pos, pan, zoom, viewport_origin)?;
    // Sort indices by z descending (high on top)
    let mut order: Vec<usize> = (0..widgets.len()).collect();
    order.sort_by(|&a, &b| widgets[b].z.cmp(&widgets[a].z));

    for &i in &order {
        let w = &widgets[i];
        if point_in_world_rect(world, &w.rect) {
            return Some(w.id);
        }
    }
    None
}

fn point_in_world_rect(p: WorldPoint, r: &WorldRect) -> bool {
    p.x >= r.origin.x
        && p.x <= r.origin.x + r.size.w
        && p.y >= r.origin.y
        && p.y <= r.origin.y + r.size.h
}

/// Compute world delta from two screen points (for drag).
pub fn world_delta(
    screen_from: ScreenPoint,
    screen_to: ScreenPoint,
    pan: [f64; 2],
    zoom: f32,
    viewport_origin: [f64; 2],
) -> Option<WorldPoint> {
    let w_from = screen_to_world(screen_from, pan, zoom, viewport_origin)?;
    let w_to = screen_to_world(screen_to, pan, zoom, viewport_origin)?;
    Some(WorldPoint {
        x: w_to.x - w_from.x,
        y: w_to.y - w_from.y,
    })
}

/// Apply world delta to a rect (for drag). Returns new rect (no clamp here).
pub fn apply_world_delta(rect: WorldRect, delta: WorldPoint) -> WorldRect {
    WorldRect {
        origin: WorldPoint {
            x: rect.origin.x + delta.x,
            y: rect.origin.y + delta.y,
        },
        size: rect.size,
    }
}

/// Resize from a corner (anchor other). Delta in world. Clamp to MIN_SIZE.
/// Simple: resize right/bottom for now (caller decides handle).
pub fn resize_clamped(rect: WorldRect, delta: WorldPoint, min: Option<WorldSize>) -> WorldRect {
    let min = min.unwrap_or(WorldRect::MIN_SIZE);
    let mut new_w = (rect.size.w + delta.x).max(min.w);
    let mut new_h = (rect.size.h + delta.y).max(min.h);
    if !new_w.is_finite() || new_w <= 0.0 {
        new_w = min.w;
    }
    if !new_h.is_finite() || new_h <= 0.0 {
        new_h = min.h;
    }
    WorldRect {
        origin: rect.origin,
        size: WorldSize { w: new_w, h: new_h },
    }
}

/// Bring widget to top z (lift).
pub fn lift_to_top(widgets: &mut [Widget], id: uuid::Uuid) {
    if let Some(max_z) = widgets.iter().map(|w| w.z).max() {
        if let Some(w) = widgets.iter_mut().find(|w| w.id == id) {
            w.z = max_z + 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::home::model::Widget;
    use crate::home::transform::WorldPoint;

    fn mk_widget(z: i32, x: f64, y: f64, w: f64, h: f64) -> Widget {
        let mut wd = Widget::new("test", WorldRect::new(x, y, w, h));
        wd.z = z;
        wd
    }

    #[test]
    fn z_reverse_hit_topmost() {
        let ws = vec![
            mk_widget(1, 0.0, 0.0, 200.0, 200.0),
            mk_widget(10, 50.0, 50.0, 100.0, 100.0), // on top
        ];
        let sp = ScreenPoint { x: 80.0, y: 80.0 };
        let hit = hit_test(&ws, sp, [0.0,0.0], 1.0, [0.0,0.0]);
        assert_eq!(hit, Some(ws[1].id));
    }

    #[test]
    fn world_delta_drag() {
        let from = ScreenPoint { x: 100.0, y: 100.0 };
        let to = ScreenPoint { x: 130.0, y: 140.0 };
        let d = world_delta(from, to, [0.0,0.0], 1.0, [0.0,0.0]).unwrap();
        assert!((d.x - 30.0).abs() < 1e-9);
        assert!((d.y - 40.0).abs() < 1e-9);
    }

    #[test]
    fn resize_clamps_min() {
        let r = WorldRect::new(0.0, 0.0, 200.0, 150.0);
        let small = resize_clamped(r, WorldPoint { x: -100.0, y: -100.0 }, None);
        assert_eq!(small.size.w, WorldRect::MIN_SIZE.w);
        assert_eq!(small.size.h, WorldRect::MIN_SIZE.h);
    }

    #[test]
    fn overlap_selects_top() {
        let ws = vec![mk_widget(5, 0.,0.,300.,300.), mk_widget(20, 10.,10.,50.,50.)];
        let hit = hit_test(&ws, ScreenPoint{x:15.,y:15.}, [0.;2],1.,[0.;2]);
        assert_eq!(hit, Some(ws[1].id));
    }
}

/// Interaction controller (pure logic; owns model for plan19 skeleton).
/// Mouse: down=select+lift, drag=move selected with world delta.
/// Zoom/pan/keyboard provided as pure fns callers invoke on event.
/// No specific widget kinds rendered here.
#[derive(Clone, Debug, Default)]
pub struct CanvasController {
    pub state: CanvasState,
    pub selected: Option<uuid::Uuid>,
    last_screen: Option<ScreenPoint>,
}

impl CanvasController {
    pub fn new() -> Self {
        Self {
            state: CanvasState::new(),
            selected: None,
            last_screen: None,
        }
    }

    /// Mouse down: hit test, select top, lift z.
    pub fn on_mouse_down(&mut self, screen: ScreenPoint, pan: [f64; 2], zoom: f32, vp: [f64; 2]) {
        if let Some(id) = hit_test(&self.state.widgets, screen, pan, zoom, vp) {
            self.selected = Some(id);
            lift_to_top(&mut self.state.widgets, id);
        } else {
            self.selected = None;
        }
        self.last_screen = Some(screen);
    }

    /// Drag: compute world delta from last, apply to selected rect.
    pub fn on_drag(&mut self, current_screen: ScreenPoint, pan: [f64; 2], zoom: f32, vp: [f64; 2]) {
        if let (Some(id), Some(last)) = (self.selected, self.last_screen) {
            if let Some(delta) = world_delta(last, current_screen, pan, zoom, vp) {
                if let Some(w) = self.state.find_mut(id) {
                    w.rect = apply_world_delta(w.rect, delta);
                }
            }
        }
        self.last_screen = Some(current_screen);
    }

    /// End drag.
    pub fn on_drag_end(&mut self) {
        self.last_screen = None;
    }

    /// Zoom at anchor (mouse screen pos).
    pub fn zoom_at(&mut self, target_zoom: f32, mouse: ScreenPoint, vp: [f64; 2]) {
        if let Some(new_pan) = zoom_anchored(self.state.pan, self.state.zoom, target_zoom, mouse, vp) {
            self.state.pan = new_pan;
            self.state.zoom = clamp_zoom(target_zoom);
        }
    }

    /// Pan by screen delta.
    pub fn pan_by(&mut self, delta_screen: [f64; 2]) {
        self.state.pan = apply_pan(self.state.pan, delta_screen);
    }

    /// Keyboard move selected (world units, default 10 / fine 1 per plan).
    pub fn keyboard_move(&mut self, dx: f64, dy: f64) {
        if let Some(id) = self.selected {
            if let Some(w) = self.state.find_mut(id) {
                w.rect.origin.x += dx;
                w.rect.origin.y += dy;
            }
        }
    }

    /// Fit all visible: simple bounds, center + scale.
    pub fn fit_all(&mut self, vp_size: WorldSize, vp: [f64; 2]) {
        if self.state.widgets.is_empty() {
            self.state.pan = [0.0, 0.0];
            self.state.zoom = 1.0;
            return;
        }
        let mut minx = f64::MAX; let mut miny = f64::MAX;
        let mut maxx = f64::MIN; let mut maxy = f64::MIN;
        for w in &self.state.widgets {
            minx = minx.min(w.rect.origin.x);
            miny = miny.min(w.rect.origin.y);
            maxx = maxx.max(w.rect.origin.x + w.rect.size.w);
            maxy = maxy.max(w.rect.origin.y + w.rect.size.h);
        }
        if minx == f64::MAX { return; }
        let content_w = (maxx - minx).max(1.0);
        let content_h = (maxy - miny).max(1.0);
        let z = (vp_size.w / content_w).min(vp_size.h / content_h).min(2.0).max(0.25) as f32;
        self.state.zoom = clamp_zoom(z);
        let zf = self.state.zoom as f64;
        self.state.pan = [
            vp[0] + (vp_size.w - content_w * zf) * 0.5 - minx * zf,
            vp[1] + (vp_size.h - content_h * zf) * 0.5 - miny * zf,
        ];
    }

    /// Reset view.
    pub fn reset_view(&mut self) {
        self.state.pan = [0.0, 0.0];
        self.state.zoom = 1.0;
    }
}

#[cfg(test)]
mod controller_tests {
    use super::*;

    #[test]
    fn select_drag_lift() {
        let mut c = CanvasController::new();
        let r = WorldRect::new(0.0,0.0,200.,200.);
        let w = Widget::new("a", r);
        let id = w.id;
        c.state.add_widget(w);
        c.on_mouse_down(ScreenPoint{x:50.,y:50.}, [0.;2],1.,[0.;2]);
        assert_eq!(c.selected, Some(id));
        c.on_drag(ScreenPoint{x:60.,y:60.}, [0.;2],1.,[0.;2]);
        let moved = c.state.find(id).unwrap();
        assert!((moved.rect.origin.x - 10.0).abs() < 1.0);
    }

    #[test]
    fn keyboard_and_fit() {
        let mut c = CanvasController::new();
        let r = WorldRect::new(0.,0.,200.,100.);
        c.state.add_widget(Widget::new("k", r));
        c.selected = Some(c.state.widgets[0].id);
        c.keyboard_move(10.0, 5.0);
        assert!((c.state.widgets[0].rect.origin.x - 10.0).abs() < 0.1);
        c.fit_all(WorldSize{w:800.,h:600.}, [0.;2]);
        assert!(c.state.zoom <= 2.0 && c.state.zoom >= 0.25);
    }
}
