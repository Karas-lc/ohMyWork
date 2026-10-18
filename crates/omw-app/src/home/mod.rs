//! Plan 19: Homepage infinite canvas (pure model + math + interaction only).
//! Per 19-canvas.md: no specific widgets, no persist (plan 20), no grid/calendar/plugin.
//! Canvas hosted in Home tab (shell dock already confirmed).
pub mod view;

pub mod model;
pub mod transform;

pub use model::{CanvasState, Widget};
pub mod hit_test;

pub use hit_test::{
    CanvasController, apply_world_delta, hit_test, lift_to_top, resize_clamped, world_delta,
};
pub use transform::{
    ScreenPoint, WorldPoint, WorldRect, WorldSize, apply_pan, clamp_zoom, screen_to_world,
    world_to_screen, zoom_anchored,
};
pub mod widgets;

pub use widgets::{BuiltinWidgetKind, default_start_layout, is_valid_kind};
pub mod persist;

pub use persist::CanvasPersist;
