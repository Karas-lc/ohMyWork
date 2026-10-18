//! Plan 23: portable mode switch for ui-state.json authority.
//! .ohmywork/portable.json is the marker (presence + valid schema).
//! .ohmywork/ui-state.json becomes sole authority for dock + canvas when enabled.
//! Converts absolute content refs (paths in dock panel values / canvas options) to workspace-relative on enable.
//! Absolute refs outside workspace root are left untouched (never pretend portable).
//! Never includes document body, edit history, Pi transcripts, credentials, Pi command, absolute machine roots in payload.
//! Switch: read current -> (optional convert) -> atomic write dest -> re-read+verify -> atomic marker update (or remove on disable).
//! Any failure before marker change: old authority kept; no dual-write.
//! Uses AppDataPaths.ui_state_file() as default non-portable authority.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::atomic_write::{atomic_write, AtomicWriteError, read_version};
use crate::{AppDataPaths, PathsError};

pub const PORTABLE_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortableMarker {
    pub schema_version: u32,
}

pub fn portable_marker_path(workspace_root: impl AsRef<Path>) -> PathBuf {
    workspace_root.as_ref().join(".ohmywork").join("portable.json")
}

pub fn workspace_ui_state_path(workspace_root: impl AsRef<Path>) -> PathBuf {
    workspace_root.as_ref().join(".ohmywork").join("ui-state.json")
}

pub fn is_portable_mode(workspace_root: impl AsRef<Path>) -> bool {
    let p = portable_marker_path(workspace_root);
    if !p.exists() {
        return false;
    }
    match fs::read(&p) {
        Ok(bytes) if !bytes.is_empty() => match serde_json::from_slice::<PortableMarker>(&bytes) {
            Ok(m) => m.schema_version == PORTABLE_SCHEMA_VERSION,
            Err(_) => false,
        },
        _ => false,
    }
}

pub fn resolve_ui_state_path(paths: &AppDataPaths, workspace_root: &Path) -> PathBuf {
    if is_portable_mode(workspace_root) {
        workspace_ui_state_path(workspace_root)
    } else {
        paths.ui_state_file()
    }
}

/// Recursively rewrite absolute paths under `root` to relative (using / for portability).
/// Leaves strings that are not absolute or outside root unchanged.
pub fn relativize_paths(value: &mut serde_json::Value, workspace_root: &Path) {
    match value {
        serde_json::Value::String(s) => {
            let candidate = Path::new(s.as_str());
            if candidate.is_absolute() {
                if let Ok(rel) = candidate.strip_prefix(workspace_root) {
                    let rel_str = rel.to_string_lossy().replace('\\', "/");
                    *s = rel_str;
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr.iter_mut() {
                relativize_paths(item, workspace_root);
            }
        }
        serde_json::Value::Object(map) => {
            for item in map.values_mut() {
                relativize_paths(item, workspace_root);
            }
        }
        _ => {}
    }
}

/// Recursively rewrite relative paths to absolute by joining to workspace_root.
pub fn absolutize_paths(value: &mut serde_json::Value, workspace_root: &Path) {
    match value {
        serde_json::Value::String(s) => {
            let candidate = Path::new(s.as_str());
            if !candidate.is_absolute() && !s.is_empty() {
                let abs = workspace_root.join(candidate);
                *s = abs.to_string_lossy().to_string();
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr.iter_mut() {
                absolutize_paths(item, workspace_root);
            }
        }
        serde_json::Value::Object(map) => {
            for item in map.values_mut() {
                absolutize_paths(item, workspace_root);
            }
        }
        _ => {}
    }
}

#[derive(Debug)]
pub enum PortableSwitchError {
    Paths(PathsError),
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
    UnsupportedSchema {
        path: PathBuf,
        version: u32,
    },
    Atomic(AtomicWriteError),
    VerifyFailed(String),
}

impl fmt::Display for PortableSwitchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Paths(error) => error.fmt(formatter),
            Self::Io {
                operation,
                path,
                source,
            } => write!(formatter, "{operation} {} 失败：{source}", path.display()),
            Self::Json { path, source } => {
                write!(formatter, "解析便携 ui-state {} 失败：{source}", path.display())
            }
            Self::UnsupportedSchema { path, version } => write!(
                formatter,
                "便携 ui-state {} 使用不支持的 schema 版本 {version}；原文件已保留",
                path.display()
            ),
            Self::Atomic(error) => write!(formatter, "便携原子写入失败：{error}"),
            Self::VerifyFailed(msg) => write!(formatter, "便携切换验证失败：{msg}"),
        }
    }
}

