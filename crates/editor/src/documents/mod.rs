mod document;
mod projection;
mod source;
use source::SourceSession;
pub(crate) mod persistence;
pub(crate) mod store;
pub(crate) mod task_commit;

pub use document::{PageCommand, PageDocument, PageEditOutcome, PageError};
pub use projection::PageSnapshot;
pub use source::{SourceRope, SourceSelection, decode_source};
pub use store::{DocumentEvent, DocumentStore, PageOpenState};
pub use task_commit::TaskIntent;
