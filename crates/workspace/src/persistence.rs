use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

use project::{AppDataPaths, atomic_write, portable};
use serde::{Deserialize, Serialize};

use crate::home::{CanvasPersist, CanvasState};

const APP_UI_SCHEMA_VERSION: u32 = 1;

/// Dock state stored without depending on GPUI's DockArea representation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PersistedDockState {
    pub schema_version: u32,
    pub kit_layout_version: u32,
    pub dock: serde_json::Value,
}

/// Versioned top-level UI state persisted to ui-state.json.
/// The serialized fields intentionally match the existing schema.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct AppUiState {
    schema_version: u32,
    dock: Option<PersistedDockState>,
    canvas: Option<serde_json::Value>,
    #[serde(default)]
    portable: bool,
}

impl Default for AppUiState {
    fn default() -> Self {
        Self {
            schema_version: APP_UI_SCHEMA_VERSION,
            dock: None,
            canvas: None,
            portable: false,
        }
    }
}


pub struct UiStateSnapshot {
    pub dock: Option<PersistedDockState>,
    pub canvas: Option<CanvasState>,
}

/// Load the persisted Dock and Canvas sections without allowing UI state types into this module.
fn decode_canvas_section(value: serde_json::Value) -> Result<CanvasState, String> {
    let persisted: CanvasPersist = serde_json::from_value(value)
        .map_err(|error| format!("invalid canvas section: {error}"))?;
    if persisted.schema_version != 1 {
        return Err(format!("unsupported canvas schema version {}", persisted.schema_version));
    }
    let import_json = serde_json::json!({
        "canvas_schema_version": persisted.schema_version,
        "canvas": persisted.canvas,
    });
    let import_json = serde_json::to_string(&import_json).map_err(|error| error.to_string())?;
    CanvasPersist::import_canvas(&import_json)
}
pub fn load_ui_state(paths: &AppDataPaths, root: &Path) -> Result<UiStateSnapshot, String> {
    let state = load_state(paths, root)?;
    let canvas = state.canvas.map(decode_canvas_section).transpose()?;
    Ok(UiStateSnapshot {
        dock: state.dock,
        canvas,
    })
}
/// Cloneable handle to a single sequential UI-state writer.
#[derive(Clone)]
pub struct UiStateWriter {
    sender: Sender<PersistChange>,
}

enum PersistChange {
    Dock(PersistedDockState),
    Canvas(CanvasState),
    Flush(Sender<Result<(), String>>),
}

impl UiStateWriter {
    pub fn new(paths: AppDataPaths, root: PathBuf) -> Result<Self, String> {
        let (sender, receiver) = mpsc::channel();
        thread::Builder::new()
            .name("ohmywork-ui-state".into())
            .spawn(move || writer_loop(paths, root, receiver))
            .map_err(|error| error.to_string())?;
        Ok(Self { sender })
    }

    pub fn write_dock(&self, dock: PersistedDockState) -> Result<(), String> {
        self.sender
            .send(PersistChange::Dock(dock))
            .map_err(|error| error.to_string())
    }

    pub fn write_canvas(&self, canvas: CanvasState) -> Result<(), String> {
        self.sender
            .send(PersistChange::Canvas(canvas))
            .map_err(|error| error.to_string())
    }

    /// Enqueue a barrier and receive the result of all writes preceding it.
    pub fn flush(&self) -> Result<Receiver<Result<(), String>>, String> {
        let (reply, result) = mpsc::channel();
        self.sender
            .send(PersistChange::Flush(reply))
            .map_err(|error| error.to_string())?;
        Ok(result)
    }
}

