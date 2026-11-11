//! What rak keeps between runs: the folder, and the model picked last.

use std::path::PathBuf;

/// Where rak keeps the history and the model picked last.
pub fn config_dir() -> Option<PathBuf> {
    let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    var("XDG_CONFIG_HOME").or_else(|| var("HOME").map(|home| home.join(".config"))).or_else(|| var("APPDATA")).map(|base| base.join("rak"))
}

pub fn home() -> String {
    std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_default()
}

pub fn saved_model() -> Option<String> {
    let text = std::fs::read_to_string(config_dir()?.join("model")).ok()?;
    Some(text.trim().to_string()).filter(|m| !m.is_empty())
}

pub fn save_model(model: &str) {
    if let Some(dir) = config_dir() {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("model"), format!("{model}\n"));
    }
}
