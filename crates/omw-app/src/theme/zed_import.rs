//! Zed family JSON import adapter. Manual only.
//! Limits: size < 2MiB, depth < 8, members < 64. Unknown ignored with diag.
//! Version aware: known fields only; deterministic fallbacks to builtin light/dark.

use std::path::Path;
use serde::Deserialize;
use gpui_kit::{hsla, Hsla};
use crate::theme::tokens::{SemanticTheme, TokenSet, ThemeMode};
use crate::theme::contrast::{ensure_contrast, ContrastFix};

const MAX_JSON_BYTES: usize = 2 * 1024 * 1024;
const MAX_DEPTH: usize = 8;
const MAX_MEMBERS: usize = 64;

fn json_depth(v: &serde_json::Value) -> usize {
    match v {
        serde_json::Value::Object(o) => 1 + o.values().map(json_depth).max().unwrap_or(0),
        serde_json::Value::Array(a) => 1 + a.iter().map(json_depth).max().unwrap_or(0),
        _ => 0,
    }
}

#[derive(Debug, Clone)]
pub struct ImportedTheme {
    pub semantic: SemanticTheme,
    pub original_json: Vec<u8>,
    pub meta: ImportMeta,
    pub fixes: Vec<ContrastFix>,
}

#[derive(Debug, Clone)]
pub struct ImportMeta {
    pub family: String,
    pub member: String,
    pub schema: Option<String>,
    pub hash: String,
}

#[derive(Deserialize, Default)]
struct ZedStyle {
    background: Option<String>,
    foreground: Option<String>,
    border: Option<String>,
    selection: Option<String>,
    #[serde(rename = "panel.background")]
    panel_background: Option<String>,
    #[serde(rename = "editor.background")]
    editor_background: Option<String>,
    text: Option<String>,
    accent: Option<String>,
}

#[derive(Deserialize)]
struct ZedThemeMember {
    name: Option<String>,
    appearance: Option<String>,
    style: Option<ZedStyle>,
}

#[derive(Deserialize)]
struct ZedThemeFamily {
    name: Option<String>,
    schema: Option<String>,
    themes: Option<Vec<ZedThemeMember>>,
}

fn parse_hex_color(s: &str) -> Result<Hsla, String> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 3 && s.len() != 6 && s.len() != 8 {
        return Err(format!("bad color len: {}", s));
    }
    let expand = |c: char| -> Result<u8, String> {
        let v = c.to_digit(16).ok_or_else(|| "bad hex".to_string())?;
        Ok(v as u8 * 17)
    };
    let (r, g, b, a) = if s.len() == 3 {
        (expand(s.as_bytes()[0] as char)?, expand(s.as_bytes()[1] as char)?, expand(s.as_bytes()[2] as char)?, 255u8)
    } else if s.len() == 6 {
        let r = u8::from_str_radix(&s[0..2], 16).map_err(|_| "hex r")?;
        let g = u8::from_str_radix(&s[2..4], 16).map_err(|_| "hex g")?;
        let b = u8::from_str_radix(&s[4..6], 16).map_err(|_| "hex b")?;
        (r, g, b, 255)
    } else {
        let r = u8::from_str_radix(&s[0..2], 16).map_err(|_| "hex r")?;
        let g = u8::from_str_radix(&s[2..4], 16).map_err(|_| "hex g")?;
        let b = u8::from_str_radix(&s[4..6], 16).map_err(|_| "hex b")?;
        let a = u8::from_str_radix(&s[6..8], 16).map_err(|_| "hex a")?;
        (r, g, b, a)
    };
    let rf = r as f32 / 255.0;
    let gf = g as f32 / 255.0;
    let bf = b as f32 / 255.0;
    let af = a as f32 / 255.0;
    let max = rf.max(gf).max(bf);
    let min = rf.min(gf).min(bf);
    let l = (max + min) / 2.0;
    let s = if max == min { 0.0 } else {
        let d = max - min;
        if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) }
    };
    let h = if max == min {
        0.0
    } else if max == rf {
        ((gf - bf) / (max - min) + if gf < bf { 6.0 } else { 0.0 }) / 6.0
    } else if max == gf {
        ((bf - rf) / (max - min) + 2.0) / 6.0
    } else {
        ((rf - gf) / (max - min) + 4.0) / 6.0
    };
    Ok(hsla(h, s, l, af))
}

