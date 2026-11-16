//! Small text helpers shared by the library's modules.

/// `text` on one line, whitespace runs folded, cut to `max` characters with "…".
/// Pure — unit-tested. A message's text: the string, or the text parts of a
/// content list (a request with images), images left out.
pub fn message_text(content: &serde_json::Value) -> Option<String> {
    match content {
        serde_json::Value::String(text) => Some(text.clone()),
        serde_json::Value::Array(parts) => {
            Some(parts.iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join("\n")).filter(|t| !t.is_empty())
        }
        _ => None,
    }
}

pub fn one_line(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > max {
        flat.chars().take(max).collect::<String>() + "…"
    } else {
        flat
    }
}

/// Standard base64 with padding, for images sent to a model.
pub fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { ALPHABET[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { ALPHABET[n as usize & 63] as char } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_with_images_still_has_its_text() {
        use serde_json::json;
        assert_eq!(message_text(&json!("plain")).as_deref(), Some("plain"));
        let parts = json!([{ "type": "text", "text": "why [image 1]?" }, { "type": "image_url", "image_url": { "url": "data:x" } }]);
        assert_eq!(message_text(&parts).as_deref(), Some("why [image 1]?"));
        assert_eq!(message_text(&json!([{ "type": "image_url" }])), None);
        assert_eq!(crate::sessions::title(&[json!({ "role": "user", "content": parts })]), "why [image 1]?");
    }

    #[test]
    fn base64_matches_the_standard() {
        for (input, expected) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foobar", "Zm9vYmFy"), ("rak!", "cmFrIQ==")] {
            assert_eq!(base64(input.as_bytes()), expected);
        }
        assert_eq!(base64(&[0xff, 0xfe, 0x00]), "//4A");
    }

    #[test]
    fn text_is_folded_onto_one_line_and_cut() {
        assert_eq!(one_line("a\n  b\tc", 10), "a b c");
        assert_eq!(one_line("abcdef", 3), "abc…");
        assert_eq!(one_line("日本語です", 2), "日本…");
    }
}
