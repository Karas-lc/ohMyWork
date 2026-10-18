//! omw-preview-worker types for host integration (protocol, limits).
//! The binary is launched separately; this crate is a dependency for types + fake test harness.

pub mod protocol;
pub mod limits;

// Re-export for convenience in omw-app
pub use protocol::{PreviewRequest, PreviewResponse, FrameHeader, PROTOCOL_VERSION, encode_frame, decode_header_len_and_header};
pub use limits::{check_bitmap_size, check_stride, check_cache_add};
