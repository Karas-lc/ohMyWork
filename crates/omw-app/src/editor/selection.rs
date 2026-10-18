use omw_document::{VisualPos, VisualSelection};

/// Per-view selection; never stored in the shared document entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditorSelection(VisualSelection);
impl EditorSelection {
    pub fn caret(at: VisualPos) -> Self {
        Self(VisualSelection::collapsed(at))
    }
    pub fn new(anchor: VisualPos, head: VisualPos) -> Self {
        Self(VisualSelection::new(anchor, head))
    }
    pub fn selection(self) -> VisualSelection {
        self.0
    }
    pub fn move_head(&mut self, to: VisualPos, extend: bool) {
        self.0 = if extend {
            VisualSelection::new(self.0.anchor, to)
        } else {
            VisualSelection::collapsed(to)
        };
    }
}
