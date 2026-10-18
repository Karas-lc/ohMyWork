//! Semantic tokens for ohMyWork. Two snapshots: light + dark.
//! Components consume these, not raw Zed keys.
use gpui_kit::{hsla, Hsla};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeMode {
    Light,
    Dark,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TokenSet {
    pub background: Hsla,
    pub foreground: Hsla,
    pub panel: Hsla,
    pub editor_bg: Hsla,
    pub border: Hsla,
    pub text_primary: Hsla,
    pub selection: Hsla,
    pub focus: Hsla,
    pub error: Hsla,
    pub warning: Hsla,
    pub disabled: Hsla,
    // app extensions
    pub canvas_bg: Hsla,
    pub task_done: Hsla,
    pub pi_bg: Hsla,
}

impl TokenSet {
    pub fn light_default() -> Self {
        Self {
            background: hsla(0.0, 0.0, 0.98, 1.0),
            foreground: hsla(0.0, 0.0, 0.1, 1.0),
            panel: hsla(0.0, 0.0, 0.95, 1.0),
            editor_bg: hsla(0.0, 0.0, 1.0, 1.0),
            border: hsla(0.0, 0.0, 0.85, 1.0),
            text_primary: hsla(0.0, 0.0, 0.1, 1.0),
            selection: hsla(210.0 / 360.0, 0.8, 0.85, 0.6),
            focus: hsla(210.0 / 360.0, 0.9, 0.6, 1.0),
            error: hsla(0.0, 0.8, 0.5, 1.0),
            warning: hsla(40.0 / 360.0, 0.9, 0.5, 1.0),
            disabled: hsla(0.0, 0.0, 0.6, 1.0),
            canvas_bg: hsla(0.0, 0.0, 0.97, 1.0),
            task_done: hsla(120.0 / 360.0, 0.5, 0.4, 1.0),
            pi_bg: hsla(0.0, 0.0, 0.92, 1.0),
        }
    }

    pub fn dark_default() -> Self {
        Self {
            background: hsla(0.0, 0.0, 0.12, 1.0),
            foreground: hsla(0.0, 0.0, 0.9, 1.0),
            panel: hsla(0.0, 0.0, 0.16, 1.0),
            editor_bg: hsla(0.0, 0.0, 0.1, 1.0),
            border: hsla(0.0, 0.0, 0.25, 1.0),
            text_primary: hsla(0.0, 0.0, 0.9, 1.0),
            selection: hsla(210.0 / 360.0, 0.7, 0.35, 0.6),
            focus: hsla(210.0 / 360.0, 0.8, 0.55, 1.0),
            error: hsla(0.0, 0.7, 0.45, 1.0),
            warning: hsla(40.0 / 360.0, 0.8, 0.55, 1.0),
            disabled: hsla(0.0, 0.0, 0.5, 1.0),
            canvas_bg: hsla(0.0, 0.0, 0.14, 1.0),
            task_done: hsla(120.0 / 360.0, 0.4, 0.5, 1.0),
            pi_bg: hsla(0.0, 0.0, 0.18, 1.0),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SemanticTheme {
    pub version: u32,
    pub mode: ThemeMode,
    pub tokens: TokenSet,
}

impl SemanticTheme {
    pub fn builtin_light() -> Self {
        Self { version: 1, mode: ThemeMode::Light, tokens: TokenSet::light_default() }
    }
    pub fn builtin_dark() -> Self {
        Self { version: 1, mode: ThemeMode::Dark, tokens: TokenSet::dark_default() }
    }
}
