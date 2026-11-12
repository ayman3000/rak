//! `rak keys`: creates ~/.config/rak/.env from the template when it doesn't
//! exist yet, and shows which keys are set and where each comes from. Values
//! are never printed.

use super::settings::{global_key_file, home, shows_value, Keys, KNOWN, TEMPLATE};
use super::welcome::short_path;

pub fn run(args: &[String]) -> ! {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("rak keys — create ~/.config/rak/.env from a template, and show which keys are set\n\nusage: rak keys");
        std::process::exit(0);
    }
    let Some(file) = global_key_file() else {
        eprintln!("rak: no home folder to keep keys in; set them in the environment instead");
        std::process::exit(1);
    };
    let short = |path: &str| short_path(path, &home(), 60);
    let shown = short(&file.to_string_lossy());
    if file.exists() {
        println!("Keys file: {shown}");
    } else {
        if let Err(e) = create(&file) {
            eprintln!("rak: cannot create {}: {e}", file.display());
            std::process::exit(1);
        }
        println!("Created {shown} — open it in any editor and fill in the keys you use.");
    }
    let cwd = std::env::current_dir().unwrap_or_default();
    let keys = Keys::load(&cwd);
    let status = |name: &str| match keys.source(name) {
        Some(source) => format!("set, from {}", if source.starts_with('/') { short(&source) } else { source }),
        None => "not set".to_string(),
    };
    let default = keys.get("RAK_PROVIDER");
    println!("\nProviders (pick one with --provider NAME, or RAK_PROVIDER for the default)\n");
    for p in rak::providers::all() {
        let mark = if default.as_deref().is_some_and(|d| d.eq_ignore_ascii_case(&p.name)) { "▸" } else { " " };
        let key = match &p.key {
            Some(name) => format!("{name:<20} {}", status(name)),
            None => "no key needed".to_string(),
        };
        println!(" {mark} {:<13} {key:<52} {}", p.name, p.label);
    }
    println!("\nOther settings\n");
    for (name, purpose) in KNOWN {
        let state = match keys.get(name) {
            Some(value) if shows_value(name) => format!("{value} ({})", status(name)),
            _ => status(name),
        };
        println!("   {name:<16} {state:<50} {purpose}");
    }
    if default.is_none() {
        println!("\nNo RAK_PROVIDER: rak uses Ollama on this machine (or Anthropic for a claude-* model when its key is set).");
    }
    println!("\nThe environment wins, then .env in the project folder, then {shown}.");
    std::process::exit(0);
}

/// The template, readable by its owner only: it is for keys.
fn create(file: &std::path::Path) -> std::io::Result<()> {
    if let Some(folder) = file.parent() {
        std::fs::create_dir_all(folder)?;
    }
    std::fs::write(file, TEMPLATE)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}