fn writer_loop(paths: AppDataPaths, root: PathBuf, receiver: Receiver<PersistChange>) {
    let mut last_result = Ok(());
    while let Ok(change) = receiver.recv() {
        match change {
            PersistChange::Flush(reply) => {
                let _ = reply.send(last_result.clone());
            }
            PersistChange::Dock(dock) => {
                last_result = match load_state(&paths, &root) {
                    Ok(mut state) => {
                        state.dock = Some(dock);
                        save_state(&paths, &root, &state)
                    }
                    Err(error) => Err(error),
                };
            }
            PersistChange::Canvas(canvas) => {
                last_result = match load_state(&paths, &root) {
                    Ok(mut state) => match CanvasPersist::export_canvas(&canvas)
                        .and_then(|json| CanvasPersist::import_canvas(&json))
                        .and_then(|canvas| serde_json::to_value(CanvasPersist::new(canvas)).map_err(|error| error.to_string()))
                    {
                        Ok(canvas) => {
                            state.canvas = Some(canvas);
                            save_state(&paths, &root, &state)
                        }
                        Err(error) => Err(error),
                    },
                    Err(error) => Err(error),
                };
            }
        }
    }
}

fn load_state(paths: &AppDataPaths, root: &Path) -> Result<AppUiState, String> {
    let path = portable::resolve_ui_state_path(paths, root);
    let mut state = match std::fs::read(&path) {
        Ok(bytes) => {
            let state: AppUiState = serde_json::from_slice(&bytes)
                .map_err(|error| format!("read UI state {}: invalid JSON: {error}", path.display()))?;
            if state.schema_version != APP_UI_SCHEMA_VERSION {
                return Err(format!(
                    "read UI state {}: unsupported schema version {}",
                    path.display(),
                    state.schema_version
                ));
            }
            if let Some(dock) = &state.dock {
                if dock.schema_version != 1 || dock.kit_layout_version != 1 {
                    return Err(format!(
                        "read UI state {}: unsupported dock schema/layout version {}/{}",
                        path.display(),
                        dock.schema_version,
                        dock.kit_layout_version
                    ));
                }
            }
            if let Some(canvas) = &state.canvas {
                decode_canvas_section(canvas.clone())
                    .map_err(|error| format!("read UI state {}: {error}", path.display()))?;
            }
            state
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => AppUiState::default(),
        Err(error) => {
            return Err(format!("read UI state {}: {error}", path.display()));
        }
    };

    if portable::is_portable_mode(root) {
        if let Some(dock) = &mut state.dock {
            portable::absolutize_paths(&mut dock.dock, root);
        }
        if let Some(canvas) = &mut state.canvas {
            portable::absolutize_paths(canvas, root);
        }
    }
    Ok(state)
}

fn save_state(paths: &AppDataPaths, root: &Path, state: &AppUiState) -> Result<(), String> {
    let path = portable::resolve_ui_state_path(paths, root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }

    let mut stored = state.clone();
    if portable::is_portable_mode(root) {
        stored.portable = true;
        if let Some(dock) = &mut stored.dock {
            portable::relativize_paths(&mut dock.dock, root);
        }
        if let Some(canvas) = &mut stored.canvas {
            portable::relativize_paths(canvas, root);
        }
    } else {
        stored.portable = false;
    }

    let bytes = serde_json::to_vec_pretty(&stored).map_err(|error| error.to_string())?;
    let expected = project::read_version(&path)
        .map_err(|error| error.to_string())?;
    atomic_write(&path, expected.as_ref(), &bytes, 0)
        .map(|_| ())
        .map_err(|error| error.to_string())
}
#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn temp_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("ohmywork-ui-state-{label}-{}", Uuid::new_v4()))
    }

    fn writer(paths: &AppDataPaths, root: &Path) -> UiStateWriter {
        UiStateWriter::new(paths.clone(), root.to_path_buf()).unwrap()
    }

    fn flush(writer: &UiStateWriter) -> Result<(), String> {
        writer.flush().unwrap().recv().unwrap()
    }

    #[test]
    fn dock_and_canvas_writes_preserve_each_other_in_both_orders() {
        let base = temp_dir("sections");
        let paths = AppDataPaths::from_base(base.join("app")).unwrap();
        let root = base.join("workspace");
        std::fs::create_dir_all(&root).unwrap();
        let writer = writer(&paths, &root);
        let dock = PersistedDockState {
            schema_version: 1,
            kit_layout_version: 1,
            dock: serde_json::json!({"selected": "home"}),
        };
        let canvas = CanvasState::new();

        writer.write_dock(dock.clone()).unwrap();
        writer.write_canvas(canvas.clone()).unwrap();
        flush(&writer).unwrap();
        writer.write_canvas(canvas.clone()).unwrap();
        writer.write_dock(dock.clone()).unwrap();
        flush(&writer).unwrap();

        let loaded = load_ui_state(&paths, &root).unwrap();
        assert_eq!(loaded.dock.unwrap().dock, dock.dock);
        assert_eq!(loaded.canvas, Some(canvas));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn malformed_state_is_retained_and_flush_reports_error() {
        let base = temp_dir("malformed");
        let paths = AppDataPaths::from_base(base.join("app")).unwrap();
        let root = base.join("workspace");
        std::fs::create_dir_all(&root).unwrap();
        let path = portable::resolve_ui_state_path(&paths, &root);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let original = b"not json {";
        std::fs::write(&path, original).unwrap();

        let writer = writer(&paths, &root);
        writer.write_canvas(CanvasState::new()).unwrap();
        let error = flush(&writer).unwrap_err();
        assert!(error.contains("invalid JSON"));
        assert_eq!(std::fs::read(&path).unwrap(), original);
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn invalid_canvas_is_retained_and_rejects_other_section_writes() {
        let base = temp_dir("invalid-canvas");
        let paths = AppDataPaths::from_base(base.join("app")).unwrap();
        let root = base.join("workspace");
        std::fs::create_dir_all(&root).unwrap();
        let path = portable::resolve_ui_state_path(&paths, &root);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut canvas = CanvasState::new();
        canvas.widgets.push(crate::home::Widget::new(
            "unknown-kind",
            crate::home::WorldRect::new(0.0, 0.0, 100.0, 100.0),
        ));
        let mut state = AppUiState::default();
        state.canvas = Some(serde_json::to_value(CanvasPersist::new(canvas)).unwrap());
        let original = serde_json::to_vec_pretty(&state).unwrap();
        std::fs::write(&path, &original).unwrap();

        let writer = writer(&paths, &root);
        writer.write_dock(PersistedDockState {
            schema_version: 1,
            kit_layout_version: 1,
            dock: serde_json::json!({"selected": "home"}),
        }).unwrap();
        assert!(flush(&writer).unwrap_err().contains("unknown builtin kind"));
        assert_eq!(std::fs::read(&path).unwrap(), original);
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn unsupported_state_schema_is_retained_and_portable_paths_round_trip() {
        let base = temp_dir("portable");
        let paths = AppDataPaths::from_base(base.join("app")).unwrap();
        let root = base.join("workspace");
        std::fs::create_dir_all(root.join(".ohmywork")).unwrap();
        std::fs::write(root.join(".ohmywork/portable.json"), b"{\"schema_version\":1}").unwrap();
        let path = portable::resolve_ui_state_path(&paths, &root);
        let unsupported = br#"{"schema_version":99,"dock":null,"canvas":null}"#;
        std::fs::write(&path, unsupported).unwrap();
        let writer = writer(&paths, &root);
        writer.write_canvas(CanvasState::new()).unwrap();
        assert!(flush(&writer).unwrap_err().contains("unsupported schema version"));
        assert_eq!(std::fs::read(&path).unwrap(), unsupported);

        std::fs::remove_file(&path).unwrap();
        let absolute = root.join("Atlas").join("note.md");
        let absolute_value = absolute.to_string_lossy().into_owned();
        writer.write_dock(PersistedDockState {
            schema_version: 1,
            kit_layout_version: 1,
            dock: serde_json::json!({"path": absolute_value}),
        }).unwrap();
        flush(&writer).unwrap();
        let stored = std::fs::read_to_string(&path).unwrap();
        assert!(!stored.contains(root.to_string_lossy().as_ref()));
        assert!(stored.contains("note.md"));
        let loaded = load_ui_state(&paths, &root).unwrap();
        let restored = PathBuf::from(loaded.dock.unwrap().dock["path"].as_str().unwrap());
        assert_eq!(restored, absolute);
        let _ = std::fs::remove_dir_all(base);
    }
}
