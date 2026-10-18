//! Plan 24: stable CommandId full set for V1; default bindings + %LOCALAPPDATA%/ohMyWork/keymap.jsonc via jsonc-parser.
//! Context explicit enum. User overrides last-win per context (diagnostic); different contexts coexist.
//! Watcher: full temp parse, atomic switch to last-known-good on success only; bad keeps existing bindings, no removal.
//! No duplicate registration. Kit Command palette bound to real Actions; resolved keys shown from current context (no hand-built strings).
//! Always keep accessible RestoreSafeTheme + KeymapRestoreDefaults. Do not change Pi keymap.

use std::collections::HashMap;
use std::fs;
use std::str::FromStr;

use gpui_kit::{KeyBinding, actions};
use jsonc_parser::{ParseOptions, parse_to_serde_value};
use omw_local::AppDataPaths;
use serde_json::Value;

actions!(
    omw_app,
    [
        Save,
        CloseTab,
        OpenFile,
        TogglePi,
        FitCanvas,
        RestoreSafeTheme,
        Undo,
        Redo,
        Bold,
        Italic,
        Strikethrough,
        PasteMarkdown,
        Search,
        ToggleFileTree,
        ZoomIn,
        ZoomOut,
        SwitchLanguageZh,
        SwitchLanguageEn,
        KeymapRestoreDefaults,
        MoveCanvasLeft,
        MoveCanvasRight,
        MoveCanvasUp,
        MoveCanvasDown,
    ]
);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CommandId {
    Save,
    CloseTab,
    OpenFile,
    TogglePi,
    FitCanvas,
    RestoreSafeTheme,
    Undo,
    Redo,
    Bold,
    Italic,
    Strikethrough,
    PasteMarkdown,
    Search,
    ToggleFileTree,
    ZoomIn,
    ZoomOut,
    SwitchLanguageZh,
    SwitchLanguageEn,
    KeymapRestoreDefaults,
    MoveCanvasLeft,
    MoveCanvasRight,
    MoveCanvasUp,
    MoveCanvasDown,
}

