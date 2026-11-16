//! A vision model for a main model that can't see (Naseem's "describe"
//! mode): images the person attaches, and those `view_image` opens, go to the
//! vision model, and its description goes on to the main model as text.
//! Set with RAK_VISION_PROVIDER and RAK_VISION_MODEL (or --vision-provider,
//! --vision-model). A main model that sees gets the images itself.

use crate::provider::{Model, Usage};
use crate::tools::Image;
use serde_json::{json, Value};
use std::path::Path;

/// Images one message may carry (Naseem's limit).
pub const MAX_IMAGES: usize = 6;
/// What an image file may be called.
pub const EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp"];

/// Said when an image arrives, the main model can't see, and no vision model
/// is set. The request still goes, with the paths as written.
pub const HINT: &str = "This model can't read images and no vision model is set, so the images were not shown to it \
    (the request went with their paths). To have them read, add a model that can see to ~/.config/rak/.env and \
    start rak again:\n  RAK_VISION_PROVIDER=<provider>   (as for RAK_PROVIDER; its key goes in the same file)\n  \
    RAK_VISION_MODEL=<a model that reads images>\nThen images go to it, and its description to this model. Or switch \
    to a model that sees (/model).";

/// The vision model, and what it has spent since last asked.
pub struct Vision {
    model: Box<dyn Model + Send>,
    /// Shown in progress lines and told to the main model ("provider/model").
    pub name: String,
    /// The main model can't see, so images go through this one. Off when the
    /// chat moves to a model that sees.
    pub active: bool,
    usage: Usage,
}

impl Vision {
    pub fn new(model: Box<dyn Model + Send>, name: impl Into<String>) -> Self {
        Vision { model, name: name.into(), active: true, usage: Usage::default() }
    }

    /// The vision model's description of `images`, for what `context` asks
    /// (the person's message, or the question view_image was called with).
    pub fn describe(&mut self, images: &[Image], context: &str) -> Result<String, String> {
        let reply = self.model.chat(&[request(images, context)], &[]).map_err(|e| e.to_string())?;
        self.usage.add(reply.usage);
        let text = reply.content.trim();
        if text.is_empty() {
            return Err(format!("the vision model {} sent back no description", self.name));
        }
        Ok(text.to_string())
    }

    /// Tokens spent since the last call, to count in the request's usage.
    pub fn take_usage(&mut self) -> Usage {
        std::mem::take(&mut self.usage)
    }
}

/// Where images go.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Route {
    /// The main model sees them itself.
    Direct,
    /// The vision model describes them for the main model.
    Describe,
    /// Nobody can see them: tell the person how to set a vision model.
    Hint,
}

/// Pure — unit-tested. A main model known to see gets images directly; one
/// known not to goes through the vision model, or gets the hint without one.
/// When the server can't say, a vision model is used if set (it surely sees),
/// else the images are sent and the server decides.
pub fn route(main_sees: Option<bool>, vision_set: bool) -> Route {
    match (main_sees, vision_set) {
        (Some(true), _) => Route::Direct,
        (_, true) => Route::Describe,
        (Some(false), false) => Route::Hint,
        (None, false) => Route::Direct,
    }
}

/// Pure — unit-tested. What the vision model is asked.
pub fn prompt(context: &str, count: usize) -> String {
    let what = if count > 1 {
        format!("Describe these {count} images, in order, each under its own heading: [image 1], [image 2], and so on.")
    } else {
        "Describe this image.".to_string()
    };
    let mut prompt = format!(
        "You are the eyes of a coding assistant that cannot see images. {what} The assistant will act only on what \
         you write, so be complete and exact: copy every piece of text, code, error message, number, file name and URL \
         exactly as shown; describe the layout, the parts of any user interface and their state, colours where they \
         matter, and anything that looks wrong or out of place. Do not guess at what is not visible; say so when \
         something cannot be read."
    );
    let context = context.trim();
    if !context.is_empty() {
        prompt.push_str(&format!("\n\nWhat the user asked, so you know what matters (still describe everything relevant):\n{context}"));
    }
    prompt
}

/// The request to the vision model: the prompt, then the images.
pub fn request(images: &[Image], context: &str) -> Value {
    let mut parts = vec![json!({ "type": "text", "text": prompt(context, images.len()) })];
    parts.extend(images.iter().map(|image| json!({ "type": "image_url", "image_url": { "url": image.data_url() } })));
    json!({ "role": "user", "content": parts })
}

