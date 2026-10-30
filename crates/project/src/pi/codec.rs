use std::str;

use crate::pi::wire::Incoming;

const DEFAULT_MAX_FRAME: usize = 1 << 20; // 1 MiB per record
const DEFAULT_MAX_BUF: usize = 4 << 20;   // 4 MiB total pending

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FramerError {
    FrameTooLarge { size: usize, max: usize },
    BufferTooLarge { size: usize, max: usize },
    InvalidUtf8 { bytes: Vec<u8> },
    Json { err: String },
    // other
}

#[derive(Debug, Clone)]
pub struct CompleteFrame {
    /// Raw line bytes without the terminating \n (CR stripped if was \r\n)
    pub line: Vec<u8>,
}

impl CompleteFrame {
    pub fn as_str(&self) -> Result<&str, FramerError> {
        str::from_utf8(&self.line).map_err(|_| FramerError::InvalidUtf8 { bytes: self.line.clone() })
    }

    pub fn parse_incoming(&self) -> Result<Incoming, FramerError> {
        let s = self.as_str()?;
        serde_json::from_str::<Incoming>(s)
            .map_err(|e| FramerError::Json { err: e.to_string() })
    }
}

/// ByteFramer: feed bytes incrementally. Only emits on LF boundaries.
/// U+2028/U+2029 inside JSON strings are NOT frame boundaries (per spec).
/// CR before LF is stripped for CRLF tolerance.
pub struct ByteFramer {
    buf: Vec<u8>,
    max_frame: usize,
    max_buf: usize,
}

impl ByteFramer {
    pub fn new() -> Self {
        Self::with_limits(DEFAULT_MAX_FRAME, DEFAULT_MAX_BUF)
    }

    pub fn with_limits(max_frame: usize, max_buf: usize) -> Self {
        ByteFramer { buf: Vec::new(), max_frame, max_buf }
    }

    /// Feed more bytes. Returns zero or more complete frames (in order).
    /// Partial line kept in internal buf. Call repeatedly.
    pub fn feed(&mut self, mut data: &[u8]) -> Result<Vec<CompleteFrame>, FramerError> {
        let mut out = Vec::new();
        while !data.is_empty() {
            if let Some(pos) = data.iter().position(|&b| b == b'\n') {
                let mut frame = self.buf.split_off(0); // take pending
                frame.extend_from_slice(&data[..pos]);
                data = &data[pos + 1..];

                // strip optional preceding CR
                if frame.ends_with(&[b'\r']) {
                    frame.pop();
                }

                if frame.len() > self.max_frame {
                    return Err(FramerError::FrameTooLarge { size: frame.len(), max: self.max_frame });
                }

                out.push(CompleteFrame { line: frame });
            } else {
                // no LF yet, append all
                self.buf.extend_from_slice(data);
                data = &[];
                if self.buf.len() > self.max_buf {
                    return Err(FramerError::BufferTooLarge { size: self.buf.len(), max: self.max_buf });
                }
            }
        }
        // after consuming, check pending buf size
        if self.buf.len() > self.max_buf {
            return Err(FramerError::BufferTooLarge { size: self.buf.len(), max: self.max_buf });
        }
        Ok(out)
    }

    /// Remaining unframed bytes (for error/diag or shutdown).
    pub fn pending(&self) -> &[u8] {
        &self.buf
    }

    /// Reset for reuse (e.g. after error recovery in tests).
    pub fn reset(&mut self) {
        self.buf.clear();
    }
}

impl Default for ByteFramer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_lf() {
        let mut f = ByteFramer::new();
        let frames = f.feed(b"{\"a\":1}\n{\"b\":2}\n").unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].as_str().unwrap(), "{\"a\":1}");
    }

    #[test]
    fn crlf() {
        let mut f = ByteFramer::new();
        let frames = f.feed(b"{\"x\":true}\r\n").unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].as_str().unwrap(), "{\"x\":true}");
    }

    #[test]
    fn split_multi_byte_utf8() {
        let mut f = ByteFramer::new();
        // "中" is e4 b8 ad
        let frames1 = f.feed(&[0xe4, 0xb8]).unwrap(); // partial
        assert!(frames1.is_empty());
        let frames2 = f.feed(&[0xad, b'\n']).unwrap();
        assert_eq!(frames2.len(), 1);
        assert_eq!(frames2[0].as_str().unwrap(), "中");
    }

    #[test]
    fn u2028_inside_not_frame() {
        // U+2028 is e2 80 a8 ; must stay inside json string, not split frame
        let mut f = ByteFramer::new();
        let json = b"{\"t\":\"a\xe2\x80\xa8b\"}\n";
        let frames = f.feed(json).unwrap();
        assert_eq!(frames.len(), 1);
        assert!(frames[0].as_str().unwrap().contains('\u{2028}'));
    }

    #[test]
    fn frame_limit() {
        let mut f = ByteFramer::with_limits(5, 100);
        let res = f.feed(b"123456\n");
        assert!(matches!(res, Err(FramerError::FrameTooLarge { .. })));
    }
}