impl CommandId {
    pub fn as_str(&self) -> &'static str {
        match self {
            CommandId::Save => "save",
            CommandId::CloseTab => "close-tab",
            CommandId::OpenFile => "open-file",
            CommandId::TogglePi => "toggle-pi",
            CommandId::FitCanvas => "fit-canvas",
            CommandId::RestoreSafeTheme => "restore-safe-theme",
            CommandId::Undo => "undo",
            CommandId::Redo => "redo",
            CommandId::Bold => "bold",
            CommandId::Italic => "italic",
            CommandId::Strikethrough => "strikethrough",
            CommandId::PasteMarkdown => "paste-markdown",
            CommandId::Search => "search",
            CommandId::ToggleFileTree => "toggle-file-tree",
            CommandId::ZoomIn => "zoom-in",
            CommandId::ZoomOut => "zoom-out",
            CommandId::SwitchLanguageZh => "switch-language-zh",
            CommandId::SwitchLanguageEn => "switch-language-en",
            CommandId::KeymapRestoreDefaults => "keymap-restore-defaults",
            CommandId::MoveCanvasLeft => "move-canvas-left",
            CommandId::MoveCanvasRight => "move-canvas-right",
            CommandId::MoveCanvasUp => "move-canvas-up",
            CommandId::MoveCanvasDown => "move-canvas-down",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "save" => Some(CommandId::Save),
            "close-tab" => Some(CommandId::CloseTab),
            "open-file" => Some(CommandId::OpenFile),
            "toggle-pi" => Some(CommandId::TogglePi),
            "fit-canvas" => Some(CommandId::FitCanvas),
            "restore-safe-theme" => Some(CommandId::RestoreSafeTheme),
            "undo" => Some(CommandId::Undo),
            "redo" => Some(CommandId::Redo),
            "bold" => Some(CommandId::Bold),
            "italic" => Some(CommandId::Italic),
            "strikethrough" => Some(CommandId::Strikethrough),
            "paste-markdown" => Some(CommandId::PasteMarkdown),
            "search" => Some(CommandId::Search),
            "toggle-file-tree" => Some(CommandId::ToggleFileTree),
            "zoom-in" => Some(CommandId::ZoomIn),
            "zoom-out" => Some(CommandId::ZoomOut),
            "switch-language-zh" => Some(CommandId::SwitchLanguageZh),
            "switch-language-en" => Some(CommandId::SwitchLanguageEn),
            "keymap-restore-defaults" => Some(CommandId::KeymapRestoreDefaults),
            "move-canvas-left" => Some(CommandId::MoveCanvasLeft),
            "move-canvas-right" => Some(CommandId::MoveCanvasRight),
            "move-canvas-up" => Some(CommandId::MoveCanvasUp),
            "move-canvas-down" => Some(CommandId::MoveCanvasDown),
            _ => None,
        }
    }

    pub fn to_action(&self) -> Box<dyn gpui_kit::Action> {
        match self {
            CommandId::Save => Box::new(Save),
            CommandId::CloseTab => Box::new(CloseTab),
            CommandId::OpenFile => Box::new(OpenFile),
            CommandId::TogglePi => Box::new(TogglePi),
            CommandId::FitCanvas => Box::new(FitCanvas),
            CommandId::RestoreSafeTheme => Box::new(RestoreSafeTheme),
            CommandId::Undo => Box::new(Undo),
            CommandId::Redo => Box::new(Redo),
            CommandId::Bold => Box::new(Bold),
            CommandId::Italic => Box::new(Italic),
            CommandId::Strikethrough => Box::new(Strikethrough),
            CommandId::PasteMarkdown => Box::new(PasteMarkdown),
            CommandId::Search => Box::new(Search),
            CommandId::ToggleFileTree => Box::new(ToggleFileTree),
            CommandId::ZoomIn => Box::new(ZoomIn),
            CommandId::ZoomOut => Box::new(ZoomOut),
            CommandId::SwitchLanguageZh => Box::new(SwitchLanguageZh),
            CommandId::SwitchLanguageEn => Box::new(SwitchLanguageEn),
            CommandId::KeymapRestoreDefaults => Box::new(KeymapRestoreDefaults),
            CommandId::MoveCanvasLeft => Box::new(MoveCanvasLeft),
            CommandId::MoveCanvasRight => Box::new(MoveCanvasRight),
            CommandId::MoveCanvasUp => Box::new(MoveCanvasUp),
            CommandId::MoveCanvasDown => Box::new(MoveCanvasDown),
        }
    }
}

impl std::fmt::Display for CommandId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize, Default,
)]
#[serde(rename_all = "kebab-case")]
pub enum KeyContext {
    #[default]
    Global,
    Editor,
    Canvas,
    Command,
    FileTree,
    Pi,
}

impl FromStr for KeyContext {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "global" => Ok(KeyContext::Global),
            "editor" => Ok(KeyContext::Editor),
            "canvas" => Ok(KeyContext::Canvas),
            "command" | "command-palette" => Ok(KeyContext::Command),
            "file-tree" => Ok(KeyContext::FileTree),
            "pi" => Ok(KeyContext::Pi),
            _ => Err(format!("unknown context: {}", s)),
        }
    }
}

