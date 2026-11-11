//! `rak keys`: creates ~/.config/rak/.env from the template when it doesn't
//! exist yet, and shows which keys are set and where each comes from. Values
//! are never printed.

use super::settings::{global_key_file, home, Keys, KNOWN, TEMPLATE};
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
    println!();
    for (name, purpose) in KNOWN {
        let status = match keys.source(name) {
            Some(source) => format!("set, from {}", if source.starts_with('/') { short(&source) } else { source }),
            None => "not set".to_string(),
        };
        println!("  {name:<18} {status:<36} {purpose}");
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
