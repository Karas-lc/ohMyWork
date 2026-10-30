//! Plan 24: i18n with stable keys, identical zh/en key sets, runtime switch.
//! All host UI strings via StringId; do not translate dir names, storage keys, Markdown, Pi output.

use std::collections::HashMap;
#[cfg(test)]
use std::collections::HashSet;
use std::sync::OnceLock;

use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Lang {
    #[default]
    En,
    ZhCn,
}

impl Lang {
    pub fn from_str(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "zh-cn" | "zh" | "chinese" => Lang::ZhCn,
            _ => Lang::En,
        }
    }

    pub fn as_key(&self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::ZhCn => "zh-CN",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StringId {
    Save,
    Close,
    Open,
    Fit,
    Undo,
    Redo,
    Bold,
    Italic,
    Strikethrough,
    PasteMarkdown,
    Search,
    TogglePi,
    ToggleFileTree,
    ZoomIn,
    ZoomOut,
    RestoreSafeTheme,
    KeymapRestoreDefaults,
    SwitchLangZh,
    SwitchLangEn,
    ChooseDirectory,
    InitializeWorkspace,
    WorkspaceRoot,
    Cancel,
    Confirm,
    Status,
    Editor,
    Canvas,
    FileTree,
    PiPanel,
    CommandPalette,
    SaveStatus,
    Dirty,
    Saved,
    Error,
    Focus,
    Selection,
}

impl StringId {
    pub fn as_key(&self) -> &'static str {
        match self {
            StringId::Save => "save",
            StringId::Close => "close",
            StringId::Open => "open",
            StringId::Fit => "fit",
            StringId::Undo => "undo",
            StringId::Redo => "redo",
            StringId::Bold => "bold",
            StringId::Italic => "italic",
            StringId::Strikethrough => "strikethrough",
            StringId::PasteMarkdown => "paste-markdown",
            StringId::Search => "search",
            StringId::TogglePi => "toggle-pi",
            StringId::ToggleFileTree => "toggle-file-tree",
            StringId::ZoomIn => "zoom-in",
            StringId::ZoomOut => "zoom-out",
            StringId::RestoreSafeTheme => "restore-safe-theme",
            StringId::KeymapRestoreDefaults => "keymap-restore-defaults",
            StringId::SwitchLangZh => "switch-lang-zh",
            StringId::SwitchLangEn => "switch-lang-en",
            StringId::ChooseDirectory => "choose-directory",
            StringId::InitializeWorkspace => "initialize-workspace",
            StringId::WorkspaceRoot => "workspace-root",
            StringId::Cancel => "cancel",
            StringId::Confirm => "confirm",
            StringId::Status => "status",
            StringId::Editor => "editor",
            StringId::Canvas => "canvas",
            StringId::FileTree => "file-tree",
            StringId::PiPanel => "pi-panel",
            StringId::CommandPalette => "command-palette",
            StringId::SaveStatus => "save-status",
            StringId::Dirty => "dirty",
            StringId::Saved => "saved",
            StringId::Error => "error",
            StringId::Focus => "focus",
            StringId::Selection => "selection",
        }
    }
}

#[derive(Deserialize)]
struct RawLocale {
    #[serde(flatten)]
    entries: HashMap<String, String>,
}

static LOCALES: OnceLock<(HashMap<String, String>, HashMap<String, String>)> = OnceLock::new();

fn load_locales() -> (&'static HashMap<String, String>, &'static HashMap<String, String>) {
    let (en, zh) = LOCALES.get_or_init(|| {
        let en_raw: RawLocale = serde_json::from_str(include_str!("../locales/en.json"))
            .expect("valid en.json");
        let zh_raw: RawLocale = serde_json::from_str(include_str!("../locales/zh-CN.json"))
            .expect("valid zh-CN.json");
        (en_raw.entries, zh_raw.entries)
    });
    (en, zh)
}

/// Validate that zh and en have exactly the same key sets. Used in tests.
#[cfg(test)]
pub fn validate_key_sets() -> Result<(), String> {
    let (en, zh) = load_locales();
    let en_keys: HashSet<_> = en.keys().collect();
    let zh_keys: HashSet<_> = zh.keys().collect();
    if en_keys != zh_keys {
        let missing_en: Vec<_> = zh_keys.difference(&en_keys).collect();
        let missing_zh: Vec<_> = en_keys.difference(&zh_keys).collect();
        return Err(format!(
            "locale key mismatch: missing_in_en={:?} missing_in_zh={:?}",
            missing_en, missing_zh
        ));
    }
    for sid in all_string_ids() {
        let k = sid.as_key();
        if !en.contains_key(k) {
            return Err(format!("StringId key missing in locales: {}", k));
        }
    }
    Ok(())
}

#[cfg(test)]
fn all_string_ids() -> Vec<StringId> {
    use StringId::*;
    vec![
        Save, Close, Open, Fit, Undo, Redo, Bold, Italic, Strikethrough, PasteMarkdown,
        Search, TogglePi, ToggleFileTree, ZoomIn, ZoomOut, RestoreSafeTheme, KeymapRestoreDefaults,
        SwitchLangZh, SwitchLangEn, ChooseDirectory, InitializeWorkspace, WorkspaceRoot, Cancel,
        Confirm, Status, Editor, Canvas, FileTree, PiPanel, CommandPalette, SaveStatus, Dirty,
        Saved, Error, Focus, Selection,
    ]
}

pub struct I18n {
    current: Lang,
}

impl Default for I18n {
    fn default() -> Self {
        Self { current: Lang::default() }
    }
}

impl I18n {
    pub fn new(lang: Lang) -> Self {
        Self { current: lang }
    }

    pub fn set_lang(&mut self, lang: Lang) {
        self.current = lang;
    }

    pub fn current_lang(&self) -> Lang {
        self.current
    }

    pub fn t(&self, id: StringId) -> String {
        let key = id.as_key();
        let (en, zh) = load_locales();
        let dict = match self.current {
            Lang::En => en,
            Lang::ZhCn => zh,
        };
        dict.get(key).cloned().unwrap_or_else(|| key.to_string())
    }

    pub fn t_key(&self, key: &str) -> String {
        let (en, zh) = load_locales();
        let dict = match self.current {
            Lang::En => en,
            Lang::ZhCn => zh,
        };
        dict.get(key).cloned().unwrap_or_else(|| key.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zh_en_key_sets_identical_and_stringid_covered() {
        assert!(validate_key_sets().is_ok());
    }

    #[test]
    fn missing_key_detect() {
        let i18n = I18n::default();
        assert_eq!(i18n.t_key("nonexistent-key"), "nonexistent-key");
    }

    #[test]
    fn runtime_switch_works() {
        let mut i18n = I18n::default();
        assert_eq!(i18n.t(StringId::Save), "Save");
        i18n.set_lang(Lang::ZhCn);
        assert_eq!(i18n.t(StringId::Save), "保存");
    }
}