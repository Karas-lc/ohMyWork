use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::atomic_write::FileIdentity;

/// Resolve a filesystem path using the operating system's link semantics.
pub fn resolve_filesystem_path(path: &Path) -> std::io::Result<PathBuf> {
    fs::canonicalize(path)
}

/// Return the volume/file identity of the resolved filesystem object at `path`.
pub fn filesystem_identity(path: &Path) -> std::io::Result<FileIdentity> {
    let resolved = fs::canonicalize(path)?;
    let metadata = fs::metadata(&resolved)?;
    let identity = crate::atomic_write::file_identity_for_traversal(&resolved, &metadata)
        .unwrap_or_else(|| FileIdentity {
            volume_serial: 0,
            file_index: u64::from_le_bytes(
                blake3::hash(resolved.to_string_lossy().as_bytes()).as_bytes()[..8]
                    .try_into()
                    .unwrap_or_default(),
            ),
        });
    Ok(identity)
}

/// Stable key used to serialize work for one filesystem path.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DocumentKey(PathBuf);

impl DocumentKey {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self(normalize_document_path(path))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

/// Normalize aliases before a path enters a per-document queue.
///
/// Existing paths are canonicalized so symlinks and relative spellings share a
/// key. A deleted path cannot be canonicalized; its absolute spelling remains
/// usable for deletion/recreation handling.
pub fn normalize_document_path(path: impl Into<PathBuf>) -> PathBuf {
    let path = path.into();
    let absolute = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .map(|directory| directory.join(&path))
            .unwrap_or(path)
    };
    fs::canonicalize(&absolute).unwrap_or(absolute)
}

/// Application-owned atomic-write temporaries are ignored by path filtering,
/// but only for that exact temporary path. Events for the destination path are
/// never suppressed by a directory-wide time window.
pub fn is_application_temporary(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    name.starts_with('.') && name.contains(".ohmywork-") && name.ends_with(".tmp")
}
