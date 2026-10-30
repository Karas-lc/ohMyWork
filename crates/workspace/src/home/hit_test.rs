//! Plan 19: hit test + manipulation (z reverse, world deltas, resize clamp).
//! No UI here; returns indices/ids/deltas for controller + Element to apply to model.
//! Linear scan ok for V1 100 widgets.

use crate::home::model::Widget;
use crate::home::transform::{screen_to_world, ScreenPoint, WorldPoint, WorldRect, WorldSize};
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

