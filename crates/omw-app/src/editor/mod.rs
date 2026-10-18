//! Paragraph editor surface state. Document facts stay in `omw-document`; view state is per-view.
mod a11y;
mod element;
mod input;
mod layout;
mod selection;
mod toolbar;
mod view;

pub use a11y::{AccessibleEditor, AccessibleTextRun};
pub use element::VisibleBlock;
pub use input::{
    InputError, Utf16Range, VisibleProjection, commit_text, delete_visible_range,
    utf16_range_to_visual,
};
pub use layout::{BlockLayout, LayoutCache, LayoutKey, Viewport};
pub use selection::EditorSelection;
pub use view::{EditorDocument, EditorSurface, EditorViewState, ParsePool, ParseResult};
