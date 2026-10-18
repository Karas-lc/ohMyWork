//! Plan 21 preview worker main. Single-thread serial decode using protocol + real pdf/image.
//! Reads u32_le header_len + JSON FrameHeader + payload.
//! For Bitmap responses: payload = u32 meta_json_len_le + json_bytes + rgba_bytes (total in header payload_len).
//! Enforces limits. Over: Error, stay alive. No crash UI.

use std::collections::{HashMap, VecDeque};
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::time::Instant;

mod image;
mod limits;
mod pdf;
mod protocol;

use limits::{
    check_cache_add, check_decode_time, check_open_count, check_queued_pages, check_rss,
    check_stride,
};
use protocol::{
    FrameHeader, PROTOCOL_VERSION, PreviewRequest, PreviewResponse, decode_header_len_and_header,
    encode_frame,
};

/// Cache key: document, page, target width, target height.
type CacheKey = (u64, u32, u32, u32);

/// Byte-aware LRU of decoded bitmaps. Cap is `MAX_CACHE_BYTES` via `check_cache_add`.
struct BitmapCache {
    order: VecDeque<CacheKey>,
    map: HashMap<CacheKey, (Vec<u8>, u32, u32)>,
    bytes: u64,
}

impl BitmapCache {
    fn new() -> Self {
        Self {
            order: VecDeque::new(),
            map: HashMap::new(),
            bytes: 0,
        }
    }

    fn get(&mut self, key: &CacheKey) -> Option<(Vec<u8>, u32, u32)> {
        let hit = self.map.get(key).cloned()?;
        self.order.retain(|k| k != key);
        self.order.push_back(*key);
        Some(hit)
    }

    fn insert(
        &mut self,
        key: CacheKey,
        data: Vec<u8>,
        w: u32,
        h: u32,
    ) -> Result<(), (String, String)> {
        let add = data.len() as u64;
        if let Some((old, _, _)) = self.map.remove(&key) {
            self.bytes = self.bytes.saturating_sub(old.len() as u64);
            self.order.retain(|k| k != &key);
        }
        while check_cache_add(self.bytes, add).is_err() {
            let Some(old_key) = self.order.pop_front() else {
                break;
            };
            if let Some((d, _, _)) = self.map.remove(&old_key) {
                self.bytes = self.bytes.saturating_sub(d.len() as u64);
            }
        }
        check_cache_add(self.bytes, add)?;
        self.bytes += add;
        self.map.insert(key, (data, w, h));
        self.order.push_back(key);
        Ok(())
    }

    fn remove_doc(&mut self, document_id: u64) {
        let keys: Vec<CacheKey> = self
            .map
            .keys()
            .copied()
            .filter(|k| k.0 == document_id)
            .collect();
        for k in keys {
            if let Some((d, _, _)) = self.map.remove(&k) {
                self.bytes = self.bytes.saturating_sub(d.len() as u64);
            }
        }
        self.order.retain(|k| k.0 != document_id);
    }
}

const BASE_RSS: u64 = 4 * 1024 * 1024;

