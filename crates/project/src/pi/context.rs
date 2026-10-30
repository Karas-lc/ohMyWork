//! Explicit user context (attachments) for Pi prompts.
//! Never auto-attach. File = absolute path (or relative unambiguous), selection = text snapshot.
//! Used only when user explicitly adds attachments before send.
//! Prompt wrapper: user text + separator + file/selection blocks. No credentials, no body logging.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ContextItem {
    /// Absolute path preferred for unambiguous reference.
    pub path: PathBuf,
    /// Exact user-selected text snapshot at attach time (not live). Empty or None means whole-file intent for preview.
    pub selection: Option<String>,
}

impl ContextItem {
    pub fn new(path: impl Into<PathBuf>, selection: Option<String>) -> Self {
        Self { path: path.into(), selection }
    }

    pub fn file_only(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into(), selection: None }
    }
}

/// Build the final prompt message for Command::prompt.
/// Prepends user message, then separator, then explicit file/selection blocks.
/// Keeps it simple and unambiguous per plan17 + ADR-0010.
/// Does not add cwd prefix; caller may relativize for display only.
pub fn wrap_context_for_prompt(user_message: &str, attachments: &[ContextItem]) -> String {
    if attachments.is_empty() {
        return user_message.to_string();
    }
    let mut out = String::with_capacity(user_message.len() + attachments.len() * 256 + 64);
    out.push_str(user_message);
    out.push_str("\n\n---\n\n");
    for (i, item) in attachments.iter().enumerate() {
        if i > 0 {
            out.push_str("\n");
        }
        out.push_str("File: ");
        out.push_str(&item.path.display().to_string());
        out.push('\n');
        if let Some(sel) = &item.selection {
            if !sel.is_empty() {
                out.push_str("Selection:\n");
                out.push_str(sel);
                out.push('\n');
            }
        }
    }
    out
}

/// For UI display only: attempt relative to cwd if under it; else abs.
pub fn display_path(item: &ContextItem, cwd: Option<&Path>) -> String {
    if let Some(c) = cwd {
        if let Ok(rel) = item.path.strip_prefix(c) {
            return rel.display().to_string();
        }
    }
    item.path.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_empty() {
        assert_eq!(wrap_context_for_prompt("hi", &[]), "hi");
    }

    #[test]
    fn wrap_with_file_and_sel() {
        let items = vec![
            ContextItem::new("/abs/a.md", Some("foo\nbar".into())),
            ContextItem::file_only("/abs/b.txt"),
        ];
        let wrapped = wrap_context_for_prompt("task", &items);
        assert!(wrapped.contains("task\n\n---\n\n"));
        assert!(wrapped.contains("File: /abs/a.md"));
        assert!(wrapped.contains("Selection:\nfoo\nbar"));
        assert!(wrapped.contains("File: /abs/b.txt"));
    }
}
