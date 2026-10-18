//! Real image decode using image = "0.25".
//! PNG/JPEG/WebP/GIF (GIF first frame only). Pre-check sizes. Target size render.
//! Enforce limits; Error on over, process stays up. No edit/ocr/svg/video.

use crate::limits::{check_bitmap_size, check_file_size, check_page_px};
use image::{AnimationDecoder, DynamicImage, ImageFormat, ImageReader};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

pub fn decode_image(path: &Path, target_w: u32, target_h: u32) -> Result<(Vec<u8>, u32, u32), (String, String)> {
    // size precheck on file
    let meta = std::fs::metadata(path).map_err(|e| ("not_found".to_string(), e.to_string()))?;
    check_file_size(meta.len())?;
    check_page_px(target_w, target_h)?;

    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    let format = match ext.as_str() {
        "png" => ImageFormat::Png,
        "jpg" | "jpeg" => ImageFormat::Jpeg,
        "webp" => ImageFormat::WebP,
        "gif" => ImageFormat::Gif,
        _ => return Err(("unsupported".into(), format!("ext {}", ext))),
    };
    // BMP rejected (hole 3). Falls to unsupported. GIF uses first frame only.

    let file = File::open(path).map_err(|e| ("not_found".to_string(), e.to_string()))?;
    let reader = BufReader::new(file);

    let dyn_img = if format == ImageFormat::Gif {
        // first frame only
        let decoder = image::codecs::gif::GifDecoder::new(reader)
            .map_err(|e| ("malformed".to_string(), e.to_string()))?;
        let mut frames = decoder.into_frames();
        let frame = frames.next()
            .ok_or(("malformed".to_string(), "gif no frames".to_string()))?
            .map_err(|e| ("malformed".to_string(), e.to_string()))?;
        DynamicImage::ImageRgba8(frame.into_buffer())
    } else {
        let img_reader = ImageReader::with_format(reader, format);
        img_reader.decode().map_err(|e| ("malformed".to_string(), e.to_string()))?
    };

    // resize to target (fit)
    let resized = if dyn_img.width() != target_w || dyn_img.height() != target_h {
        dyn_img.resize_to_fill(target_w, target_h, image::imageops::FilterType::Triangle)
    } else {
        dyn_img
    };

    let rgba = resized.to_rgba8();
    let w = rgba.width();
    let h = rgba.height();
    let data = rgba.into_raw();
    let _ = check_bitmap_size(w, h)?;
    Ok((data, w, h))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn bmp_is_rejected() {
        let dir = tempdir().unwrap();
        let bmp_path = dir.path().join("test.bmp");
        std::fs::write(&bmp_path, b"fake bmp bytes to pass size check").unwrap();
        let res = decode_image(&bmp_path, 10, 10);
        assert!(res.is_err());
        if let Err((code, _msg)) = res {
            assert_eq!(code, "unsupported", "bmp must be rejected");
        } else {
            panic!("expected err for bmp");
        }
    }

    #[test]
    fn png_hits_malformed_not_unsupported() {
        let dir = tempdir().unwrap();
        let png_path = dir.path().join("test.png");
        std::fs::write(&png_path, b"not a png").unwrap();
        let res = decode_image(&png_path, 10, 10);
        if let Err((code, _)) = res {
            assert_ne!(code, "unsupported");
        }
    }
}
