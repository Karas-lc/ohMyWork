//! Plan 20: builtin widget kinds + helpers.
//! Canvas only does world pos/z/clip; widgets compose Kit inside their rect.
//! Only V1 builtins; no calendar, no multi-ws, no plugins.

use serde::{Deserialize, Serialize};

use crate::home::transform::WorldSize;

/// Built-in widget kinds for V1 Homepage (HOME-003).
/// All deletable; empty canvas allows re-add.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BuiltinWidgetKind {
    /// Workspace root entry (Atlas)
    Atlas,
    /// Efforts/projects entry
    Efforts,
    /// Archive (excluded by default)
    Archive,
    /// Recent pages (path refs only)
    Recent,
    /// Pinned pages (path refs only)
    Pinned,
    /// Basic tasks from index (no copy)
    Tasks,
    /// Pi status/entry (no transcript)
    Pi,
    /// Quick actions
    Actions,
}

impl BuiltinWidgetKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Atlas => "atlas",
            Self::Efforts => "efforts",
            Self::Archive => "archive",
            Self::Recent => "recent",
            Self::Pinned => "pinned",
            Self::Tasks => "tasks",
            Self::Pi => "pi",
            Self::Actions => "actions",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "atlas" => Some(Self::Atlas),
            "efforts" => Some(Self::Efforts),
            "archive" => Some(Self::Archive),
            "recent" => Some(Self::Recent),
            "pinned" => Some(Self::Pinned),
            "tasks" => Some(Self::Tasks),
            "pi" => Some(Self::Pi),
            "actions" => Some(Self::Actions),
            _ => None,
        }
    }

    /// Minimum logical size (can be overridden by kind in future).
    pub fn min_size(&self) -> WorldSize {
        match self {
            Self::Atlas | Self::Efforts | Self::Archive => WorldSize { w: 160.0, h: 80.0 },
            Self::Recent | Self::Pinned => WorldSize { w: 220.0, h: 140.0 },
            Self::Tasks => WorldSize { w: 280.0, h: 160.0 },
            Self::Pi => WorldSize { w: 200.0, h: 100.0 },
            Self::Actions => WorldSize { w: 180.0, h: 60.0 },
        }
    }

    pub fn default_title(&self) -> &'static str {
        match self {
            Self::Atlas => "Atlas",
            Self::Efforts => "Efforts",
            Self::Archive => "Archive",
            Self::Recent => "Recent",
            Self::Pinned => "Pinned",
            Self::Tasks => "Tasks",
            Self::Pi => "Pi",
            Self::Actions => "Actions",
        }
    }

    /// Default options (empty; validated on import).
    pub fn default_options(&self) -> serde_json::Value {
        serde_json::json!({})
    }
}

impl std::fmt::Display for BuiltinWidgetKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Starting deletable layout (per HOME-003, no calendar/multi-ws).
pub fn default_start_layout() -> crate::home::CanvasState {
    use crate::home::{CanvasState, Widget, WorldRect};

    let mut state = CanvasState::new();

    // Small initial set; user can delete/add all.
    let kinds = [
        BuiltinWidgetKind::Atlas,
        BuiltinWidgetKind::Efforts,
        BuiltinWidgetKind::Tasks,
        BuiltinWidgetKind::Pi,
        BuiltinWidgetKind::Actions,
    ];

    let mut x = 40.0;
    let y = 40.0;
    for k in kinds {
        let size = k.min_size();
        let mut w = Widget::new(k.as_str(), WorldRect { origin: crate::home::WorldPoint { x, y }, size });
        w.options = k.default_options();
        state.add_widget(w);
        x += size.w + 20.0;
    }

    state
}

/// Validate kind string is builtin (for import).
pub fn is_valid_kind(kind: &str) -> bool {
    BuiltinWidgetKind::from_str(kind).is_some()
}