impl std::error::Error for PortableSwitchError {}

impl From<PathsError> for PortableSwitchError {
    fn from(error: PathsError) -> Self {
        Self::Paths(error)
    }
}

impl From<AtomicWriteError> for PortableSwitchError {
    fn from(error: AtomicWriteError) -> Self {
        Self::Atomic(error)
    }
}

/// Enable portable: read from current (appdata) authority, convert refs to relative,
/// atomic write to .ohmywork/ui-state.json (temp+replace), re-read verify schema,
/// then atomic write marker. Any step fail before marker: no change to mode/authority.
pub fn enable_portable(
    workspace_root: &Path,
    app_data: &AppDataPaths,
) -> Result<(), PortableSwitchError> {
    if is_portable_mode(workspace_root) {
        return Ok(());
    }
    let src = app_data.ui_state_file();
    let dst = workspace_ui_state_path(workspace_root);
    if let Some(parent) = dst.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            return Err(PortableSwitchError::Io {
                operation: "ensure portable ui-state dir",
                path: dst.clone(),
                source: e,
            });
        }
    }

    let content = match fs::read(&src) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(e) => {
            return Err(PortableSwitchError::Io {
                operation: "read current authority for portable enable",
                path: src,
                source: e,
            })
        }
    };

    let mut val: serde_json::Value = if content.is_empty() {
        serde_json::json!({
            "schema_version": 1,
            "dock": null,
            "canvas": null,
            "portable": false
        })
    } else {
        match serde_json::from_slice(&content) {
            Ok(v) => v,
            Err(e) => {
                return Err(PortableSwitchError::Json {
                    path: src,
                    source: e,
                })
            }
        }
    };

    if let Some(ver) = val.get("schema_version").and_then(|u| u.as_u64()) {
        if ver != 1 {
            return Err(PortableSwitchError::UnsupportedSchema {
                path: src,
                version: ver as u32,
            });
        }
    }

    relativize_paths(&mut val, workspace_root);
    val["portable"] = serde_json::json!(true);

    let converted = serde_json::to_vec_pretty(&val).map_err(|e| PortableSwitchError::Json {
        path: dst.clone(),
        source: e,
    })?;

    let expected = read_version(&dst).ok().flatten();
    atomic_write(&dst, expected.as_ref(), &converted, 0)?;
    // re-read + verify
    let reread = match fs::read(&dst) {
        Ok(b) => b,
        Err(e) => {
            return Err(PortableSwitchError::Io {
                operation: "re-read verify after write",
                path: dst,
                source: e,
            })
        }
    };
    let reread_val: serde_json::Value =
        serde_json::from_slice(&reread).map_err(|e| PortableSwitchError::Json {
            path: dst.clone(),
            source: e,
        })?;
    if reread_val.get("schema_version").and_then(|u| u.as_u64()) != Some(1) {
        return Err(PortableSwitchError::VerifyFailed(
            "schema_version != 1 after portable write".into(),
        ));
    }

    // finally marker (last; failure here keeps old)
    let marker = PortableMarker {
        schema_version: PORTABLE_SCHEMA_VERSION,
    };
    let marker_bytes =
        serde_json::to_vec_pretty(&marker).map_err(|e| PortableSwitchError::Json {
            path: portable_marker_path(workspace_root),
            source: e,
        })?;
    let marker_path = portable_marker_path(workspace_root);
    atomic_write(&marker_path, None, &marker_bytes, 0)?;

    Ok(())
}

