//! Plan 21: strict resource limits for preview worker.
//! Check before any allocation. Report typed errors.

use crate::protocol::{
    MAX_BITMAP_BYTES, MAX_CACHE_BYTES, MAX_DECODE_SECS, MAX_FILE_BYTES, MAX_OPEN_DOCS, MAX_PAGE_PX, MAX_PDF_PAGES,
    MAX_RSS_BYTES, MAX_QUEUED_PAGES,
};
use std::time::Instant;

/// Check bitmap dimensions against byte limit (w*h*4).
/// Returns Err(code, msg) if would overflow or exceed.
pub fn check_bitmap_size(width: u32, height: u32) -> Result<u64, (String, String)> {
    let w = width as u64;
    let h = height as u64;
    let bytes = w.checked_mul(h).and_then(|wh| wh.checked_mul(4))
        .ok_or_else(|| ("overflow".to_string(), "width*height*4 overflow".to_string()))?;
    if bytes > MAX_BITMAP_BYTES {
        return Err(("overflow".into(), format!("bitmap {} > {} limit", bytes, MAX_BITMAP_BYTES)));
    }
    if bytes == 0 {
        return Err(("malformed".into(), "zero size bitmap".into()));
    }
    Ok(bytes)
}

/// Stride must be >= width*4 and reasonable.
pub fn check_stride(width: u32, height: u32, stride: u32) -> Result<(), (String, String)> {
    let min_stride = width.checked_mul(4).ok_or_else(|| ("overflow".into(), "stride calc".into()))?;
    if stride < min_stride {
        return Err(("malformed".into(), "stride too small".into()));
    }
    // additional sanity: stride * height should match or exceed expected bytes
    let total = (stride as u64).checked_mul(height as u64).ok_or(("overflow".into(), "total bytes".into()))?;
    if total > MAX_BITMAP_BYTES {
        return Err(("overflow".into(), "stride*height exceeds".into()));
    }
    Ok(())
}

/// For cache management (LRU byte accounting).
pub fn check_cache_add(current: u64, add: u64) -> Result<u64, (String, String)> {
    let new = current.checked_add(add).ok_or(("overflow".into(), "cache total".into()))?;
    if new > MAX_CACHE_BYTES {
        return Err(("overflow".into(), "cache would exceed".into()));
    }
    Ok(new)
}
pub fn check_file_size(len: u64) -> Result<(), (String, String)> {
    if len > MAX_FILE_BYTES {
        return Err(("overflow".into(), format!("file {} > {} limit", len, MAX_FILE_BYTES)));
    }
    if len == 0 {
        return Err(("malformed".into(), "empty file".into()));
    }
    Ok(())
}

pub fn check_pdf_pages(n: u32) -> Result<(), (String, String)> {
    if n > MAX_PDF_PAGES {
        return Err(("overflow".into(), format!("pdf pages {} > {}", n, MAX_PDF_PAGES)));
    }
    if n == 0 {
        return Err(("malformed".into(), "0 pages".into()));
    }
    Ok(())
}

pub fn check_page_px(w: u32, h: u32) -> Result<(), (String, String)> {
    if w > MAX_PAGE_PX || h > MAX_PAGE_PX {
        return Err(("overflow".into(), format!("page dim {}x{} > {}", w, h, MAX_PAGE_PX)));
    }
    if w == 0 || h == 0 {
        return Err(("malformed".into(), "zero dim".into()));
    }
    Ok(())
}

pub fn check_decode_time(start: Instant) -> Result<(), (String, String)> {
    if start.elapsed().as_secs() > MAX_DECODE_SECS {
        return Err(("timeout".into(), "decode exceeded 8s".into()));
    }
    Ok(())
}

pub fn check_open_count(n: usize) -> Result<(), (String, String)> {
    if n >= MAX_OPEN_DOCS {
        return Err(("overflow".into(), "max 2 docs".into()));
    }
    Ok(())
}
pub fn check_queued_pages(n: usize) -> Result<(), (String, String)> {
    if n > MAX_QUEUED_PAGES {
        return Err(("overflow".to_string(), format!("queued pages {} > {} limit", n, MAX_QUEUED_PAGES)));
    }
    Ok(())
}

/// Approximate RSS (sum of decoded bitmap bytes etc). Enforce without killing process.
pub fn check_rss(bytes: u64) -> Result<(), (String, String)> {
    if bytes > MAX_RSS_BYTES {
        return Err(("overflow".to_string(), format!("rss {} > {} limit", bytes, MAX_RSS_BYTES)));
    }
    Ok(())
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn rejects_huge_bitmap() {
        assert!(check_bitmap_size(100_000, 100_000).is_err());
    }

    #[test]
    fn accepts_reasonable() {
        assert!(check_bitmap_size(100, 100).is_ok());
    }

    #[test]
    fn stride_check() {
        assert!(check_stride(10, 10, 40).is_ok());
        assert!(check_stride(10, 10, 39).is_err());
    }

    #[test]
    fn file_pdf_page_px_doc_time_limits() {
        assert!(check_file_size(MAX_FILE_BYTES + 1).is_err());
        assert!(check_pdf_pages(MAX_PDF_PAGES + 1).is_err());
        assert!(check_page_px(MAX_PAGE_PX + 1, 100).is_err());
        assert!(check_open_count(MAX_OPEN_DOCS).is_err());
        let late = Instant::now() - Duration::from_secs(MAX_DECODE_SECS + 1);
        assert!(check_decode_time(late).is_err());
    }
    #[test]
    fn queued_and_rss_limits() {
        assert!(check_queued_pages(MAX_QUEUED_PAGES + 1).is_err());
        assert!(check_queued_pages(3).is_ok());
        assert!(check_rss(MAX_RSS_BYTES + 1).is_err());
        assert!(check_rss(1024).is_ok());
    }

}