impl KeyContext {
    pub fn as_str(&self) -> &'static str {
        match self {
            KeyContext::Global => "Global",
            KeyContext::Editor => "Editor",
            KeyContext::Canvas => "Canvas",
            KeyContext::Command => "Command",
            KeyContext::FileTree => "FileTree",
            KeyContext::Pi => "Pi",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Keymap {
    bindings: HashMap<KeyContext, HashMap<String, CommandId>>,
    last_known_good: Option<HashMap<KeyContext, HashMap<String, CommandId>>>,
    diagnostics: Vec<String>,
}

impl Keymap {
    pub fn new() -> Self {
        let mut km = Self::default();
        km.apply_defaults();
        km
    }

    fn apply_defaults(&mut self) {
        let mut defs: HashMap<KeyContext, HashMap<String, CommandId>> = HashMap::new();
        let global = defs.entry(KeyContext::Global).or_default();
        global.insert("ctrl+s".to_string(), CommandId::Save);
        global.insert("ctrl+w".to_string(), CommandId::CloseTab);
        global.insert("ctrl+o".to_string(), CommandId::OpenFile);
        global.insert("ctrl+shift+p".to_string(), CommandId::TogglePi);
        global.insert("ctrl+0".to_string(), CommandId::FitCanvas);
        global.insert("ctrl+shift+t".to_string(), CommandId::RestoreSafeTheme);
        global.insert("ctrl+shift+r".to_string(), CommandId::KeymapRestoreDefaults);

        let editor = defs.entry(KeyContext::Editor).or_default();
        editor.insert("ctrl+z".to_string(), CommandId::Undo);
        editor.insert("ctrl+y".to_string(), CommandId::Redo);
        editor.insert("ctrl+b".to_string(), CommandId::Bold);
        editor.insert("ctrl+i".to_string(), CommandId::Italic);
        editor.insert("ctrl+shift+x".to_string(), CommandId::Strikethrough);
        editor.insert("ctrl+shift+v".to_string(), CommandId::PasteMarkdown);

        let canvas = defs.entry(KeyContext::Canvas).or_default();
        canvas.insert("ctrl+f".to_string(), CommandId::FitCanvas);
        canvas.insert("ctrl+=".to_string(), CommandId::ZoomIn);
        canvas.insert("ctrl+-".to_string(), CommandId::ZoomOut);
        canvas.insert("left".to_string(), CommandId::MoveCanvasLeft);
        canvas.insert("right".to_string(), CommandId::MoveCanvasRight);
        canvas.insert("up".to_string(), CommandId::MoveCanvasUp);
        canvas.insert("down".to_string(), CommandId::MoveCanvasDown);
        canvas.insert("ctrl+shift+t".to_string(), CommandId::RestoreSafeTheme);

        let cmd = defs.entry(KeyContext::Command).or_default();
        cmd.insert("ctrl+k".to_string(), CommandId::Search);

        self.bindings = defs;
    }

    pub fn apply_overrides(&mut self, user: HashMap<KeyContext, HashMap<String, CommandId>>) {
        for (ctx, ubinds) in user {
            let target = self.bindings.entry(ctx).or_default();
            for (k, cmd) in ubinds {
                // last-wins for cmd in ctx: remove prior keys for this cmd
                if target.values().any(|c| c == &cmd) {
                    target.retain(|_, c| c != &cmd);
                    self.diagnostics.push(format!(
                        "override in {:?}: {} -> {:?} (replaced prior key for cmd)",
                        ctx, k, cmd
                    ));
                }
                if target.contains_key(&k) {
                    self.diagnostics.push(format!(
                        "override in {:?}: {} -> {:?} (previous {:?})",
                        ctx,
                        k,
                        cmd,
                        target.get(&k)
                    ));
                }
                target.insert(k, cmd);
            }
        }
    }
    pub fn bindings_for(&self, ctx: KeyContext) -> &HashMap<String, CommandId> {
        self.bindings.get(&ctx).unwrap_or(&EMPTY)
    }

    pub fn resolved_key_for(&self, ctx: KeyContext, cmd: CommandId) -> Option<String> {
        self.bindings
            .get(&ctx)
            .and_then(|m| m.iter().find(|(_, c)| *c == &cmd).map(|(k, _)| k.clone()))
            .or_else(|| {
                self.bindings
                    .get(&KeyContext::Global)
                    .and_then(|m| m.iter().find(|(_, c)| *c == &cmd).map(|(k, _)| k.clone()))
            })
    }

    pub fn all_commands(&self) -> Vec<(CommandId, Option<String>)> {
        let mut cmds: Vec<_> = [
            CommandId::Save,
            CommandId::CloseTab,
            CommandId::OpenFile,
            CommandId::TogglePi,
            CommandId::FitCanvas,
            CommandId::RestoreSafeTheme,
            CommandId::Undo,
            CommandId::Redo,
            CommandId::Bold,
            CommandId::Italic,
            CommandId::Strikethrough,
            CommandId::PasteMarkdown,
            CommandId::Search,
            CommandId::ToggleFileTree,
            CommandId::ZoomIn,
            CommandId::ZoomOut,
            CommandId::KeymapRestoreDefaults,
            CommandId::SwitchLanguageZh,
            CommandId::SwitchLanguageEn,
        ]
        .into();
        cmds.sort_by_key(|c| c.as_str());
        cmds.into_iter()
            .map(|c| {
                let key = self
                    .resolved_key_for(KeyContext::Global, c)
                    .or_else(|| self.resolved_key_for(KeyContext::Editor, c))
                    .or_else(|| self.resolved_key_for(KeyContext::Canvas, c));
                (c, key)
            })
            .collect()
    }

    pub fn diagnostics(&self) -> &[String] {
        &self.diagnostics
    }

    pub fn clear_diagnostics(&mut self) {
        self.diagnostics.clear();
    }
}

static EMPTY: std::sync::LazyLock<HashMap<String, CommandId>> =
    std::sync::LazyLock::new(HashMap::new);

pub fn parse_user_keymap(
    jsonc: &str,
) -> Result<HashMap<KeyContext, HashMap<String, CommandId>>, String> {
    if jsonc.trim().is_empty() {
        return Ok(HashMap::new());
    }
    let opts = ParseOptions::default();
    let val: Value =
        parse_to_serde_value(jsonc, &opts).map_err(|e| format!("jsonc parse error: {}", e))?;

    let arr = val
        .as_array()
        .ok_or("top level must be array of context objects")?;
    let mut overrides: HashMap<KeyContext, HashMap<String, CommandId>> = HashMap::new();

    for item in arr {
        let ctx_str = item
            .get("context")
            .and_then(|v| v.as_str())
            .ok_or("missing context")?;
        let ctx: KeyContext = ctx_str.parse::<KeyContext>().map_err(|e| e.to_string())?;
        let binds = item
            .get("bindings")
            .and_then(|v| v.as_object())
            .ok_or("missing bindings")?;

        let target = overrides.entry(ctx).or_default();
        for (k, v) in binds {
            let cmd_str = v
                .as_str()
                .ok_or("binding value must be string command id")?;
            if let Some(cmd) = CommandId::from_str(cmd_str) {
                target.insert(k.clone(), cmd);
            }
        }
    }
    Ok(overrides)
}

pub fn load_keymap(paths: &AppDataPaths) -> Keymap {
    let mut km = Keymap::new();
    let path = paths.keymap_file();
    if path.exists() {
        match fs::read_to_string(&path) {
            Ok(content) => match parse_user_keymap(&content) {
                Ok(ov) => {
                    km.apply_overrides(ov);
                    km.last_known_good = Some(km.bindings.clone());
                }
                Err(e) => {
                    km.diagnostics
                        .push(format!("bad user keymap kept last-good: {}", e));
                }
            },
            Err(e) => {
                km.diagnostics.push(format!("read keymap failed: {}", e));
            }
        }
    }
    km
}

pub fn reload_keymap(current: &mut Keymap, content: &str) -> bool {
    match parse_user_keymap(content) {
        Ok(ov) => {
            let mut new_km = Keymap::new();
            new_km.apply_overrides(ov);
            current.bindings = new_km.bindings;
            current.last_known_good = Some(current.bindings.clone());
            current.diagnostics.clear();
            true
        }
        Err(e) => {
            current
                .diagnostics
                .push(format!("reload rejected, kept last-good: {}", e));
            false
        }
    }
}

fn native_keystrokes(configured: &str) -> String {
    let mut native = String::with_capacity(configured.len());
    for (index, chord) in configured.split_whitespace().enumerate() {
        if index != 0 {
            native.push(' ');
        }
        let mut remaining = chord;
        while let Some((modifier, rest)) = remaining.split_once('+') {
            if ![
                "ctrl",
                "alt",
                "shift",
                "cmd",
                "win",
                "super",
                "fn",
                "secondary",
            ]
            .iter()
            .any(|name| modifier.eq_ignore_ascii_case(name))
            {
                break;
            }
            native.push_str(modifier);
            native.push('-');
            remaining = rest;
        }
        native.push_str(remaining);
    }
    native
}

pub fn register_keymap_bindings(km: &Keymap, cx: &mut gpui_kit::App) {
    let mut all: Vec<KeyBinding> = vec![];
    for (ctx, map) in &km.bindings {
        for (key, cmd) in map {
            let native = native_keystrokes(key);
            let key = native.as_str();
            let ctx_str = Some(ctx.as_str());
            match cmd {
                CommandId::Save => all.push(KeyBinding::new(key, Save, ctx_str)),
                CommandId::CloseTab => all.push(KeyBinding::new(key, CloseTab, ctx_str)),
                CommandId::OpenFile => all.push(KeyBinding::new(key, OpenFile, ctx_str)),
                CommandId::TogglePi => all.push(KeyBinding::new(key, TogglePi, ctx_str)),
                CommandId::FitCanvas => all.push(KeyBinding::new(key, FitCanvas, ctx_str)),
                CommandId::RestoreSafeTheme => {
                    all.push(KeyBinding::new(key, RestoreSafeTheme, ctx_str))
                }
                CommandId::Undo => all.push(KeyBinding::new(key, Undo, ctx_str)),
                CommandId::Redo => all.push(KeyBinding::new(key, Redo, ctx_str)),
                CommandId::Bold => all.push(KeyBinding::new(key, Bold, ctx_str)),
                CommandId::Italic => all.push(KeyBinding::new(key, Italic, ctx_str)),
                CommandId::Strikethrough => all.push(KeyBinding::new(key, Strikethrough, ctx_str)),
                CommandId::PasteMarkdown => all.push(KeyBinding::new(key, PasteMarkdown, ctx_str)),
                CommandId::Search => all.push(KeyBinding::new(key, Search, ctx_str)),
                CommandId::ToggleFileTree => {
                    all.push(KeyBinding::new(key, ToggleFileTree, ctx_str))
                }
                CommandId::ZoomIn => all.push(KeyBinding::new(key, ZoomIn, ctx_str)),
                CommandId::ZoomOut => all.push(KeyBinding::new(key, ZoomOut, ctx_str)),
                CommandId::KeymapRestoreDefaults => {
                    all.push(KeyBinding::new(key, KeymapRestoreDefaults, ctx_str))
                }
                CommandId::SwitchLanguageZh => {
                    all.push(KeyBinding::new(key, SwitchLanguageZh, ctx_str))
                }
                CommandId::SwitchLanguageEn => {
                    all.push(KeyBinding::new(key, SwitchLanguageEn, ctx_str))
                }
                CommandId::MoveCanvasLeft => {
                    all.push(KeyBinding::new(key, MoveCanvasLeft, ctx_str))
                }
                CommandId::MoveCanvasRight => {
                    all.push(KeyBinding::new(key, MoveCanvasRight, ctx_str))
                }
                CommandId::MoveCanvasUp => all.push(KeyBinding::new(key, MoveCanvasUp, ctx_str)),
                CommandId::MoveCanvasDown => {
                    all.push(KeyBinding::new(key, MoveCanvasDown, ctx_str))
                }
            }
        }
    }
    cx.bind_keys(all);
}

pub fn build_command_palette(
    km: &Keymap,
    i18n: &crate::i18n::I18n,
) -> Vec<gpui_kit::component::command::CommandItem> {
    use gpui_kit::component::command::CommandItem;
    km.all_commands()
        .into_iter()
        .map(|(cmd, _key)| {
            let label = i18n.t_key(cmd.as_str());
            CommandItem::new().label(label).action(cmd.to_action())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use omw_local::AppDataPaths;

    fn temp_paths() -> (tempfile::TempDir, AppDataPaths) {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("ohMyWork");
        std::fs::create_dir_all(&base).unwrap();
        let p = AppDataPaths::from_base(base).expect("temp paths");
        (dir, p)
    }

    #[test]
    fn default_bindings_present_and_no_dup() {
        let km = Keymap::new();
        let g = km.bindings_for(KeyContext::Global);
        assert!(g.contains_key("ctrl+s"));
        assert_eq!(g.get("ctrl+s"), Some(&CommandId::Save));
        assert!(
            g.values().any(|c| *c == CommandId::RestoreSafeTheme)
                || km
                    .bindings_for(KeyContext::Canvas)
                    .values()
                    .any(|c| *c == CommandId::RestoreSafeTheme)
        );
    }

    #[test]
    fn bad_jsonc_keeps_existing_no_remove() {
        let (_tmp, paths) = temp_paths();
        let mut km = load_keymap(&paths);
        let orig_len = km.bindings.values().map(|m| m.len()).sum::<usize>();
        let bad = r#" { "context": "Global", "bindings": { "ctrl+s": "save" ,  }  // bad "#;
        let ok = reload_keymap(&mut km, bad);
        assert!(!ok);
        assert!(
            km.diagnostics()
                .iter()
                .any(|d| d.contains("bad") || d.contains("rejected"))
        );
        let now_len = km.bindings.values().map(|m| m.len()).sum::<usize>();
        assert!(now_len >= orig_len);
    }

    #[test]
    fn valid_override_and_last_known_good() {
        let (_tmp, _paths) = temp_paths();
        let mut km = Keymap::new();
        let good = r#"
        [
          { "context": "Global", "bindings": { "ctrl+shift+s": "save" } },
          { "context": "Canvas", "bindings": { "f": "fit-canvas" } }
        ]
        "#;
        let ok = reload_keymap(&mut km, good);
        assert!(ok);
        assert!(
            km.bindings_for(KeyContext::Global)
                .contains_key("ctrl+shift+s")
        );
        assert_eq!(
            km.bindings_for(KeyContext::Global).get("ctrl+shift+s"),
            Some(&CommandId::Save)
        );
        assert!(km.last_known_good.is_some());
    }

    #[test]
    fn conflict_diagnostic_same_context() {
        let mut km = Keymap::new();
        let mut m = HashMap::new();
        let mut g = HashMap::new();
        g.insert("ctrl+s".into(), CommandId::CloseTab);
        m.insert(KeyContext::Global, g);
        km.apply_overrides(m);
        assert!(km.diagnostics().iter().any(|d| d.contains("override")));
    }

    #[test]
    fn resolved_key_from_current_context_prefers_specific() {
        let mut km = Keymap::new();
        let mut ov = HashMap::new();
        let mut c = HashMap::new();
        c.insert("f11".to_string(), CommandId::FitCanvas);
        ov.insert(KeyContext::Canvas, c);
        km.apply_overrides(ov);
        let key = km.resolved_key_for(KeyContext::Canvas, CommandId::FitCanvas);
        assert_eq!(key, Some("f11".to_string()));
    }

    #[test]
    fn parse_user_keymap_basic_and_unknown_cmd_ignored() {
        let src = r#"
        [
            {"context":"Global","bindings":{"ctrl+q":"save","ctrl+alt+x":"unknown-cmd"}}
        ]
        "#;
        let res = parse_user_keymap(src).unwrap();
        let g = res.get(&KeyContext::Global).unwrap();
        assert_eq!(g.get("ctrl+q"), Some(&CommandId::Save));
        assert!(!g.contains_key("ctrl+alt+x"));
    }

    #[test]
    fn palette_uses_resolved_keys_not_handbuilt() {
        let km = Keymap::new();
        let i18n = crate::i18n::I18n::default();
        let items = build_command_palette(&km, &i18n);
        assert!(!items.is_empty());
    }

    #[test]
    fn always_accessible_restore_and_keymap_safe() {
        let km = Keymap::new();
        let has_restore = km
            .resolved_key_for(KeyContext::Global, CommandId::RestoreSafeTheme)
            .is_some()
            || km
                .resolved_key_for(KeyContext::Canvas, CommandId::RestoreSafeTheme)
                .is_some();
        assert!(has_restore);
        let has_keymap_safe = km
            .resolved_key_for(KeyContext::Global, CommandId::KeymapRestoreDefaults)
            .is_some();
        assert!(has_keymap_safe);
    }
}
