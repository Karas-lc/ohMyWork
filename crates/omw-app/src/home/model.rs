//! Plan 19: pure data model for canvas (math + hit only; no UI/persist).
//! Widgets have stable UUID (for future identity across load).
//! options: serde_json::Value for kind-specific (no plugin system per V1).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::home::transform::{WorldRect, WorldSize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Widget {
    pub id: Uuid,
    /// Kind string e.g. "note", "task-list", "pi-status" (plan20 will map to visuals).
    pub kind: String,
    pub rect: WorldRect,
    /// z-order; higher on top. Hit test uses reverse (high to low).
    pub z: i32,
    /// Kind-specific freeform; empty for base. Versioned by canvas version.
    #[serde(default)]
    pub options: serde_json::Value,
}

impl Widget {
    pub fn new(kind: impl Into<String>, rect: WorldRect) -> Self {
        Self {
            id: Uuid::new_v4(),
            kind: kind.into(),
            rect,
            z: 0,
            options: serde_json::Value::Null,
        }
    }

    /// Enforce min size at construction/resize.
    pub fn clamp_min(&mut self) {
        let min = WorldRect::MIN_SIZE;
        if self.rect.size.w < min.w {
            self.rect.size.w = min.w;
        }
        if self.rect.size.h < min.h {
            self.rect.size.h = min.h;
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CanvasState {
    pub version: u32,
    pub pan: [f64; 2],
    pub zoom: f32,
    pub widgets: Vec<Widget>,
}

impl Default for CanvasState {
    fn default() -> Self {
        Self {
            version: 1,
            pan: [0.0, 0.0],
            zoom: 1.0,
            widgets: vec![],
        }
    }
}

impl CanvasState {
    pub const CURRENT_VERSION: u32 = 1;

    pub fn new() -> Self {
        Self::default()
    }

    /// Add widget; lift to top z.
    pub fn add_widget(&mut self, mut w: Widget) {
        let max_z = self.widgets.iter().map(|x| x.z).max().unwrap_or(0);
        w.z = max_z + 1;
        w.clamp_min();
        self.widgets.push(w);
    }

    /// Remove by id (explicit only).
    pub fn remove_widget(&mut self, id: Uuid) {
        self.widgets.retain(|w| w.id != id);
    }

    /// Bring to front (increase z).
    pub fn raise(&mut self, id: Uuid) {
        if let Some(idx) = self.widgets.iter().position(|w| w.id == id) {
            let max = self.widgets.iter().map(|x| x.z).max().unwrap_or(0);
            self.widgets[idx].z = max + 1;
        }
    }

    /// Find widget by id.
    pub fn find(&self, id: Uuid) -> Option<&Widget> {
        self.widgets.iter().find(|w| w.id == id)
    }

    /// Mutable find.
    pub fn find_mut(&mut self, id: Uuid) -> Option<&mut Widget> {
        self.widgets.iter_mut().find(|w| w.id == id)
    }

    /// Reject invalid (NaN etc) before apply. Pure model check.
    pub fn validate(&self) -> bool {
        self.version == Self::CURRENT_VERSION
            && self.pan.iter().all(|&v| v.is_finite())
            && self.zoom.is_finite() && (0.25..=2.0).contains(&self.zoom)
            && self.widgets.iter().all(|w| {
                let r = &w.rect;
                r.origin.x.is_finite() && r.origin.y.is_finite()
                    && r.size.w.is_finite() && r.size.h.is_finite()
                    && r.size.w > 0.0 && r.size.h > 0.0
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::home::transform::WorldPoint;

    #[test]
    fn add_raises_z_and_clamps() {
        let mut c = CanvasState::new();
        let r = WorldRect::new(0.0, 0.0, 10.0, 10.0); // below min
        let w = Widget::new("test", r);
        c.add_widget(w);
        assert_eq!(c.widgets.len(), 1);
        assert!(c.widgets[0].z > 0);
        assert_eq!(c.widgets[0].rect.size.w, WorldRect::MIN_SIZE.w);
    }

    #[test]
    fn validate_rejects_bad() {
        let mut c = CanvasState::new();
        c.zoom = 3.0;
        assert!(!c.validate());
        c.zoom = 1.0;
        let mut bad = Widget::new("x", WorldRect::new(0.0, 0.0, 10.0, 10.0));
        bad.rect.size.w = f64::NAN;
        c.widgets.push(bad);
        assert!(!c.validate());
    }
}