fn main() {
    let mut stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut next_doc_id: u64 = 1;
    let mut open_docs: HashMap<u64, (PathBuf, u32)> = HashMap::new();
    let mut cache = BitmapCache::new();
    let mut rss_bytes: u64 = BASE_RSS;
    let mut queued_pages: usize = 0;
    loop {
        let mut len_buf = [0u8; 4];
        if stdin.read_exact(&mut len_buf).is_err() {
            break;
        }
        let header_len = u32::from_le_bytes(len_buf);
        if header_len == 0 || header_len > 2 * 1024 * 1024 {
            let err = make_error(0, "malformed", "bad header len");
            let _ = write_response(&mut stdout, &err, None);
            continue;
        }

        let mut header_buf = vec![0u8; header_len as usize];
        if stdin.read_exact(&mut header_buf).is_err() {
            break;
        }
        let mut full = len_buf.to_vec();
        full.extend_from_slice(&header_buf);

        match decode_header_len_and_header(&full) {
            Ok((frame_hdr, _)) => {
                let mut payload = vec![0u8; frame_hdr.payload_len as usize];
                if frame_hdr.payload_len > 0 && stdin.read_exact(&mut payload).is_err() {
                    break;
                }
                match serde_json::from_slice::<PreviewRequest>(&payload) {
                    Ok(req) => {
                        let (resp, bitmap_data) = handle_request(
                            req,
                            &mut next_doc_id,
                            &mut open_docs,
                            &mut rss_bytes,
                            &mut queued_pages,
                            &mut cache,
                        );
                        let _ = write_response(&mut stdout, &resp, bitmap_data);
                    }
                    Err(e) => {
                        let err = make_error(
                            frame_hdr.request_id,
                            "malformed",
                            &format!("bad req json: {}", e),
                        );
                        let _ = write_response(&mut stdout, &err, None);
                    }
                }
            }
            Err(e) => {
                let err = make_error(0, "malformed", &e);
                let _ = write_response(&mut stdout, &err, None);
            }
        }
    }
}

fn make_error(request_id: u64, code: &str, msg: &str) -> PreviewResponse {
    PreviewResponse::Error {
        request_id,
        code: code.to_string(),
        message: msg.to_string(),
    }
}

