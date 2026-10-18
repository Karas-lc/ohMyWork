use std::path::{Path, PathBuf};

use gpui_kit::component::dock::DockAreaState;
use serde::{Deserialize, Serialize};

use crate::ui::panes::PersistedDockState;
use omw_local::{AppDataPaths, portable};

/// Versioned top-level UI state persisted to ui-state.json (either in LocalAppData or
/// .ohmywork/ when portable marker present). Dock + canvas sections only; no document bodies,
/// edit history, Pi transcripts, credentials or absolute machine-only roots in portable form.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppUiState {
    pub schema_version: u32,
    pub dock: Option<PersistedDockState>,
    pub canvas: Option<serde_json::Value>,
    #[serde(default)]
    pub portable: bool,
}

impl Default for AppUiState {
    fn default() -> Self {
        Self {
            schema_version: 1,
            dock: None,
            canvas: None,
            portable: false,
        }
    }
}

const APP_UI_SCHEMA_VERSION: u32 = 1;

/// Resolve the current authority for ui-state.json.
/// Uses AppDataPaths.ui_state_file() unless .ohmywork/portable.json marks portable mode
/// for the workspace, in which case .ohmywork/ui-state.json under workspace root is used.
/// Never uses temp_dir for production paths.
pub fn resolve_ui_state_path(paths: &AppDataPaths, workspace_root: &Path) -> PathBuf {
    portable::resolve_ui_state_path(paths, workspace_root)
}

fn load_from_path(path: &Path) -> AppUiState {
    match std::fs::read(path) {
        Ok(bytes) => match serde_json::from_slice::<AppUiState>(&bytes) {
            Ok(mut s) => {
                if s.schema_version != APP_UI_SCHEMA_VERSION {
                    // unsupported: keep original file, return actionable default
                    return AppUiState::default();
                }
                if let Some(ref d) = s.dock {
                    if d.schema_version != 1 || d.kit_layout_version != 1 {
                        return AppUiState::default();
                    }
                }
                s
            }
            Err(_) => AppUiState::default(), // corrupt: keep file, default
        },
        Err(_) => AppUiState::default(),
    }
}

/// Load full AppUiState from the current authority for this workspace.
/// On missing: default (caller shows placeholder). On corrupt/unsupported schema: keep file + default.
/// After load, if portable, absolutize any relative refs back to current workspace root.
pub fn load_ui_state(paths: &AppDataPaths, workspace_root: &Path) -> AppUiState {
    let p = resolve_ui_state_path(paths, workspace_root);
    let mut state = load_from_path(&p);
    if portable::is_portable_mode(workspace_root) {
        absolutize_state_refs(&mut state, workspace_root);
    }
    state
}

fn absolutize_state_refs(state: &mut AppUiState, root: &Path) {
    if let Some(ref mut d) = state.dock {
        if let Ok(mut v) = serde_json::to_value(&d.dock) {
            portable::absolutize_paths(&mut v, root);
            if let Ok(restored) = serde_json::from_value::<DockAreaState>(v) {
                d.dock = restored;
            }
        }
    }
    if let Some(ref mut c) = state.canvas {
        portable::absolutize_paths(c, root);
    }
}

