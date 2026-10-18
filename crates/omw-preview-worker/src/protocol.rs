//! Plan 21: preview protocol.
//! Frame: u32_le header_len (little endian) + JSON header + payload (binary for bitmaps).
//! Header: version, request_id (u64), kind (string), payload_len (u32).
//! All lengths/width*height*4/stride checked against limits before any alloc.
//! RGBA binary, no base64. Single-thread serial in worker.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const PROTOCOL_VERSION: u32 = 1;

// Per acceptance + plan: both sides enforce. Over -> typed Error, worker process stays up.
pub const MAX_FILE_BYTES: u64 = 80 * 1024 * 1024;
pub const MAX_PDF_PAGES: u32 = 200;
pub const MAX_PAGE_PX: u32 = 10_000;
pub const MAX_BITMAP_BYTES: u64 = 32 * 1024 * 1024;
pub const MAX_DECODE_SECS: u64 = 8;
pub const MAX_RSS_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_OPEN_DOCS: usize = 2;
pub const MAX_QUEUED_PAGES: usize = 8;

/// Legacy cache target (LRU impl optional in V1; byte aware).
pub const MAX_CACHE_BYTES: u64 = 96 * 1024 * 1024;

/// Request from host to worker. Matches user spec (no PageInfo req).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PreviewRequest {
    Open {
        path: PathBuf,
        request_id: u64,
    },
    Render {
        document_id: u64,
        page: u32,
        target_size: [u32; 2], // w, h logical px; pre-checked <= MAX_PAGE_PX
        generation: u64,
        request_id: u64,
    },
    Close {
        document_id: u64,
        request_id: u64,
    },
    Shutdown {
        request_id: u64,
    },
}

/// Response from worker.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PreviewResponse {
    Opened {
        request_id: u64,
        document_id: u64,
        page_count: u32,
        title: Option<String>,
    },
    PageInfo {
        request_id: u64,
        page: u32,
        width: u32,
        height: u32,
        // more metadata later
    },
    Bitmap {
        request_id: u64,
        document_id: u64,
        page: u32,
        generation: u64,
        width: u32,
        height: u32,
        stride: u32, // bytes per row, usually width*4
        // payload is raw RGBA after header
    },
    Error {
        request_id: u64,
        code: String, // "overflow", "malformed", "timeout", "not_found", "password", "unsupported"
        message: String,
    },
    Closed {
        request_id: u64,
        document_id: u64,
    },
}

/// Wire frame header (JSON serialized).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FrameHeader {
    pub version: u32,
    pub request_id: u64,
    pub kind: String, // "request" or "response" or specific for dispatch
    pub payload_len: u32,
}

/// Helper to encode a frame (header json + payload).
/// Caller must ensure payload_len == payload.len() and checks passed.
pub fn encode_frame(header: &FrameHeader, payload: &[u8]) -> Vec<u8> {
    let header_bytes = serde_json::to_vec(header).expect("header must serialize");
    let header_len = header_bytes.len() as u32;
    let mut buf = Vec::with_capacity(4 + header_bytes.len() + payload.len());
    buf.extend_from_slice(&header_len.to_le_bytes());
    buf.extend_from_slice(&header_bytes);
    buf.extend_from_slice(payload);
    buf
}

/// Decode header len + header. Returns (header, header_bytes_consumed).
/// Does NOT read payload; caller reads exactly payload_len after.
pub fn decode_header_len_and_header(bytes: &[u8]) -> Result<(FrameHeader, usize), String> {
    if bytes.len() < 4 {
        return Err("short header len".into());
    }
    let header_len = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let start = 4;
    if bytes.len() < start + header_len as usize {
        return Err("short header".into());
    }
    let header: FrameHeader = serde_json::from_slice(&bytes[start..start + header_len as usize])
        .map_err(|e| format!("header json: {}", e))?;
    if header.version != PROTOCOL_VERSION {
        return Err(format!("bad version {}", header.version));
    }
    Ok((header, start + header_len as usize))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_frame() {
        let h = FrameHeader {
            version: PROTOCOL_VERSION,
            request_id: 42,
            kind: "request".into(),
            payload_len: 0,
        };
        let b = encode_frame(&h, &[]);
        let (h2, _) = decode_header_len_and_header(&b).unwrap();
        assert_eq!(h2.request_id, 42);
    }
}