/// Disable portable: read from workspace ui-state, convert refs to absolute,
/// atomic write to appdata ui-state, re-read verify, then remove marker.
/// Failure at any point before marker removal keeps portable mode.
pub fn disable_portable(
    workspace_root: &Path,
    app_data: &AppDataPaths,
) -> Result<(), PortableSwitchError> {
    if !is_portable_mode(workspace_root) {
        return Ok(());
    }
    let src = workspace_ui_state_path(workspace_root);
    let dst = app_data.ui_state_file();
    if let Some(parent) = dst.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let content = match fs::read(&src) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(e) => {
            return Err(PortableSwitchError::Io {
                operation: "read portable authority for disable",
                path: src,
                source: e,
            })
        }
    };

    let mut val: serde_json::Value = if content.is_empty() {
        serde_json::json!({
            "schema_version": 1,
            "dock": null,
            "canvas": null,
            "portable": true
        })
    } else {
        match serde_json::from_slice(&content) {
            Ok(v) => v,
            Err(e) => {
                return Err(PortableSwitchError::Json {
                    path: src,
                    source: e,
                })
            }
        }
    };

    if let Some(ver) = val.get("schema_version").and_then(|u| u.as_u64()) {
        if ver != 1 {
            return Err(PortableSwitchError::UnsupportedSchema {
                path: src,
                version: ver as u32,
            });
        }
    }

    absolutize_paths(&mut val, workspace_root);
    val["portable"] = serde_json::json!(false);

    let converted = serde_json::to_vec_pretty(&val).map_err(|e| PortableSwitchError::Json {
        path: dst.clone(),
        source: e,
    })?;

    let expected = read_version(&dst).ok().flatten();
    atomic_write(&dst, expected.as_ref(), &converted, 0)?;
    // verify
    let reread = match fs::read(&dst) {
        Ok(b) => b,
        Err(e) => {
            return Err(PortableSwitchError::Io {
                operation: "re-read verify after disable write",
                path: dst,
                source: e,
            })
        }
    };
    let reread_val: serde_json::Value =
        serde_json::from_slice(&reread).map_err(|e| PortableSwitchError::Json {
            path: dst.clone(),
            source: e,
        })?;
    if reread_val.get("schema_version").and_then(|u| u.as_u64()) != Some(1) {
        return Err(PortableSwitchError::VerifyFailed(
            "schema_version != 1 after disable write".into(),
        ));
    }

    // remove marker last
    let marker_path = portable_marker_path(workspace_root);
    if let Err(e) = fs::remove_file(&marker_path) {
        return Err(PortableSwitchError::Io {
            operation: "remove portable marker",
            path: marker_path,
            source: e,
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use uuid::Uuid;

    fn temp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("ohmywork-portable-{}-{}", name, Uuid::new_v4()));
        let _ = fs::create_dir_all(&p);
        p
    }

    #[test]
    fn portable_marker_roundtrip_and_is_mode() {
        let ws = temp("marker");
        assert!(!is_portable_mode(&ws));
        let marker_p = portable_marker_path(&ws);
        if let Some(parent) = marker_p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let m = PortableMarker { schema_version: PORTABLE_SCHEMA_VERSION };
        let b = serde_json::to_vec(&m).unwrap();
        fs::write(&marker_p, b).unwrap();
        assert!(is_portable_mode(&ws));
        let _ = fs::remove_dir_all(&ws);
    }

    #[test]
    fn resolve_prefers_workspace_when_marker_present() {
        let base = temp("resolve");
        let app = AppDataPaths::from_base(base.join("appdata")).unwrap();
        let ws = base.join("ws");
        let _ = fs::create_dir_all(&ws.join(".ohmywork"));
        fs::write(portable_marker_path(&ws), b"{\"schema_version\":1}").unwrap();
        let resolved = resolve_ui_state_path(&app, &ws);
        assert_eq!(resolved, workspace_ui_state_path(&ws));
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn relativize_and_absolutize_paths_only_under_root() {
        let root = Path::new("C:/ws");
        let mut v = serde_json::json!({
            "dock": {"panel": {"info": {"panel": "C:/ws/Atlas/a.md"}}},
            "outside": "C:/other/b.md",
            "rel": "Efforts/c.md",
            "num": 42
        });
        relativize_paths(&mut v, root);
        assert!(v["dock"]["panel"]["info"]["panel"].as_str().unwrap().contains("Atlas/a.md"));
        assert_eq!(v["outside"].as_str().unwrap(), "C:/other/b.md");
        assert_eq!(v["rel"].as_str().unwrap(), "Efforts/c.md");

        absolutize_paths(&mut v, root);
        assert!(v["dock"]["panel"]["info"]["panel"].as_str().unwrap().starts_with("C:/ws"));
        assert!(v["outside"].as_str().unwrap().starts_with("C:/other"));
    }
}
