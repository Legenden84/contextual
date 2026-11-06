//! Image files that can be sent to a vision model as pictures (port of `fs/images.ts`).

/// Azure OpenAI's per-image upload limit.
pub const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;

/// Tokens a picture costs at high detail (OpenAI/Azure vision); used when dimensions are unknown.
pub const IMAGE_TOKENS_ESTIMATE: u64 = 765;

fn extension(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}

/// Formats Azure OpenAI vision accepts (`/\.(png|jpe?g|gif|webp)$/i`).
pub fn is_image(name: &str) -> bool {
    name.contains('.')
        && matches!(
            extension(name).to_ascii_lowercase().as_str(),
            "png" | "jpg" | "jpeg" | "gif" | "webp"
        )
}

pub fn image_mime(name: &str) -> &'static str {
    match extension(name).to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_names() {
        assert!(is_image("a/b/logo.PNG"));
        assert!(is_image("x.jpeg") && is_image("x.jpg") && is_image("x.webp") && is_image("x.gif"));
        assert!(!is_image("x.svg") && !is_image("png") && !is_image("x.png.txt"));
        assert_eq!(image_mime("A.JPG"), "image/jpeg");
        assert_eq!(image_mime("x.bmp"), "application/octet-stream");
        assert_eq!(image_mime("png"), "image/png");
    }
}
