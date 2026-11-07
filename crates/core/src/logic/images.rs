//! Pictures attached to a message (`lib/images.ts`): vision token estimate, and scaling plus
//! data-URL encoding with the `image` crate (replacing the browser canvas).

use std::io::Cursor;

use base64::Engine as _;
use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ImageFormat, imageops::FilterType};
use serde::{Deserialize, Serialize};

use super::js;
use crate::{Error, Result};

/// MIME types that can be sent.
pub const IMAGE_TYPES: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];

/// Maximum encoded source size, before decoding or resizing (20 MiB per attachment).
pub const MAX_SOURCE_BYTES: usize = 20 * 1024 * 1024;

/// Longest side sent; larger pictures are scaled down (the service would do so anyway).
pub const MAX_SIDE: u32 = 2048;
/// PNGs larger than this after scaling are re-encoded as JPEG.
const MAX_PNG_BYTES: f64 = 4.0 * 1024.0 * 1024.0;
/// Canvas `toDataURL` qualities used by the web app.
const JPEG_QUALITY: u8 = 92;
const JPEG_FALLBACK_QUALITY: u8 = 90;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    pub id: String,
    pub name: String,
    /// `data:` URL sent to the model.
    pub url: String,
    pub width: u32,
    pub height: u32,
    /// Estimated input tokens at high detail.
    pub tokens: u64,
}

/// Vision input cost at high detail (OpenAI / Azure OpenAI): fit within 2048×2048, scale the
/// short side to 768, then 170 tokens per 512px tile plus 85.
pub fn vision_tokens(width: u32, height: u32) -> u64 {
    let (mut w, mut h) = (f64::from(width), f64::from(height));
    let fit = (2048.0 / w.max(h)).min(1.0);
    w *= fit;
    h *= fit;
    let shrink = (768.0 / w.min(h)).min(1.0);
    w *= shrink;
    h *= shrink;
    85 + 170 * (w / 512.0).ceil() as u64 * (h / 512.0).ceil() as u64
}

/// Size after scaling the longest side down to [`MAX_SIDE`] (rounded like `Math.round`).
pub fn scaled_size(width: u32, height: u32) -> (u32, u32) {
    let scale = (f64::from(MAX_SIDE) / f64::from(width.max(height))).min(1.0);
    let r = |v: u32| (js::round(f64::from(v) * scale) as u32).max(1);
    (r(width), r(height))
}

/// `screenshot-HHMMSS.png` for pasted pictures without a real name, else the file name.
pub fn attachment_name(file_name: &str, now: time::Time) -> String {
    if !file_name.is_empty() && file_name != "image.png" {
        return file_name.to_owned();
    }
    format!(
        "screenshot-{:02}{:02}{:02}.png",
        now.hour(),
        now.minute(),
        now.second()
    )
}

fn data_url(img: &DynamicImage, jpeg: Option<u8>) -> Result<String> {
    let mut bytes = Vec::new();
    let mime = match jpeg {
        Some(quality) => {
            // JPEG has no alpha; like a canvas, transparent pixels lose it.
            let rgb = img.to_rgb8();
            JpegEncoder::new_with_quality(&mut bytes, quality)
                .encode_image(&rgb)
                .map_err(|e| Error::internal(format!("JPEG encoding failed: {e}")))?;
            "image/jpeg"
        }
        None => {
            img.write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
                .map_err(|e| Error::internal(format!("PNG encoding failed: {e}")))?;
            "image/png"
        }
    };
    Ok(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

/// Reads an image file into an attachment, scaling it down if needed. Screenshots stay PNG (sharp
/// text); JPEGs stay JPEG; very large PNGs become JPEG. `name` is the final display name (see
/// [`attachment_name`]).
pub fn to_attachment(bytes: &[u8], mime: &str, name: &str, id: &str) -> Result<Attachment> {
    if !IMAGE_TYPES.contains(&mime) {
        return Err(Error::invalid(format!(
            "{name}: only PNG, JPEG, GIF and WebP pictures can be sent."
        )));
    }
    if bytes.len() > MAX_SOURCE_BYTES {
        return Err(Error::invalid(format!(
            "{name}: images must be 20 MiB or smaller."
        )));
    }
    let img = image::load_from_memory(bytes)
        .map_err(|_| Error::invalid(format!("Not a readable image: {name}")))?;
    let (width, height) = scaled_size(img.width(), img.height());
    let img = if (width, height) == (img.width(), img.height()) {
        img
    } else {
        img.resize_exact(width, height, FilterType::Triangle)
    };
    let mut url = data_url(&img, (mime == "image/jpeg").then_some(JPEG_QUALITY))?;
    if url.len() as f64 * 0.75 > MAX_PNG_BYTES {
        url = data_url(&img, Some(JPEG_FALLBACK_QUALITY))?;
    }
    Ok(Attachment {
        id: id.to_owned(),
        name: name.to_owned(),
        url,
        width,
        height,
        tokens: vision_tokens(width, height),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens() {
        assert_eq!(vision_tokens(1024, 1024), 765);
        assert_eq!(vision_tokens(1, 1), 255);
    }

    #[test]
    fn scaling_rounds_half_up() {
        assert_eq!(scaled_size(4096, 2049), (2048, 1025));
        assert_eq!(scaled_size(100, 50), (100, 50));
        assert_eq!(scaled_size(10_000, 1), (2048, 1));
        assert_eq!(scaled_size(1, 10_000), (1, 2048));
    }

    #[test]
    fn rejects_oversized_sources_before_decoding() {
        let bytes = vec![0; MAX_SOURCE_BYTES + 1];
        let error = to_attachment(&bytes, "image/png", "large.png", "1").unwrap_err();
        assert!(error.message.contains("20 MiB"));
    }

    #[test]
    fn encodes_png_and_scales() {
        let img = DynamicImage::new_rgba8(3000, 10);
        let mut png = Vec::new();
        img.write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
            .unwrap();
        let a = to_attachment(&png, "image/png", "wide.png", "1").unwrap();
        assert_eq!((a.width, a.height), (2048, 7));
        assert!(a.url.starts_with("data:image/png;base64,"));
        assert!(to_attachment(&png, "image/bmp", "x.bmp", "2").is_err());
        assert!(to_attachment(b"nope", "image/png", "x.png", "3").is_err());
    }

    #[test]
    fn names() {
        let t = time::Time::from_hms(9, 5, 7).unwrap();
        assert_eq!(attachment_name("image.png", t), "screenshot-090507.png");
        assert_eq!(attachment_name("", t), "screenshot-090507.png");
        assert_eq!(attachment_name("a.jpg", t), "a.jpg");
    }
}
