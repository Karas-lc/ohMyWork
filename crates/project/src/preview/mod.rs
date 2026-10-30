//! Preview worker protocol and resource limits shared with the host adapter.

pub mod protocol;
pub mod limits;
pub mod client;

pub use client::{PreviewClient, PreviewHandle, PreviewReply};

pub use protocol::{
    PreviewRequest, PreviewResponse, FrameHeader, PROTOCOL_VERSION, encode_frame,
    decode_header_len_and_header,
};
pub use limits::{check_bitmap_size, check_stride, check_cache_add};
