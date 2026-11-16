//! Ctrl+V (or Alt+V) in the chat: an image on the clipboard saved to a file,
//! which the request then carries (see vision.rs). Read with the system's own
//! tools: osascript on macOS, wl-paste or xclip on Linux, PowerShell on Windows.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Pasted images older than this are removed when a new one is saved.
const KEEP: Duration = Duration::from_secs(24 * 60 * 60);

/// Where pasted images go.
pub fn folder() -> PathBuf {
    std::env::temp_dir().join("rak-pasted")
}

/// Saves the clipboard's image and returns its path, or says why not.
pub fn save_image() -> Result<PathBuf, String> {
    let dir = folder();
    std::fs::create_dir_all(&dir).map_err(|e| format!("can't make {}: {e}", dir.display()))?;
    clean(&dir);
    let millis = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let file = dir.join(format!("paste-{millis}.png"));
    write_clipboard_png(&file)?;
    let bytes = std::fs::read(&file).unwrap_or_default();
    if bytes.is_empty() || rak::tools::image::media_type(&bytes).is_none() {
        let _ = std::fs::remove_file(&file);
        return Err("there is no image on the clipboard (copy one, or take a screenshot to the clipboard)".into());
    }
    Ok(file)
}

#[cfg(target_os = "macos")]
fn write_clipboard_png(file: &Path) -> Result<(), String> {
    // AppleScript reads the clipboard as PNG data and writes it to the file;
    // with no image there it says "none".
    let script = [
        "on run argv",
        "try",
        "set png to (the clipboard as «class PNGf»)",
        "on error",
        "return \"none\"",
        "end try",
        "set out to open for access (POSIX file (item 1 of argv)) with write permission",
        "set eof out to 0",
        "write png to out",
        "close access out",
        "return \"ok\"",
        "end run",
    ];
    let mut command = Command::new("osascript");
    for line in script {
        command.args(["-e", line]);
    }
    let done = command.arg(file).output().map_err(|e| format!("can't run osascript: {e}"))?;
    match String::from_utf8_lossy(&done.stdout).trim() {
        "ok" => Ok(()),
        _ => Err("there is no image on the clipboard (copy one, or take a screenshot with Ctrl+Cmd+Shift+4)".into()),
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn write_clipboard_png(file: &Path) -> Result<(), String> {
    // Wayland first, then X11.
    let tries: [(&str, &[&str]); 2] =
        [("wl-paste", &["--no-newline", "--type", "image/png"]), ("xclip", &["-selection", "clipboard", "-t", "image/png", "-o"])];
    let mut missing = Vec::new();
    for (program, args) in tries {
        match Command::new(program).args(args).output() {
            Ok(out) if out.status.success() && !out.stdout.is_empty() => {
                return std::fs::write(file, out.stdout).map_err(|e| format!("can't save the image: {e}"));
            }
            Ok(_) => return Err("there is no image on the clipboard".into()),
            Err(_) => missing.push(program),
        }
    }
    Err(format!("reading an image from the clipboard needs {} (install one of them)", missing.join(" or ")))
}

#[cfg(windows)]
fn write_clipboard_png(file: &Path) -> Result<(), String> {
    let script = "Add-Type -AssemblyName System.Windows.Forms; Add-Type -AssemblyName System.Drawing; \
        $i = [System.Windows.Forms.Clipboard]::GetImage(); \
        if ($i) { $i.Save($args[0], [System.Drawing.Imaging.ImageFormat]::Png); 'ok' } else { 'none' }";
    let done = Command::new("powershell")
        .args(["-NoProfile", "-STA", "-Command", script])
        .arg(file)
        .output()
        .map_err(|e| format!("can't run PowerShell: {e}"))?;
    match String::from_utf8_lossy(&done.stdout).trim() {
        "ok" => Ok(()),
        _ => Err("there is no image on the clipboard".into()),
    }
}

/// Removes pasted images older than a day.
fn clean(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let old = entry.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > KEEP);
        if old && entry.file_name().to_string_lossy().starts_with("paste-") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}
