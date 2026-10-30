//! Framing contract 1 tests (pure, no process).
//! Split UTF8, U+2028 inside not boundary, CRLF, multi-frame feed, limits, bad utf8/json.

use project::pi::{ByteFramer, CompleteFrame, FramerError};

fn feed_all(f: &mut ByteFramer, chunks: &[&[u8]]) -> Result<Vec<CompleteFrame>, FramerError> {
    let mut out = vec![];
    for c in chunks {
        out.extend(f.feed(c)?);
    }
    Ok(out)
}

#[test]
fn split_utf8_across_feeds() {
    let mut f = ByteFramer::new();
    // "中文" = e4 b8 ad e6 96 87
    let frames = feed_all(&mut f, &[&[0xe4, 0xb8], &[0xad, 0xe6, 0x96, 0x87, b'\n']]).unwrap();
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].as_str().unwrap(), "中文");
}

#[test]
fn u2028_inside_json_string_is_not_frame_boundary() {
    let mut f = ByteFramer::new();
    // JSON with literal U+2028 inside string value (bytes: e2 80 a8)
    let mut input = b"{\"msg\":\"hello".to_vec();
    input.extend_from_slice(&[0xe2, 0x80, 0xa8]);
    input.extend_from_slice(b"world\"}\n");
    let frames = f.feed(&input).unwrap();
    assert_eq!(frames.len(), 1);
    let s = frames[0].as_str().unwrap();
    assert!(s.contains('\u{2028}'));
    assert!(s.contains("hello"));
}
fn crlf_stripped() {
    let mut f = ByteFramer::new();
    let frames = f.feed(b"{\"ok\":true}\r\nnext\n").unwrap();
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0].as_str().unwrap(), "{\"ok\":true}");
    assert_eq!(frames[1].as_str().unwrap(), "next");
}

#[test]
fn multiple_frames_one_feed() {
    let mut f = ByteFramer::new();
    let frames = f.feed(b"a\nb\nc\n").unwrap();
    assert_eq!(frames.len(), 3);
}

#[test]
fn frame_limit_enforced() {
    let mut f = ByteFramer::with_limits(3, 100);
    let err = f.feed(b"1234\n").unwrap_err();
    assert!(matches!(err, FramerError::FrameTooLarge { .. }));
}

#[test]
fn buf_limit_enforced_on_partial() {
    let mut f = ByteFramer::with_limits(100, 5);
    let _ = f.feed(b"abc"); // no lf
    let err = f.feed(b"def").unwrap_err();
    assert!(matches!(err, FramerError::BufferTooLarge { .. }));
}

#[test]
fn bad_utf8_yields_error_on_complete() {
    let mut f = ByteFramer::new();
    // lone continuation byte + lf  => frame ok, as_str fails
    let frames = f.feed(&[0x80, b'\n']).unwrap();
    let err = frames[0].as_str().unwrap_err();
    assert!(matches!(err, FramerError::InvalidUtf8 { .. }));
}
#[test]
fn bad_json_yields_json_error_on_parse() {
    let mut f = ByteFramer::new();
    let frames = f.feed(b"not json\n").unwrap();
    let err = frames[0].parse_incoming().unwrap_err();
    assert!(matches!(err, FramerError::Json { .. }));
}

#[test]
fn pending_after_partial() {
    let mut f = ByteFramer::new();
    let _ = f.feed(b"partial");
    assert_eq!(f.pending(), b"partial");
    f.reset();
    assert!(f.pending().is_empty());
}