fn pick_member<'a>(members: &'a [ZedThemeMember]) -> Option<&'a ZedThemeMember> {
    members.iter().find(|m| m.appearance.as_deref() == Some("light"))
        .or_else(|| members.iter().find(|m| m.appearance.as_deref() == Some("dark")))
        .or_else(|| members.first())
}

fn build_tokens_from_style(style: &ZedStyle, mode: ThemeMode) -> TokenSet {
    let mut t = match mode {
        ThemeMode::Light => TokenSet::light_default(),
        ThemeMode::Dark => TokenSet::dark_default(),
    };
    if let Some(bg) = &style.background {
        if let Ok(c) = parse_hex_color(bg) {
            t.background = c;
            t.editor_bg = c;
        }
    }
    if let Some(fg) = &style.foreground {
        if let Ok(c) = parse_hex_color(fg) {
            t.foreground = c;
            t.text_primary = c;
        }
    }
    if let Some(b) = &style.border {
        if let Ok(c) = parse_hex_color(b) { t.border = c; }
    }
    if let Some(s) = &style.selection {
        if let Ok(c) = parse_hex_color(s) { t.selection = c; }
    }
    if let Some(p) = &style.panel_background {
        if let Ok(c) = parse_hex_color(p) { t.panel = c; }
    }
    if let Some(e) = &style.editor_background {
        if let Ok(c) = parse_hex_color(e) { t.editor_bg = c; }
    }
    if let Some(txt) = &style.text {
        if let Ok(c) = parse_hex_color(txt) { t.text_primary = c; }
    }
    t
}

