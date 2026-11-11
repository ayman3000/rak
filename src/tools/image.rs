//! view_image: shows the model an image file, a screenshot above all, so it
//! can check what is on a screen instead of assuming. Offered only to models
//! that can see (see session.rs).

use super::{str_arg, Ctx, Image, Tool, ToolResult};
use crate::text::base64;
use serde_json::{json, Value};
use std::sync::Arc;

/// Model servers refuse bigger images (Anthropic's limit is 5 MB).
pub const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;

pub struct ViewImage(pub Arc<Ctx>);

impl Tool for ViewImage {
    fn name(&self) -> &str {
        "view_image"
    }
    fn description(&self) -> &str {
        "Look at an image file: a PNG, JPEG, GIF or WebP, such as a screenshot you took. The image is shown to you \
         after this call. Use it to check what a screen really shows before saying anything about it."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "path": { "type": "string", "description": "The image file (a leading ~ is expanded)." } },
            "required": ["path"]
        })
    }
    fn run(&self, args: &Value) -> ToolResult {
        let Some(raw) = str_arg(args, "path").filter(|p| !p.is_empty()) else {
            return ToolResult::err("view_image requires a `path`.");
        };
        let path = self.0.resolve(raw);
        if let Some(reason) = self.0.block_reason(&path) {
            return ToolResult::err(format!("Refusing to read {raw}: {reason}."));
        }
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) => return ToolResult::err(format!("Cannot read {raw}: {e}.")),
        };
        let Some(media_type) = media_type(&bytes) else {
            return ToolResult::err(format!("{raw} is not a PNG, JPEG, GIF or WebP image."));
        };
        if bytes.len() > MAX_IMAGE_BYTES {
            return ToolResult::err(format!(
                "{raw} is {:.1} MB; model servers take images up to 5 MB. Make a smaller one (on a Mac: sips -Z 1600 {raw} --out small.png).",
                bytes.len() as f64 / 1_048_576.0
            ));
        }
        let size = png_size(&bytes).map(|(w, h)| format!("{w}×{h}, ")).unwrap_or_default();
        let mut result = ToolResult::ok(format!("{raw} ({size}{} KB) is shown to you below.", bytes.len() / 1024));
        result.image = Some(Image { media_type: media_type.to_string(), data: base64(&bytes) });
        result
    }
}

/// Pure — unit-tested. The image type, from the file's first bytes.
pub fn media_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// Pure — unit-tested. A PNG's width and height, from its header.
pub fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if media_type(bytes) != Some("image/png") || bytes.len() < 24 {
        return None;
    }
    let read = |at: usize| u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    Some((read(16), read(20)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::testing::temp_dir;

    /// A 2×3 PNG's first 24 bytes (signature and the start of IHDR), enough for these checks.
    fn png_header() -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR".to_vec();
        bytes.extend_from_slice(&2u32.to_be_bytes());
        bytes.extend_from_slice(&3u32.to_be_bytes());
        bytes
    }

    #[test]
    fn image_types_are_told_apart_by_their_first_bytes() {
        assert_eq!(media_type(&png_header()), Some("image/png"));
        assert_eq!(media_type(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("image/jpeg"));
        assert_eq!(media_type(b"GIF89a...."), Some("image/gif"));
        assert_eq!(media_type(b"RIFF\x00\x00\x00\x00WEBPVP8 "), Some("image/webp"));
        assert_eq!(media_type(b"%PDF-1.7"), None);
        assert_eq!(png_size(&png_header()), Some((2, 3)));
    }

    #[test]
    fn an_image_in_the_project_comes_back_attached() {
        let dir = temp_dir("view-image");
        let ctx = Ctx::new(&dir, false);
        std::fs::write(dir.join("shot.png"), png_header()).unwrap();
        std::fs::write(dir.join("notes.txt"), "not an image").unwrap();
        let tool = ViewImage(ctx.clone());
        assert!(!tool.needs_approval());

        let seen = tool.run(&json!({ "path": "shot.png" }));
        assert!(seen.ok && seen.text.starts_with("shot.png (2×3, 0 KB) is shown"), "{}", seen.text);
        let image = seen.image.unwrap();
        assert_eq!(image.media_type, "image/png");
        assert!(image.data_url().starts_with("data:image/png;base64,iVBORw0KGgo"));

        assert!(!tool.run(&json!({ "path": "notes.txt" })).ok);
        assert!(tool.run(&json!({ "path": "missing.png" })).text.contains("Cannot read"));
        assert!(tool.run(&json!({ "path": "/etc/hosts" })).text.contains("outside the project"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
