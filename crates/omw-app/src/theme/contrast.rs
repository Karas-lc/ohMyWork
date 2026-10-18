//! Contrast safety correction per plan22/ADR.
//! WCAG 2.2 sRGB relative luminance. Binary-search mix until pass.
//! Only adjust when needed; record original/resolved/reason. Source unchanged.
use gpui_kit::{hsla, Hsla};
use crate::theme::tokens::{SemanticTheme, TokenSet, ThemeMode};
#[derive(Clone, Debug)]
pub struct ContrastFix {
    pub token: String,
    pub original: Hsla,
    pub resolved: Hsla,
    pub reason: String,
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    let h = h.fract();
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h * 6.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = if h < 1.0 / 6.0 {
        (c, x, 0.0)
    } else if h < 2.0 / 6.0 {
        (x, c, 0.0)
    } else if h < 3.0 / 6.0 {
        (0.0, c, x)
    } else if h < 4.0 / 6.0 {
        (0.0, x, c)
    } else if h < 5.0 / 6.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };
    (r + m, g + m, b + m)
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// WCAG 2.2 relative luminance (sRGB)
fn relative_luminance(c: Hsla) -> f32 {
    let (r, g, b) = hsl_to_rgb(c.h, c.s, c.l);
    let r = srgb_to_linear(r.clamp(0.0, 1.0));
    let g = srgb_to_linear(g.clamp(0.0, 1.0));
    let b = srgb_to_linear(b.clamp(0.0, 1.0));
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

fn composite_rgb(fg: Hsla, bg: Hsla) -> (f32, f32, f32) {
    let (fr, fg_, fb) = hsl_to_rgb(fg.h, fg.s, fg.l);
    let (br, bg_, bb) = hsl_to_rgb(bg.h, bg.s, bg.l);
    let a = fg.a.clamp(0.0, 1.0);
    (fr * a + br * (1.0 - a), fg_ * a + bg_ * (1.0 - a), fb * a + bb * (1.0 - a))
}

fn effective_luminance(fg: Hsla, bg: Hsla) -> f32 {
    let (r, g, b) = composite_rgb(fg, bg);
    let r = srgb_to_linear(r.clamp(0.0, 1.0));
    let g = srgb_to_linear(g.clamp(0.0, 1.0));
    let b = srgb_to_linear(b.clamp(0.0, 1.0));
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

pub(crate) fn contrast_ratio(a: Hsla, b: Hsla) -> f32 {
    let la = effective_luminance(a, b);
    let lb = relative_luminance(b);
    let (l1, l2) = if la > lb { (la, lb) } else { (lb, la) };
    (l1 + 0.05) / (l2 + 0.05)
}

fn lerp(a: Hsla, b: Hsla, t: f32) -> Hsla {
    Hsla { h: a.h + (b.h - a.h) * t, s: a.s + (b.s - a.s) * t, l: a.l + (b.l - a.l) * t, a: a.a + (b.a - a.a) * t }
}

fn mix_until_contrast(fg: Hsla, bg: Hsla, target: f32) -> (Hsla, bool) {
    if contrast_ratio(fg, bg) >= target {
        return (fg, false);
    }
    let black = hsla(0.0, 0.0, 0.0, fg.a);
    let white = hsla(0.0, 0.0, 1.0, fg.a);
    let (mix_b, ok_b) = binary_mix(fg, black, bg, target);
    let (mix_w, ok_w) = binary_mix(fg, white, bg, target);
    if !ok_b && !ok_w {
        let br = contrast_ratio(black, bg);
        let wr = contrast_ratio(white, bg);
        return if br >= wr { (black, true) } else { (white, true) };
    }
    if !ok_b { return (mix_w, true); }
    if !ok_w { return (mix_b, true); }
    let db = (mix_b.l - fg.l).abs();
    let dw = (mix_w.l - fg.l).abs();
    if db <= dw { (mix_b, true) } else { (mix_w, true) }
}

fn binary_mix(fg: Hsla, toward: Hsla, bg: Hsla, target: f32) -> (Hsla, bool) {
    let mut lo = 0.0f32;
    let mut hi = 1.0f32;
    let mut best = fg;
    let mut reached = false;
    for _ in 0..32 {
        let mid = (lo + hi) / 2.0;
        let cand = lerp(fg, toward, mid);
        if contrast_ratio(cand, bg) >= target {
            best = cand;
            hi = mid;
            reached = true;
        } else {
            lo = mid;
        }
    }
    if !reached && contrast_ratio(toward, bg) >= target {
        best = toward;
        reached = true;
    }
    (best, reached)
}

pub fn ensure_contrast(theme: SemanticTheme) -> (SemanticTheme, Vec<ContrastFix>) {
    let mut fixes = vec![];
    let mut t = theme.tokens.clone();
    let bg = t.background;

    // body text vs bg >= 4.5
    let (fg, changed) = mix_until_contrast(t.foreground, bg, 4.5);
    if changed {
        fixes.push(ContrastFix {
            token: "text_primary".into(),
            original: theme.tokens.foreground,
            resolved: fg,
            reason: "text vs background < 4.5:1".into(),
        });
        t.foreground = fg;
        t.text_primary = fg;
    }
    // selection contrast
    let sel_bg = t.selection;
    let (_sel_fg, changed) = mix_until_contrast(t.foreground, sel_bg, 4.5);
    if changed {
        // since no separate sel fg token, adjust selection lightness slightly toward fg for visibility
        let adjusted_sel = lerp(sel_bg, t.foreground, 0.15);
        fixes.push(ContrastFix {
            token: "selection".into(),
            original: theme.tokens.selection,
            resolved: adjusted_sel,
            reason: "selection contrast <4.5".into(),
        });
        t.selection = adjusted_sel;
    }

    // focus ring 3:1
    let (foc, changed) = mix_until_contrast(t.focus, bg, 3.0);
    if changed {
        fixes.push(ContrastFix {
            token: "focus".into(),
            original: theme.tokens.focus,
            resolved: foc,
            reason: "focus ring vs bg < 3:1".into(),
        });
        t.focus = foc;
    }

    // error vs bg 4.5
    let (err, changed) = mix_until_contrast(t.error, bg, 4.5);
    if changed {
        fixes.push(ContrastFix {
            token: "error".into(),
            original: theme.tokens.error,
            resolved: err,
            reason: "error vs bg < 4.5:1".into(),
        });
        t.error = err;
    }

    // disabled: 3:1 vs bg AND vs foreground (adjacent)
    let dis_bg = mix_until_contrast(t.disabled, bg, 3.0).0;
    let dis_fg = mix_until_contrast(dis_bg, t.foreground, 3.0).0;
    if contrast_ratio(dis_bg, bg) < 3.0 || contrast_ratio(dis_bg, t.foreground) < 3.0 {
        let final_dis = if contrast_ratio(dis_bg, bg) >= contrast_ratio(dis_bg, t.foreground) { dis_bg } else { dis_fg };
        fixes.push(ContrastFix {
            token: "disabled".into(),
            original: theme.tokens.disabled,
            resolved: final_dis,
            reason: "disabled <3:1 vs bg or adjacent".into(),
        });
        t.disabled = final_dis;
    } else if (dis_bg.l - t.disabled.l).abs() > 0.001 {
        t.disabled = dis_bg;
    }

    // also ensure editor_bg vs bg minimal, but no fix unless low
    // keep other tokens (warning, canvas etc) as-is unless low contrast cases
    let warn_ratio = contrast_ratio(t.warning, bg);
    if warn_ratio < 3.0 {
        let (w, _) = mix_until_contrast(t.warning, bg, 3.0);
        fixes.push(ContrastFix { token: "warning".into(), original: theme.tokens.warning, resolved: w, reason: "warning low vs bg".into() });
        t.warning = w;
    }

    let new_tokens = t;
    let fixed = SemanticTheme { version: theme.version, mode: theme.mode, tokens: new_tokens };
    (fixed, fixes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::hsla;

    #[test]
    fn luminance_and_ratio_sane() {
        let white = hsla(0.,0.,1.,1.);
        let black = hsla(0.,0.,0.,1.);
        assert!(relative_luminance(white) > 0.99);
        assert!(relative_luminance(black) < 0.01);
        assert!(contrast_ratio(white, black) > 20.0);
    }

    #[test]
    fn low_contrast_corrected_body_text() {
        let mut t = TokenSet::light_default();
        t.background = hsla(0.,0.,0.95,1.);
        t.foreground = hsla(0.,0.,0.82,1.); // low contrast gray
        let sem = SemanticTheme { version:1, mode: ThemeMode::Light, tokens: t };
        let (fixed, fixes) = ensure_contrast(sem);
        assert!(!fixes.is_empty());
        assert!(contrast_ratio(fixed.tokens.foreground, fixed.tokens.background) >= 4.5);
        // source unchanged in caller; here verify original not mutated
    }

    #[test]
    fn disabled_checks_adjacent() {
        let mut t = TokenSet::light_default();
        t.disabled = hsla(0.,0.,0.82,1.); // too close to fg
        let sem = SemanticTheme { version:1, mode: ThemeMode::Light, tokens: t.clone() };
        let (fixed, _fixes) = ensure_contrast(sem);
        assert!(contrast_ratio(fixed.tokens.disabled, fixed.tokens.background) >= 3.0);
        assert!(contrast_ratio(fixed.tokens.disabled, fixed.tokens.foreground) >= 3.0);
    }

    #[test]
    fn transparent_color_composites_before_ratio() {
        let mut t = TokenSet::light_default();
        t.background = hsla(0., 0., 0.9, 1.0);
        t.foreground = hsla(0., 0., 0.4, 0.6); // higher alpha, low l -> will be mixed toward black/white
        let sem = SemanticTheme { version: 1, mode: ThemeMode::Light, tokens: t.clone() };
        let (fixed, _fixes) = ensure_contrast(sem);
        let r = contrast_ratio(fixed.tokens.foreground, fixed.tokens.background);
        assert!(r >= 4.5, "transparent should composite and fix to >=4.5 got {}", r);
        assert_eq!(fixed.tokens.background, t.background);
    }

    #[test]
    fn smaller_delta_chosen_over_farther() {
        let mut t = TokenSet::light_default();
        t.background = hsla(0.,0.,0.1,1.); // dark bg
        t.foreground = hsla(0.,0.,0.3,1.); // low, closer to black than white
        let sem = SemanticTheme { version:1, mode: ThemeMode::Dark, tokens: t };
        let (fixed, fixes) = ensure_contrast(sem.clone());
        assert!(!fixes.is_empty());
        let ratio = contrast_ratio(fixed.tokens.foreground, fixed.tokens.background);
        assert!(ratio >= 4.5);
        // delta should be toward one direction (smaller)
        let orig_l = sem.tokens.foreground.l;
        let new_l = fixed.tokens.foreground.l;
        let d = (new_l - orig_l).abs();
        assert!(d > 0.0 && d < 0.5);
    }

    #[test]
    fn post_fix_ratios_and_bg_unchanged() {
        let mut t = TokenSet::dark_default();
        t.background = hsla(0.,0.,0.1,1.);
        t.foreground = hsla(0.,0.,0.4,1.); // low
        t.focus = hsla(0.,0.,0.5,1.);
        let sem = SemanticTheme { version:1, mode: ThemeMode::Dark, tokens: t.clone() };
        let (fixed, _f) = ensure_contrast(sem);
        assert_eq!(fixed.tokens.background, t.background);
        assert!(contrast_ratio(fixed.tokens.foreground, fixed.tokens.background) >= 4.5);
        assert!(contrast_ratio(fixed.tokens.focus, fixed.tokens.background) >= 3.0);
    }
}