/// Pure — unit-tested. The person's message with the vision model's look at
/// their images after it, for a main model that can't see.
pub fn described_task(task: &str, count: usize, vision: &str, description: &str) -> String {
    let images = if count == 1 { "an image".to_string() } else { format!("{count} images") };
    format!(
        "{task}\n\n[The user attached {images}. You cannot see images, so the vision model {vision} looked at {} for \
         you; take this as your own look:]\n{description}",
        if count == 1 { "it" } else { "them" }
    )
}

/// Pure — unit-tested. The images a message names: `[image: PATH]` (what
/// Ctrl+V puts in), and, with `bare`, paths a terminal pastes when a file is
/// dragged onto it (quoted, or with `\ ` for spaces). rak passes `bare` only
/// when someone is at a terminal: an unattended task (CI, a benchmark) that
/// mentions an image path is not changed. A path counts only when it starts
/// at `/`, `~/`, `./`, `../` or a drive (`C:\`), ends in an image extension,
/// and `exists` says the file is there. Each becomes `[image N]` in the
/// text; past MAX_IMAGES the rest are left as written. Returns the text and
/// the paths, `~` expanded with `home`.
pub fn attachments(text: &str, home: &str, bare: bool, exists: &dyn Fn(&str) -> bool) -> (String, Vec<String>) {
    let bytes = text.as_bytes();
    let mut out = String::new();
    let mut paths: Vec<String> = Vec::new();
    let mut copied = 0; // text[..copied] is in `out`
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        // The span [i, end) and the path it may hold.
        let (end, candidate) = if text[i..].starts_with("[image: ") {
            match text[i..].find(']') {
                Some(close) => (i + close + 1, text[i + 8..i + close].trim().to_string()),
                None => (bytes.len(), String::new()),
            }
        } else if !bare {
            // Only `[image: …]` counts: on to the next word.
            let next = text[i..].find(char::is_whitespace).map_or(bytes.len(), |n| i + n);
            (next.max(i + 1), String::new())
        } else if c == b'\'' || c == b'"' {
            match text[i + 1..].find(c as char) {
                Some(close) => (i + 1 + close + 1, text[i + 1..i + 1 + close].to_string()),
                None => (bytes.len(), String::new()),
            }
        } else {
            let mut j = i;
            while j < bytes.len() && !bytes[j].is_ascii_whitespace() {
                // "\ " is an escaped space, part of the path (not on Windows paths).
                j += if bytes[j] == b'\\' && j + 1 < bytes.len() && bytes[j + 1] == b' ' { 2 } else { 1 };
            }
            // A sentence's punctuation after the path is not part of it.
            let mut end = j;
            while end > i && matches!(bytes[end - 1], b',' | b'.' | b';' | b':' | b')' | b'!' | b'?') {
                end -= 1;
            }
            let token = text[i..end].replace("\\ ", " ");
            (end.max(i + 1), token)
        };
        let end = end.min(bytes.len());
        let path = expand(&candidate, home);
        if paths.len() < MAX_IMAGES && is_image_path(&candidate) && exists(&path) {
            out.push_str(&text[copied..i]);
            paths.push(path);
            out.push_str(&format!("[image {}]", paths.len()));
            copied = end;
        }
        i = end;
        // Step to a character boundary (a token may end inside one).
        while i < bytes.len() && !text.is_char_boundary(i) {
            i += 1;
        }
    }
    out.push_str(&text[copied..]);
    (out, paths)
}

fn is_image_path(path: &str) -> bool {
    let rooted = path.starts_with('/')
        || path.starts_with("~/")
        || path.starts_with("./")
        || path.starts_with("../")
        || (path.len() > 2 && path.as_bytes()[1] == b':' && matches!(path.as_bytes()[2], b'\\' | b'/'));
    let extension = Path::new(path).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    rooted && EXTENSIONS.contains(&extension.as_str())
}