fn handle_request(
    req: PreviewRequest,
    next_doc: &mut u64,
    open_docs: &mut HashMap<u64, (PathBuf, u32)>,
    rss_bytes: &mut u64,
    queued_pages: &mut usize,
    cache: &mut BitmapCache,
) -> (PreviewResponse, Option<Vec<u8>>) {
    match req {
        PreviewRequest::Open { path, request_id } => {
            let start = Instant::now();
            if let Err(e) =
                limits::check_file_size(std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0))
            {
                return (make_error(request_id, &e.0, &e.1), None);
            }
            if let Err(e) = check_open_count(open_docs.len()) {
                return (make_error(request_id, &e.0, &e.1), None);
            }
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            let (page_count, title) = if ext == "pdf" {
                match pdf::open_pdf(&path) {
                    Ok(d) => (d.page_count, d.title),
                    Err((code, msg)) => return (make_error(request_id, &code, &msg), None),
                }
            } else {
                (
                    1u32,
                    path.file_name().map(|s| s.to_string_lossy().into_owned()),
                )
            };
            if let Err(e) = check_decode_time(start) {
                return (make_error(request_id, &e.0, &e.1), None);
            }
            let doc_id = *next_doc;
            *next_doc += 1;
            open_docs.insert(doc_id, (path, page_count));
            (
                PreviewResponse::Opened {
                    request_id,
                    document_id: doc_id,
                    page_count,
                    title,
                },
                None,
            )
        }
        PreviewRequest::Render {
            document_id,
            page,
            target_size,
            generation,
            request_id,
        } => {
            let start = Instant::now();
            let (path, page_count) = match open_docs.get(&document_id) {
                Some(p) => p.clone(),
                None => return (make_error(request_id, "not_found", "doc not open"), None),
            };
            if page >= page_count {
                return (
                    make_error(request_id, "malformed", "page out of range"),
                    None,
                );
            }
            if let Err(e) = limits::check_page_px(target_size[0], target_size[1]) {
                return (make_error(request_id, &e.0, &e.1), None);
            }
            let key: CacheKey = (document_id, page, target_size[0], target_size[1]);
            if let Some((data, w, h)) = cache.get(&key) {
                let stride = w.saturating_mul(4);
                if let Err(e) = check_stride(w, h, stride) {
                    return (make_error(request_id, &e.0, &e.1), None);
                }
                let resp = PreviewResponse::Bitmap {
                    request_id,
                    document_id,
                    page,
                    generation,
                    width: w,
                    height: h,
                    stride,
                };
                return (resp, Some(data));
            }
            *queued_pages = queued_pages.saturating_add(1);
            if let Err(e) = check_queued_pages(*queued_pages) {
                *queued_pages = queued_pages.saturating_sub(1);
                return (make_error(request_id, &e.0, &e.1), None);
            }
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            let result = if ext == "pdf" {
                pdf::render_page(
                    &pdf::PdfDoc {
                        page_count,
                        title: None,
                    },
                    page,
                    target_size[0],
                    target_size[1],
                    Some(&path),
                )
            } else {
                image::decode_image(&path, target_size[0], target_size[1])
            };
            if let Err(e) = check_decode_time(start) {
                *queued_pages = queued_pages.saturating_sub(1);
                return (make_error(request_id, &e.0, &e.1), None);
            }
            match result {
                Ok((data, w, h)) => {
                    let stride = w.saturating_mul(4);
                    if let Err(e) = check_stride(w, h, stride) {
                        *queued_pages = queued_pages.saturating_sub(1);
                        return (make_error(request_id, &e.0, &e.1), None);
                    }
                    if let Err(e) = cache.insert(key, data.clone(), w, h) {
                        *queued_pages = queued_pages.saturating_sub(1);
                        return (make_error(request_id, &e.0, &e.1), None);
                    }
                    *rss_bytes = BASE_RSS + cache.bytes;
                    if let Err(e) = check_rss(*rss_bytes) {
                        cache.remove_doc(document_id);
                        *rss_bytes = BASE_RSS + cache.bytes;
                        *queued_pages = queued_pages.saturating_sub(1);
                        return (make_error(request_id, &e.0, &e.1), None);
                    }
                    let resp = PreviewResponse::Bitmap {
                        request_id,
                        document_id,
                        page,
                        generation,
                        width: w,
                        height: h,
                        stride,
                    };
                    *queued_pages = queued_pages.saturating_sub(1);
                    (resp, Some(data))
                }
                Err((code, msg)) => {
                    *queued_pages = queued_pages.saturating_sub(1);
                    (make_error(request_id, &code, &msg), None)
                }
            }
        }
        PreviewRequest::Close {
            document_id,
            request_id,
        } => {
            open_docs.remove(&document_id);
            cache.remove_doc(document_id);
            *rss_bytes = BASE_RSS + cache.bytes;
            (
                PreviewResponse::Closed {
                    request_id,
                    document_id,
                },
                None,
            )
        }
        PreviewRequest::Shutdown { request_id: _ } => {
            std::process::exit(0);
        }
    }
}
fn write_response(
    stdout: &mut impl Write,
    resp: &PreviewResponse,
    bitmap_data: Option<Vec<u8>>,
) -> io::Result<()> {
    let meta = match serde_json::to_vec(resp) {
        Ok(m) => m,
        Err(e) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("ser resp: {}", e),
            ));
        }
    };
    let mut payload = meta.clone();
    if let Some(data) = bitmap_data {
        // For Bitmap: prefix meta_len + meta + data
        let mlen = meta.len() as u32;
        payload = Vec::new();
        payload.extend_from_slice(&mlen.to_le_bytes());
        payload.extend(meta);
        payload.extend(data);
    }
    let request_id = match resp {
        PreviewResponse::Error { request_id, .. } => *request_id,
        PreviewResponse::Opened { request_id, .. } => *request_id,
        PreviewResponse::PageInfo { request_id, .. } => *request_id,
        PreviewResponse::Bitmap { request_id, .. } => *request_id,
        PreviewResponse::Closed { request_id, .. } => *request_id,
    };
    let header = FrameHeader {
        version: PROTOCOL_VERSION,
        request_id,
        kind: if matches!(resp, PreviewResponse::Bitmap { .. }) {
            "bitmap"
        } else {
            "response"
        }
        .into(),
        payload_len: payload.len() as u32,
    };
    let frame = encode_frame(&header, &payload);
    stdout.write_all(&frame)?;
    stdout.flush()
}