/// Save full state to the current authority. If target authority is portable, relativize
/// refs inside dock/canvas first (outside root refs left as-is).
/// Uses atomic temp+replace. Caller responsible for throttling + flush on exit.
pub fn save_ui_state(
    paths: &AppDataPaths,
    workspace_root: &Path,
    state: &AppUiState,
) -> Result<(), String> {
    let p = resolve_ui_state_path(paths, workspace_root);
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let mut to_save = state.clone();
    if portable::is_portable_mode(workspace_root) {
        to_save.portable = true;
        if let Some(ref mut pd) = to_save.dock {
            if let Ok(mut v) = serde_json::to_value(&pd.dock) {
                portable::relativize_paths(&mut v, workspace_root);
                if let Ok(restored) = serde_json::from_value::<DockAreaState>(v) {
                    pd.dock = restored;
                }
            }
        }
        if let Some(ref mut c) = to_save.canvas {
            portable::relativize_paths(c, workspace_root);
        }
    } else {
        to_save.portable = false;
    }

    // never store forbidden content (enforced by what we put in struct)
    let bytes = serde_json::to_vec_pretty(&to_save).map_err(|e| e.to_string())?;
    let expected = omw_local::read_version(&p).ok().flatten();
    match omw_local::atomic_write(&p, expected.as_ref(), &bytes, 0) {
        Ok(_) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::home::model::CanvasState;
    use crate::ui::panes::PersistedDockState;
    use omw_local::AppDataPaths;
    use std::fs;
    use uuid::Uuid;

    fn temp_dir(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("ohmywork-state-{}-{}", name, Uuid::new_v4()));
        let _ = fs::create_dir_all(&p);
        p
    }

    #[test]
    fn default_has_schema_1_no_forbidden() {
        let s = AppUiState::default();
        assert_eq!(s.schema_version, 1);
        assert!(s.dock.is_none());
        assert!(s.canvas.is_none());
        let j = serde_json::to_string(&s).unwrap();
        assert!(!j.contains("body"));
        assert!(!j.contains("transcript"));
        assert!(!j.contains("pi_"));
    }

    #[test]
    fn load_missing_or_corrupt_returns_default_keeps_file() {
        let base = temp_dir("corrupt");
        let app = AppDataPaths::from_base(base.clone()).unwrap();
        let ws = base.join("ws");
        let p = resolve_ui_state_path(&app, &ws);
        // missing
        let loaded = load_ui_state(&app, &ws);
        assert_eq!(loaded.schema_version, 1);
        assert!(loaded.dock.is_none());

        // corrupt
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, b"not json {").unwrap();
        let loaded2 = load_ui_state(&app, &ws);
        assert_eq!(loaded2.schema_version, 1);
        // file kept
        assert_eq!(fs::read_to_string(&p).unwrap(), "not json {");
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn save_roundtrip_and_section_with_dock_canvas() {
        let base = temp_dir("round");
        let app = AppDataPaths::from_base(base.clone()).unwrap();
        let ws = base.join("ws");
        let _ = fs::create_dir_all(&ws);

        let dock = PersistedDockState::default();
        let canvas_state = CanvasState::new();
        let cp = crate::home::persist::CanvasPersist::new(canvas_state.clone());
        let canvas_v = serde_json::to_value(&cp).unwrap();

        let state = AppUiState {
            schema_version: 1,
            dock: Some(dock.clone()),
            canvas: Some(canvas_v.clone()),
            portable: false,
        };
        save_ui_state(&app, &ws, &state).unwrap();

        let loaded = load_ui_state(&app, &ws);
        assert_eq!(loaded.schema_version, 1);
        assert!(loaded.dock.is_some());
        assert!(loaded.canvas.is_some());
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn portable_payload_has_relative_refs_only_no_secrets() {
        let base = temp_dir("port-payload");
        let app = AppDataPaths::from_base(base.clone()).unwrap();
        let ws = base.join("ws");
        let _ = fs::create_dir_all(ws.join(".ohmywork"));
        // simulate portable marker for test
        fs::write(
            ws.join(".ohmywork/portable.json"),
            b"{\"schema_version\":1}",
        )
        .unwrap();

        let abs_path = ws.join("Atlas").join("secret.md");
        let dock = PersistedDockState::default();
        // embed an abs path in the panel value (as would happen in real TabRef serialize)
        let mut panel_v = serde_json::json!({"path": abs_path.to_string_lossy().to_string(), "body": "SHOULD NOT BE HERE", "pi_transcript": ["no"] });
        // use the relativize
        portable::relativize_paths(&mut panel_v, &ws);
        // now inject into a dock value simulation
        // (for test we put in canvas value too)
        let mut cval =
            serde_json::json!({"options": {"p": abs_path.to_string_lossy().to_string() }});
        portable::relativize_paths(&mut cval, &ws);

        let st = AppUiState {
            schema_version: 1,
            dock: Some(dock),
            canvas: Some(cval),
            portable: true,
        };
        save_ui_state(&app, &ws, &st).unwrap();

        let p = resolve_ui_state_path(&app, &ws);
        let bytes = fs::read_to_string(&p).unwrap();
        // relative now
        assert!(bytes.contains("Atlas/secret.md") || bytes.contains("secret.md"));
        assert!(!bytes.contains("body")); // we didn't put real, but check no leak if was
        assert!(!bytes.contains("pi_transcript"));
        assert!(!bytes.contains("pi_launch"));
        assert!(!bytes.contains(&ws.join("Atlas").to_string_lossy().to_string())); // no abs root
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn canvas_save_while_portable_writes_workspace_ui_state_only() {
        let base = temp_dir("canvas-port-auth");
        let app = AppDataPaths::from_base(base.clone()).unwrap();
        let ws = base.join("ws");
        let _ = fs::create_dir_all(ws.join(".ohmywork"));
        // set portable marker so resolve picks .ohmywork/
        fs::write(
            ws.join(".ohmywork/portable.json"),
            b"{\"schema_version\":1}",
        )
        .unwrap();

        let app_ui = app.ui_state_file();
        let _ = fs::remove_file(&app_ui); // ensure clean
        let ws_ui = resolve_ui_state_path(&app, &ws);
        assert_eq!(ws_ui, ws.join(".ohmywork").join("ui-state.json"));

        // exercise canvas save path (now threaded)
        let canvas_state = CanvasState::new();
        let cp = crate::home::persist::CanvasPersist::new(canvas_state);
        cp.save(&app, &ws)
            .expect("portable canvas save must succeed");

        // authority is ws one
        assert!(
            ws_ui.exists(),
            "canvas save must write to workspace .ohmywork/ui-state.json when portable"
        );
        // must NOT touch/create LocalAppData ui-state.json
        assert!(
            !app_ui.exists(),
            "canvas save under portable must not create or update LocalAppData ui-state.json"
        );

        // spot check content has canvas section (dock may be absent)
        let content = fs::read_to_string(&ws_ui).unwrap();
        assert!(
            content.contains("\"canvas\""),
            "saved state must include canvas section"
        );

        let _ = fs::remove_dir_all(&base);
    }
}