fn expand(path: &str, home: &str) -> String {
    match path.strip_prefix("~/") {
        Some(rest) if !home.is_empty() => format!("{}/{rest}", home.trim_end_matches('/')),
        _ => path.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_go_to_the_main_model_the_vision_model_or_the_hint() {
        assert_eq!(route(Some(true), true), Route::Direct, "a model that sees needs no help");
        assert_eq!(route(Some(true), false), Route::Direct);
        assert_eq!(route(Some(false), true), Route::Describe);
        assert_eq!(route(Some(false), false), Route::Hint);
        assert_eq!(route(None, true), Route::Describe, "unknown: the vision model surely sees");
        assert_eq!(route(None, false), Route::Direct, "unknown and no vision model: the server decides");
    }

    #[test]
    fn the_vision_prompt_asks_for_exact_text_and_carries_the_question() {
        let one = prompt("why does the build fail?", 1);
        assert!(one.contains("Describe this image.") && one.contains("exactly as shown"));
        assert!(one.ends_with("why does the build fail?"));
        let two = prompt("  ", 2);
        assert!(two.contains("these 2 images") && two.contains("[image 1]") && !two.contains("What the user asked"));
        let image = Image { media_type: "image/png".into(), data: "AAAA".into() };
        let req = request(&[image.clone(), image], "q");
        assert_eq!(req["content"].as_array().unwrap().len(), 3);
        assert_eq!(req["content"][1]["image_url"]["url"], "data:image/png;base64,AAAA");
    }

    #[test]
    fn the_description_follows_the_message_as_the_main_model_s_own_look() {
        let task = described_task("fix this [image 1]", 1, "ollama/qwen-vl", "A red error: E0308");
        assert!(task.starts_with("fix this [image 1]\n\n[The user attached an image."));
        assert!(task.contains("vision model ollama/qwen-vl looked at it") && task.ends_with("A red error: E0308"));
        assert!(described_task("x", 2, "v", "d").contains("2 images") && described_task("x", 2, "v", "d").contains("at them"));
    }

    #[test]
    fn pasted_and_dragged_image_paths_become_numbered_images() {
        let real = ["/tmp/shot.png", "/Users/me/My Shots/a b.JPG", "/home/me/x.webp", "/tmp/p/rak-paste-1.png", "./ui.gif"];
        let exists = |p: &str| real.contains(&p);
        // Ctrl+V's token, a dragged path with escaped spaces, a quoted path, ~.
        let (text, paths) = attachments(
            "why [image: /tmp/p/rak-paste-1.png] and /Users/me/My\\ Shots/a\\ b.JPG, or '/tmp/shot.png' ~/x.webp?",
            "/home/me",
            true,
            &exists,
        );
        assert_eq!(text, "why [image 1] and [image 2], or [image 3] [image 4]?");
        assert_eq!(paths, ["/tmp/p/rak-paste-1.png", "/Users/me/My Shots/a b.JPG", "/tmp/shot.png", "/home/me/x.webp"]);
        // Not files, not rooted, not images, or not there: left as written.
        let (text, paths) = attachments("edit logo.png and /tmp/notes.txt and /tmp/gone.png and ./ui.gif", "", true, &exists);
        assert_eq!(text, "edit logo.png and /tmp/notes.txt and /tmp/gone.png and [image 1]");
        assert_eq!(paths, ["./ui.gif"]);
        // An image alone, with no words.
        assert_eq!(attachments("/tmp/shot.png", "", true, &exists), ("[image 1]".to_string(), vec!["/tmp/shot.png".to_string()]));
        // At most MAX_IMAGES.
        let many = "/tmp/shot.png ".repeat(8);
        assert_eq!(attachments(&many, "", true, &exists).1.len(), MAX_IMAGES);
        // Text with no paths, and with non-ASCII, comes back unchanged.
        assert_eq!(attachments("héllo — “wörld”", "", true, &exists).0, "héllo — “wörld”");
        // Unattended: a task that mentions an image path is left alone; only
        // an explicit [image: …] is attached.
        let task = "Read /tmp/shot.png and write its text to /app/out.txt. Also [image: /tmp/p/rak-paste-1.png]";
        let (text, paths) = attachments(task, "", false, &exists);
        assert_eq!(text, "Read /tmp/shot.png and write its text to /app/out.txt. Also [image 1]");
        assert_eq!(paths, ["/tmp/p/rak-paste-1.png"]);
        assert_eq!(attachments("é /tmp/shot.png", "", false, &exists).1.len(), 0);
    }
}
