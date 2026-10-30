use serde::{Deserialize, Serialize};
use crate::home::model::CanvasState;

pub const CANVAS_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CanvasPersist {
    pub schema_version: u32,
    #[serde(default)]
    pub canvas: Option<CanvasState>,
}

impl CanvasPersist {
    pub fn new(canvas: CanvasState) -> Self {
        Self {
            schema_version: CANVAS_SCHEMA_VERSION,
            canvas: Some(canvas),
        }
    }


    /// Export versioned canvas JSON (no Markdown body, no Pi transcript, no secrets).
    pub fn export_canvas(state: &CanvasState) -> Result<String, String> {
        let mut clean = state.clone();
        for w in &mut clean.widgets {
            if let Some(obj) = w.options.as_object_mut() {
                obj.remove("body");
                obj.remove("pi_transcript");
                obj.remove("secret");
                obj.remove("password");
            }
        }
        let wrapped = serde_json::json!({
            "canvas_schema_version": CANVAS_SCHEMA_VERSION,
            "canvas": clean
        });
        serde_json::to_string_pretty(&wrapped).map_err(|e| e.to_string())
    }
    /// On any fail: return Err, caller MUST keep original layout.
    pub fn import_canvas(json: &str) -> Result<CanvasState, String> {
        #[derive(Deserialize)]
        struct ImportWrap {
            #[serde(default)]
            canvas_schema_version: Option<u32>,
            canvas: Option<CanvasState>,
        }
        let v: ImportWrap = serde_json::from_str(json).map_err(|e| format!("parse error: {}", e))?;

        if let Some(ver) = v.canvas_schema_version {
            if ver != CANVAS_SCHEMA_VERSION {
                return Err(format!("unsupported canvas version {}", ver));
            }
        }

        let state = v.canvas.unwrap_or_default();

        if !state.validate() {
            return Err("canvas validate() failed (zoom/pan/rects)".into());
        }

        if state.widgets.len() > 200 {
            return Err("widget count exceeds limit (200)".into());
        }

        let mut seen = std::collections::HashSet::new();
        for w in &state.widgets {
            if !seen.insert(w.id) {
                return Err("duplicate widget UUID".into());
            }
            if !crate::home::is_valid_kind(&w.kind) {
                return Err(format!("unknown builtin kind: {}", w.kind));
            }
            if !w.rect.origin.x.is_finite()
                || !w.rect.origin.y.is_finite()
                || !w.rect.size.w.is_finite()
                || !w.rect.size.h.is_finite()
                || w.rect.size.w < 10.0
                || w.rect.size.h < 10.0
            {
                return Err("non-finite or undersized rect".into());
            }
            if let Some(obj) = w.options.as_object() {
                if obj.contains_key("secret") || obj.contains_key("password") {
                    return Err("options must not contain secrets".into());
                }
            }
        }
        Ok(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::home::model::CanvasState;
    use crate::home::Widget;
    use crate::home::WorldRect;

    #[test]
    fn roundtrip_default() {
        let p = CanvasPersist::new(CanvasState::new());
        let json = serde_json::to_string(&p).unwrap();
        let back: CanvasPersist = serde_json::from_str(&json).unwrap();
        assert_eq!(back.schema_version, CANVAS_SCHEMA_VERSION);
    }

    #[test]
    fn export_has_no_body_or_creds() {
        let mut state = CanvasState::new();
        let mut w = Widget::new("tasks", WorldRect::new(0.,0.,100.,50.));
        w.options = serde_json::json!({"body": "secret markdown", "pi_transcript": ["hi"]});
        state.add_widget(w);
        let exported = CanvasPersist::export_canvas(&state).unwrap();
        assert!(!exported.contains("secret markdown"));
        assert!(!exported.contains("pi_transcript"));
    }

    #[test]
    fn import_rejects_unknown_version() {
        let bad = r#"{"canvas_schema_version": 99, "canvas": {"version":1,"pan":[0,0],"zoom":1,"widgets":[]}}"#;
        assert!(CanvasPersist::import_canvas(bad).is_err());
    }

    #[test]
    fn import_rejects_unknown_kind() {
        let bad = r#"{"canvas_schema_version":1,"canvas":{"version":1,"pan":[0,0],"zoom":1,"widgets":[{"id":"00000000-0000-0000-0000-000000000001","kind":"calendar","rect":{"origin":{"x":0,"y":0},"size":{"w":100,"h":50}},"z":0,"options":{}}]}}"#;
        assert!(CanvasPersist::import_canvas(bad).is_err());
    }

    #[test]
    fn import_rejects_dup_id() {
        let bad = r#"{"canvas_schema_version":1,"canvas":{"version":1,"pan":[0,0],"zoom":1,"widgets":[{"id":"00000000-0000-0000-0000-000000000001","kind":"tasks","rect":{"origin":{"x":0,"y":0},"size":{"w":100,"h":50}},"z":0,"options":{}},{"id":"00000000-0000-0000-0000-000000000001","kind":"pi","rect":{"origin":{"x":120,"y":0},"size":{"w":100,"h":50}},"z":1,"options":{}}]}}"#;
        assert!(CanvasPersist::import_canvas(bad).is_err());
    }

    #[test]
    fn import_rejects_nan_or_bad_rect() {
        let bad = r#"{"canvas_schema_version":1,"canvas":{"version":1,"pan":[0,0],"zoom":1,"widgets":[{"id":"00000000-0000-0000-0000-000000000001","kind":"atlas","rect":{"origin":{"x":0,"y":0},"size":{"w":NaN,"h":50}},"z":0,"options":{}}]}}"#;
        assert!(CanvasPersist::import_canvas(bad).is_err());
    }

    #[test]
    fn import_rejects_over_limit() {
        let mut state = CanvasState::new();
        for i in 0..201u32 {
            let id = uuid::Uuid::parse_str(&format!("00000000-0000-0000-0000-{:012x}", i)).unwrap();
            let mut w = Widget::new("tasks", WorldRect::new(i as f64 * 10.0, 0., 50., 30.));
            w.id = id;
            state.widgets.push(w);
        }
        let json = CanvasPersist::export_canvas(&state).unwrap();
        assert!(CanvasPersist::import_canvas(&json).is_err());
    }

    #[test]
    fn import_success_keeps_options_clean() {
        let good = r#"{"canvas_schema_version":1,"canvas":{"version":1,"pan":[10,20],"zoom":0.8,"widgets":[{"id":"00000000-0000-0000-0000-0000000000aa","kind":"recent","rect":{"origin":{"x":10,"y":20},"size":{"w":200,"h":100}},"z":5,"options":{"paths":["/a.md"]}}]}}"#;
        let imported = CanvasPersist::import_canvas(good).unwrap();
        assert_eq!(imported.pan, [10.0, 20.0]);
        assert_eq!(imported.zoom, 0.8);
        assert_eq!(imported.widgets[0].kind, "recent");
    }
}
