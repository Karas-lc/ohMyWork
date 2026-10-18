//! Real PDF using pdfium-render 0.9.4 bound at runtime to absolute path next to exe (non-V8).
//! If load fails (no dll asset): clear "pdfium unavailable" error. Enforce limits; return typed error, stay up.

use crate::limits::{check_bitmap_size, check_page_px, check_pdf_pages};
use pdfium_render::prelude::*;
use std::path::Path;
use std::sync::OnceLock;

static PDFIUM: OnceLock<Result<Pdfium, String>> = OnceLock::new();

fn get_pdfium() -> Result<&'static Pdfium, (String, String)> {
    let res = PDFIUM.get_or_init(|| {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let lib_name = Pdfium::pdfium_platform_library_name_at_path(&exe_dir);
        match Pdfium::bind_to_library(lib_name) {
            Ok(bindings) => Ok(Pdfium::new(bindings)),
            Err(e) => Err(format!("pdfium unavailable: {}", e)),
        }
    });
    match res {
        Ok(p) => Ok(p),
        Err(msg) => Err(("unsupported".to_string(), msg.clone())),
    }
}

pub struct PdfDoc {
    pub page_count: u32,
    pub title: Option<String>,
}

pub fn open_pdf(path: &Path) -> Result<PdfDoc, (String, String)> {
    let pdfium = get_pdfium()?;
    let doc = pdfium
        .load_pdf_from_file(path.to_str().unwrap_or(""), None)
        .map_err(|e| ("malformed".to_string(), format!("pdf load: {}", e)))?;
    let page_count = doc.pages().len() as u32;
    check_pdf_pages(page_count)?;
    let title = doc
        .metadata()
        .get(pdfium_render::prelude::PdfDocumentMetadataTagType::Title)
        .map(|t| t.value().to_string());
    Ok(PdfDoc { page_count, title })
}

pub fn render_page(_doc: &PdfDoc, page: u32, target_w: u32, target_h: u32, _path_for_reopen: Option<&Path>) -> Result<(Vec<u8>, u32, u32), (String, String)> {
    check_page_px(target_w, target_h)?;
    let _ = check_bitmap_size(target_w, target_h)?;
    let pdfium = get_pdfium()?;
    let path = _path_for_reopen.ok_or(("malformed".to_string(), "no path for pdf render".to_string()))?;
    let doc = pdfium
        .load_pdf_from_file(path.to_str().unwrap_or(""), None)
        .map_err(|e| ("malformed".to_string(), format!("reopen: {}", e)))?;
    let p = doc
        .pages()
        .get(page as i32)
        .map_err(|e| ("malformed".to_string(), e.to_string()))?;
    let bitmap = p
        .render(target_w as i32, target_h as i32, None)
        .map_err(|e| ("malformed".to_string(), e.to_string()))?;
    let data = bitmap.as_rgba_bytes().to_vec();
    let w = bitmap.width() as u32;
    let h = bitmap.height() as u32;
    Ok((data, w, h))
}
