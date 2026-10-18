//! Map SemanticTheme (post-contrast) to gpui-kit Theme for standard components.
//! App-specific tokens (Canvas, Task, Pi, Markdown) live only on the SemanticTheme snapshot.
//! Core mappings: background→background, text→foreground, panel.background→sidebar/list,
//! editor.background→input (editor bg via highlight fallback in kit), + hover/active/selected/focus/error/warning/disabled.
//! Exact Kit type/path: gpui_kit::component::theme::{Theme, ThemeColor} (gpui-component-0.7.0/src/theme/{mod.rs,theme_color.rs}).
//! Never calls ThemeRegistry::watch_dir.

use gpui_kit::component::theme::ThemeColor;
use gpui_kit::Hsla;
use crate::theme::tokens::SemanticTheme;

pub fn to_kit_colors(sem: &SemanticTheme) -> ThemeColor {
    let t = &sem.tokens;
    let mut c = ThemeColor::default();
    c.background = t.background;
    c.foreground = t.text_primary;
    c.sidebar = t.panel;
    c.sidebar_foreground = t.foreground;
    c.sidebar_border = t.border;
    c.list = t.panel;
    c.list_hover = mix(t.panel, t.selection, 0.15);
    c.list_active = mix(t.panel, t.focus, 0.25);
    c.input = t.editor_bg;
    c.border = t.border;
    c.selection = t.selection;
    c.ring = t.focus;
    c.danger = t.error;
    c.danger_foreground = t.foreground;
    c.danger_hover = mix(t.error, t.background, 0.2);
    c.warning = t.warning;
    c.warning_foreground = t.foreground;
    c.warning_hover = mix(t.warning, t.background, 0.15);
    c.muted_foreground = t.disabled;
    c.muted = mix(t.panel, t.disabled, 0.25);
    c.primary_hover = mix(t.panel, t.selection, 0.18);
    c.primary_active = mix(t.panel, t.focus, 0.22);
    c.secondary_hover = mix(t.panel, t.foreground, 0.08);
    c.secondary_active = mix(t.panel, t.focus, 0.18);
    c.button_hover = c.primary_hover;
    c.button_active = c.primary_active;
    c.accent = t.focus;
    c.accent_foreground = t.foreground;
    c
}

fn mix(a: Hsla, b: Hsla, t: f32) -> Hsla {
    Hsla { h: a.h + (b.h - a.h) * t, s: a.s + (b.s - a.s) * t, l: a.l + (b.l - a.l) * t, a: a.a + (b.a - a.a) * t }
}

pub fn to_kit_theme(sem: &SemanticTheme) -> gpui_kit::component::theme::Theme {
    gpui_kit::component::theme::Theme::from(&to_kit_colors(sem))
}
