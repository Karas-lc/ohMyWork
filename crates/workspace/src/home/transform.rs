//! Plan 19: pure coordinate transform for infinite canvas.
//! Single source of truth for world <-> screen; no duplication in widgets.
//! Formula (per plan): screen = viewport_origin + pan + world * zoom
//! zoom clamped 0.25..=2.0; reject NaN/Infinity/overflow sizes.

use std::f64;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WorldPoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WorldSize {
    pub w: f64,
    pub h: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WorldRect {
    pub origin: WorldPoint,
    pub size: WorldSize,
}

impl WorldRect {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self {
            origin: WorldPoint { x, y },
            size: WorldSize { w, h },
        }
    }

    /// Minimum size per plan: 160x100 logical by default (overridable by widget kind in model).
    pub const MIN_SIZE: WorldSize = WorldSize { w: 160.0, h: 100.0 };
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ScreenPoint {
    pub x: f64,
    pub y: f64,
}
/// Clamp zoom to approved range. Never returns non-finite.
pub fn clamp_zoom(z: f32) -> f32 {
    z.clamp(0.25, 2.0)
}

/// Validate finite non-NaN for coordinates/sizes. Returns false on NaN/inf.
fn is_finite(v: f64) -> bool {
    v.is_finite()
}


fn is_finite_point(p: WorldPoint) -> bool {
    is_finite(p.x) && is_finite(p.y)
}

/// world -> screen. Returns None on non-finite input (rejects NaN/inf/overflow).
pub fn world_to_screen(
    world: WorldPoint,
    pan: [f64; 2],
    zoom: f32,
    viewport_origin: [f64; 2],
) -> Option<ScreenPoint> {
    if !is_finite_point(world) || !pan.iter().all(|&v| is_finite(v)) || !viewport_origin.iter().all(|&v| is_finite(v)) {
        return None;
    }
    let z = clamp_zoom(zoom) as f64;
    if !z.is_finite() || z == 0.0 {
        return None;
    }
    let sx = viewport_origin[0] + pan[0] + world.x * z;
    let sy = viewport_origin[1] + pan[1] + world.y * z;
    if !is_finite(sx) || !is_finite(sy) {
        return None;
    }
    Some(ScreenPoint { x: sx, y: sy })
}

/// screen -> world. Returns None on non-finite or zero zoom.
pub fn screen_to_world(
    screen: ScreenPoint,
    pan: [f64; 2],
    zoom: f32,
    viewport_origin: [f64; 2],
) -> Option<WorldPoint> {
    if !is_finite(screen.x) || !is_finite(screen.y) || !pan.iter().all(|&v| is_finite(v)) || !viewport_origin.iter().all(|&v| is_finite(v)) {
        return None;
    }
    let z = clamp_zoom(zoom) as f64;
    if !z.is_finite() || z == 0.0 {
        return None;
    }
    let wx = (screen.x - viewport_origin[0] - pan[0]) / z;
    let wy = (screen.y - viewport_origin[1] - pan[1]) / z;
    if !is_finite(wx) || !is_finite(wy) {
        return None;
    }
    Some(WorldPoint { x: wx, y: wy })
}

/// Pan in screen space (direct add to pan per formula).
pub fn apply_pan(current_pan: [f64; 2], delta_screen: [f64; 2]) -> [f64; 2] {
    if !current_pan.iter().all(|&v| is_finite(v)) || !delta_screen.iter().all(|&v| is_finite(v)) {
        return current_pan; // reject, keep old
    }
    [current_pan[0] + delta_screen[0], current_pan[1] + delta_screen[1]]
}

/// Zoom keeping mouse world point fixed on screen (anchor zoom).
/// Returns new pan (screen space).
pub fn zoom_anchored(
    current_pan: [f64; 2],
    current_zoom: f32,
    target_zoom: f32,
    mouse_screen: ScreenPoint,
    viewport_origin: [f64; 2],
) -> Option<[f64; 2]> {
    let old_z = clamp_zoom(current_zoom);
    let new_z = clamp_zoom(target_zoom);
    if old_z == new_z {
        return Some(current_pan);
    }
    let world = screen_to_world(mouse_screen, current_pan, old_z, viewport_origin)?;
    let z = new_z as f64;
    let new_pan = [
        mouse_screen.x - viewport_origin[0] - world.x * z,
        mouse_screen.y - viewport_origin[1] - world.y * z,
    ];
    if !new_pan.iter().all(|&v| is_finite(v)) {
        return None;
    }
    Some(new_pan)
}

#[cfg(test)]
/// Roundtrip check helper for tests (error within epsilon).
pub fn roundtrip_error(world: WorldPoint, pan: [f64; 2], zoom: f32, viewport: [f64; 2]) -> Option<f64> {
    let s = world_to_screen(world, pan, zoom, viewport)?;
    let w2 = screen_to_world(s, pan, zoom, viewport)?;
    let dx = (w2.x - world.x).abs();
    let dy = (w2.y - world.y).abs();
    Some(dx.max(dy))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_basic() {
        let w = WorldPoint { x: 10.0, y: 20.0 };
        let pan = [5.0, -3.0];
        let z = 1.5;
        let vp = [0.0, 0.0];
        let err = roundtrip_error(w, pan, z, vp).unwrap();
        assert!(err < 1e-9, "roundtrip error too large: {}", err);
    }

    #[test]
    fn negative_coords_and_vp() {
        let w = WorldPoint { x: -42.5, y: 100.0 };
        let pan = [0.0, 0.0];
        let z = 0.5;
        let vp = [100.0, 200.0];
        let err = roundtrip_error(w, pan, z, vp).unwrap();
        assert!(err < 1e-9);
    }

    #[test]
    fn zoom_clamp_and_anchor_no_drift() {
        let mouse = ScreenPoint { x: 300.0, y: 150.0 };
        let vp = [0.0, 0.0];
        let pan0 = [10.0, 20.0];
        let z0 = 1.0f32;
        let z1 = 1.8f32;

        let w_at_mouse = screen_to_world(mouse, pan0, z0, vp).unwrap();
        let new_pan = zoom_anchored(pan0, z0, z1, mouse, vp).unwrap();
        let w2 = screen_to_world(mouse, new_pan, z1, vp).unwrap();
        let drift = (w2.x - w_at_mouse.x).abs() + (w2.y - w_at_mouse.y).abs();
        assert!(drift < 1e-8, "anchor drift: {}", drift);

        assert_eq!(clamp_zoom(0.1), 0.25);
        assert_eq!(clamp_zoom(3.0), 2.0);
        assert_eq!(clamp_zoom(1.0), 1.0);
    }

    #[test]
    fn rejects_nan_inf() {
        let bad = WorldPoint { x: f64::NAN, y: 0.0 };
        assert!(world_to_screen(bad, [0.0,0.0], 1.0, [0.0,0.0]).is_none());
        let infp = WorldPoint { x: f64::INFINITY, y: 1.0 };
        assert!(world_to_screen(infp, [0.0,0.0], 1.0, [0.0,0.0]).is_none());

        let bad_size = WorldSize { w: 0.0, h: -5.0 }; // non positive
        // model will enforce; here size not directly in transform but used in rects
        let _ = bad_size;
    }

    #[test]
    fn pan_apply_finite_only() {
        let p = apply_pan([1.0, 2.0], [10.0, -5.0]);
        assert_eq!(p, [11.0, -3.0]);
        let bad = apply_pan([1.0, 2.0], [f64::NAN, 0.0]);
        assert_eq!(bad, [1.0, 2.0]); // rejected
    }
}
