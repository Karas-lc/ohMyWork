use omw_document::{VisualPos, VisualSelection};

/// This projection is fed into the GPUI TextInput synthetic-child tree: each
/// stable run is exposed as an AccessKit TextRun and the parent owns selection/focus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessibleTextRun {
    pub stable_id: String,
    pub text: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessibleEditor {
    pub stable_id: String,
    pub runs: Vec<AccessibleTextRun>,
    pub selection: VisualSelection,
    pub focused: bool,
}
impl AccessibleEditor {
    pub fn new(
        id: impl Into<String>,
        runs: Vec<AccessibleTextRun>,
        selection: VisualSelection,
        focused: bool,
    ) -> Self {
        Self {
            stable_id: id.into(),
            runs,
            selection,
            focused,
        }
    }
    pub fn selected_range(&self) -> (VisualPos, VisualPos) {
        let range = self.selection.range();
        (range.start, range.end)
    }
}
