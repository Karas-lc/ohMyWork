//! Rendering adapter boundary for the paragraph editor.
//!
//! This module deliberately exposes only visible text and stable block identities: the
//! source Markdown (including hidden delimiters) is never an accessibility/input string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisibleBlock {
    pub id: String,
    pub text: String,
    pub editable: bool,
}
impl VisibleBlock {
    pub fn new(id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
            editable: true,
        }
    }
    pub fn raw(id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
            editable: false,
        }
    }
}
