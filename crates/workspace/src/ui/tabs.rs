//! Tab and dock-tab utilities (plan 18 skeleton).
//! TabRef is the payload for persisted layout. No Entity inside.

use std::path::PathBuf;

use gpui_kit::SharedString;
use serde::{Deserialize, Serialize};

/// Domain tab identity. Persisted payload only; no Entity, no content bytes.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TabRef {
    Markdown { path: PathBuf },
    Preview { path: PathBuf },
    Home,
}

impl TabRef {
    pub fn display_name(&self) -> SharedString {
        match self {
            TabRef::Markdown { path } | TabRef::Preview { path } => {
                path.file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string())
                    .into()
            }
            TabRef::Home => "Home".into(),
        }
    }

}