pub fn import_zed_family(json: &[u8], _source_path: &Path) -> Result<ImportedTheme, String> {
    if json.len() > MAX_JSON_BYTES {
        return Err("theme json too large".into());
    }
    // real depth, not brace heuristic
    let depth = if let Ok(v) = serde_json::from_slice::<serde_json::Value>(json) {
        json_depth(&v)
    } else {
        0
    };
    if depth > MAX_DEPTH {
        return Err("theme json too nested".into());
    }
    let family: ZedThemeFamily = serde_json::from_slice(json)
        .map_err(|e| format!("parse zed family: {}", e))?;

    let members = family.themes.as_deref().unwrap_or(&[]);
    if members.len() > MAX_MEMBERS {
        return Err("too many theme members".into());
    }
    let chosen = pick_member(members);
    let appearance = chosen.and_then(|m| m.appearance.clone()).unwrap_or_else(|| "light".into());
    let mode = if appearance == "dark" { ThemeMode::Dark } else { ThemeMode::Light };

    let tokens = if let Some(m) = chosen {
        if let Some(st) = &m.style {
            build_tokens_from_style(st, mode)
        } else {
            match mode { ThemeMode::Light => TokenSet::light_default(), _ => TokenSet::dark_default() }
        }
    } else {
        TokenSet::light_default()
    };

    let mut semantic = SemanticTheme { version: 1, mode, tokens };
    let (fixed, fixes) = ensure_contrast(semantic.clone());
    semantic = fixed;

    let family_name = family.name.clone().unwrap_or_else(|| "unknown".into());
    let member_name = chosen.and_then(|m| m.name.clone()).unwrap_or_else(|| appearance.clone());

    let hbytes = blake3::hash(json).as_bytes()[0..8].to_vec();
    let hash = hbytes.iter().map(|b| format!("{:02x}", b)).collect::<String>();

    let meta = ImportMeta {
        family: family_name,
        member: member_name,
        schema: family.schema.clone(),
        hash,
    };

    Ok(ImportedTheme {
        semantic,
        original_json: json.to_vec(),
        meta,
        fixes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn missing_fields_fallback_to_builtin() {
        let json = br#"{"name":"empty","themes":[{"name":"l","appearance":"light"}]}"#;
        let imp = import_zed_family(json, Path::new("t.json")).expect("import");
        assert_eq!(imp.meta.family, "empty");
        assert!(imp.semantic.tokens.background.l > 0.9);
    }

    #[test]
    fn unknown_fields_ignored() {
        let json = br##"{"name":"u","themes":[{"name":"d","appearance":"dark","style":{"background":"#111","weird_key":"xx","future":42}}]}"##;
        let imp = import_zed_family(json, Path::new("t.json")).expect("import");
        assert_eq!(imp.semantic.mode, ThemeMode::Dark);
        assert!(imp.semantic.tokens.background.l < 0.3);
    }

    #[test]
    fn illegal_color_keeps_fallback() {
        let json = br##"{"name":"bad","themes":[{"name":"l","appearance":"light","style":{"background":"notacolor","foreground":"#222"}}]}"##;
        let imp = import_zed_family(json, Path::new("t.json")).expect("import");
        assert!(imp.semantic.tokens.background.l > 0.9);
        assert!(imp.semantic.tokens.foreground.l < 0.5);
    }

    #[test]
    fn name_conflict_and_reimport_same_hash() {
        let json = br##"{"name":"rain","themes":[{"name":"light","appearance":"light","style":{"background":"#fafafa"}}]}"##;
        let i1 = import_zed_family(json, Path::new("a.json")).unwrap();
        let i2 = import_zed_family(json, Path::new("b.json")).unwrap();
        assert_eq!(i1.meta.hash, i2.meta.hash);
        assert_eq!(i1.meta.family, "rain");
    }

    // Rainbow light/dark excerpt (small test fixture only, under test-only path)
    // Adapted from https://github.com/athxx/zed-theme-rainbow under MIT license.
    // This is a minimal excerpt for unit test verification only.
    const RAINBOW_LIGHT: &[u8] = b"{\"name\":\"Rainbow\",\"schema\":\"v0.2\",\"themes\":[{\"name\":\"light\",\"appearance\":\"light\",\"style\":{\"background\":\"#f5f5f5\",\"foreground\":\"#222\",\"panel.background\":\"#eee\",\"editor.background\":\"#fff\",\"text\":\"#222\",\"selection\":\"#b3d9ff\",\"border\":\"#ddd\",\"error\":\"#c00\",\"warning\":\"#e80\"}}]}";
    const RAINBOW_DARK: &[u8] = b"{\"name\":\"Rainbow\",\"schema\":\"v0.2\",\"themes\":[{\"name\":\"dark\",\"appearance\":\"dark\",\"style\":{\"background\":\"#1a1a1a\",\"foreground\":\"#eee\",\"panel.background\":\"#222\",\"editor.background\":\"#111\",\"text\":\"#eee\",\"selection\":\"#334455\",\"border\":\"#333\",\"error\":\"#f66\",\"warning\":\"#fc0\"}}]}";

    #[test]
    fn rainbow_light_dark_imports_with_contrast() {
        let i1 = import_zed_family(RAINBOW_LIGHT, Path::new("rain.json")).unwrap();
        assert_eq!(i1.meta.family, "Rainbow");
        assert_eq!(i1.meta.member, "light");
        assert!(i1.semantic.tokens.background.l > 0.9);
        // after ensure contrast, ratios good
        assert!(crate::theme::contrast::contrast_ratio(i1.semantic.tokens.foreground, i1.semantic.tokens.background) >= 4.5);
        let i2 = import_zed_family(RAINBOW_DARK, Path::new("rain.json")).unwrap();
        assert_eq!(i2.meta.member, "dark");
        assert!(i2.semantic.tokens.background.l < 0.2);
    }

    #[test]
    fn future_schema_still_imports() {
        let json = b"{\"name\":\"fut\",\"schema\":\"v99.9\",\"themes\":[{\"name\":\"l\",\"appearance\":\"light\",\"style\":{\"background\":\"#fff\"}}]}";
        let imp = import_zed_family(json, Path::new("f.json")).expect("future ok");
        assert_eq!(imp.meta.schema, Some("v99.9".into()));
        assert!(imp.semantic.tokens.background.l > 0.9);
    }

    #[test]
    fn corrupt_json_fails() {
        let bad = b"not json {";
        assert!(import_zed_family(bad, Path::new("c.json")).is_err());
    }

    #[test]
    fn reimport_same_source_keeps_stable_id() {
        let json = RAINBOW_LIGHT;
        let i1 = import_zed_family(json, Path::new("r1")).unwrap();
        let i2 = import_zed_family(json, Path::new("r2")).unwrap();
        assert_eq!(i1.meta.hash, i2.meta.hash);
        assert_eq!(i1.meta.family, i2.meta.family);
    }

}
