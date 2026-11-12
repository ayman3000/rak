//! `rak keys`: creates ~/.config/rak/.env from the template when it doesn't
//! exist yet, and shows which keys are set and where each comes from.
//! `rak keys --update` moves an older file to the current template, keeping
//! every value. Values are never printed.

use super::settings::{global_key_file, home, parse_env, shows_value, Keys, KNOWN, TEMPLATE};
use super::welcome::short_path;
use std::path::{Path, PathBuf};

const USAGE: &str = "rak keys — create ~/.config/rak/.env from a template, and show which keys are set

usage: rak keys            create the file if there is none; show every provider and its key
       rak keys --update   bring an older file up to the current template, keeping your values
                           (the old file is saved next to it as .env.bak)";

pub fn run(args: &[String]) -> ! {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        std::process::exit(0);
    }
    let updating = match args {
        [] => false,
        [flag] if flag == "--update" => true,
        _ => {
            eprintln!("rak: unknown arguments for rak keys\n\n{USAGE}");
            std::process::exit(2);
        }
    };
    let Some(file) = global_key_file() else {
        eprintln!("rak: no home folder to keep keys in; set them in the environment instead");
        std::process::exit(1);
    };
    let short = |path: &str| short_path(path, &home(), 60);
    let shown = short(&file.to_string_lossy());
    if !file.exists() {
        if let Err(e) = write_private(&file, TEMPLATE) {
            eprintln!("rak: cannot create {}: {e}", file.display());
            std::process::exit(1);
        }
        println!("Created {shown} — open it in any editor and fill in the keys you use.");
    } else if updating {
        match update_file(&file) {
            Ok(None) => println!("{shown} is already up to date."),
            Ok(Some((backup, updated, notes))) => {
                println!("Updated {shown} to the current template. Your old file is {}.", short(&backup.to_string_lossy()));
                if !updated.filled.is_empty() {
                    println!("  On their lines: {}", updated.filled.join(", "));
                }
                if !updated.kept.is_empty() {
                    println!("  Kept at the end: {}", updated.kept.join(", "));
                }
                for note in notes {
                    println!("  Note: {note}");
                }
            }
            Err(e) => {
                eprintln!("rak: cannot update {}: {e}", file.display());
                std::process::exit(1);
            }
        }
    } else {
        println!("Keys file: {shown}");
        if std::fs::read_to_string(&file).is_ok_and(|text| is_older(&text)) {
            println!("It is from an older template: `rak keys --update` brings it up to date and keeps your values.");
        }
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

/// Writes a file readable by its owner only (it is for keys): to a temporary
/// file first, then renamed over, so a failed write never leaves half a file.
fn write_private(file: &Path, text: &str) -> std::io::Result<()> {
    if let Some(folder) = file.parent() {
        std::fs::create_dir_all(folder)?;
    }
    let temporary = file.with_extension("new");
    std::fs::write(&temporary, text)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&temporary, file)
}

/// Whether a key file lacks lines the current template has: a provider's key
/// added since it was made.
fn is_older(text: &str) -> bool {
    let present = parse_env(text);
    rak::providers::all().iter().filter_map(|p| p.key.as_ref()).any(|key| !present.contains_key(key))
}

/// Saves the old file as .env.bak (or .env.bak.2, …) and writes the updated
/// one. None when there was nothing to change.
fn update_file(file: &Path) -> std::io::Result<Option<(PathBuf, Updated, Vec<String>)>> {
    let old_text = std::fs::read_to_string(file)?;
    let old = parse_env(&old_text);
    let updated = update(TEMPLATE, &old);
    if updated.text == old_text {
        return Ok(None);
    }
    let backup = (1..)
        .map(|n| if n == 1 { file.with_extension("bak") } else { file.with_extension(format!("bak.{n}")) })
        .find(|path| !path.exists())
        .expect("a free name");
    std::fs::copy(file, &backup)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&backup, std::fs::Permissions::from_mode(0o600))?;
    }
    write_private(file, &updated.text)?;
    Ok(Some((backup, updated, update_notes(&old))))
}

// MARK: - Updating a key file to the current template

/// The new key file, and what went where.
#[derive(Debug, PartialEq)]
pub struct Updated {
    pub text: String,
    /// Keys whose values went into the template's own lines.
    pub filled: Vec<String>,
    /// Values the template has no line for, kept in a section at the end.
    pub kept: Vec<String>,
}

/// Pure — unit-tested. The current template with the old file's values put
/// back: each value goes on the template's line for that key; any other value
/// (the defaults, a key rak no longer lists) goes into a section at the end,
/// so nothing that worked is lost. Empty values are dropped.
pub fn update(template: &str, old: &std::collections::HashMap<String, String>) -> Updated {
    let mut filled = Vec::new();
    let mut lines: Vec<String> = Vec::new();
    for line in template.lines() {
        let assignment = (!line.trim_start().starts_with('#')).then(|| line.split_once('=')).flatten();
        let Some((key, rest)) = assignment else {
            lines.push(line.to_string());
            continue;
        };
        let key = key.trim();
        match old.get(key).filter(|v| !v.is_empty()) {
            Some(value) => {
                let comment = rest.find('#').map(|at| rest[at..].trim_end()).unwrap_or("");
                let line = format!("{key}={}  {comment}", quote(value));
                lines.push(line.trim_end().to_string());
                filled.push(key.to_string());
            }
            None => lines.push(line.to_string()),
        }
    }
    let mut kept: Vec<String> = old.iter().filter(|(k, v)| !v.is_empty() && !filled.contains(k)).map(|(k, _)| k.clone()).collect();
    kept.sort();
    let mut text = lines.join("\n");
    if !kept.is_empty() {
        text.push_str(
            "\n\n\n# ─── Kept from your old file ───────────────────────────────────────────────\n\
             # Values the template has no line of its own for (your defaults among\n\
             # them). They work as they are; move them up into the sections above if\n\
             # you like.\n",
        );
        for key in &kept {
            text.push_str(&format!("{key}={}\n", quote(&old[key])));
        }
    }
    if !text.ends_with('\n') {
        text.push('\n');
    }
    Updated { text, filled, kept }
}

/// A value as it must be written to read back the same: quoted when it has
/// spaces, a `#` or a leading quote.
fn quote(value: &str) -> String {
    let plain = !value.contains(char::is_whitespace) && !value.contains('#') && !value.starts_with(['"', '\'']);
    match (plain, value.contains('\'')) {
        (true, _) => value.to_string(),
        (false, false) => format!("'{value}'"),
        (false, true) => format!("\"{value}\""),
    }
}

/// Pure — unit-tested. What changed meaning since an older template, by name
/// only (values are never shown).
pub fn update_notes(old: &std::collections::HashMap<String, String>) -> Vec<String> {
    let set = |name: &str| old.get(name).filter(|v| !v.is_empty());
    let mut notes = Vec::new();
    if let Some(provider) = set("RAK_PROVIDER") {
        match rak::providers::find(provider) {
            None => notes.push(format!("RAK_PROVIDER names a provider rak doesn't know. The names are: {}.", rak::providers::names())),
            Some(p) if p.name == "openai" && set("RAK_BASE_URL").is_some_and(|url| !url.contains("api.openai.com")) => notes.push(
                "RAK_PROVIDER=openai now means OpenAI itself, and your RAK_BASE_URL is another server. Set RAK_PROVIDER to that \
                 server's name instead (`rak keys` lists them) and remove RAK_BASE_URL."
                    .to_string(),
            ),
            Some(_) => {}
        }
    }
    if set("RAK_API_KEY").is_some() {
        notes.push(
            "RAK_API_KEY now goes only to a server named with --base-url. If it is a provider's key (OpenRouter, \
             OpenAI, …), move it to that provider's line."
                .to_string(),
        );
    }
    notes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::settings::{parse_env, TEMPLATE};
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn values_go_back_on_their_lines_and_the_rest_is_kept() {
        let old = env(&[
            ("ANTHROPIC_API_KEY", "sk-ant-1"),
            ("TAVILY_API_KEY", "tvly-2"),
            ("OPENAI_API_KEY", ""),
            ("RAK_MODEL", "kimi-k2.6:cloud"),
            ("SOMETHING_ELSE", "a value # with a hash"),
        ]);
        let updated = update(TEMPLATE, &old);
        assert_eq!(updated.filled, ["ANTHROPIC_API_KEY", "TAVILY_API_KEY", "RAK_MODEL"], "the model has a line of its own");
        assert_eq!(updated.kept, ["SOMETHING_ELSE"]);
        assert!(updated.text.contains("ANTHROPIC_API_KEY=sk-ant-1  # anthropic"), "the line keeps its comment");

        // Read back, the new file holds exactly what the old one set.
        let read = parse_env(&updated.text);
        let set: HashMap<String, String> = read.into_iter().filter(|(_, v)| !v.is_empty()).collect();
        let wanted: HashMap<String, String> = old.into_iter().filter(|(_, v)| !v.is_empty()).collect();
        assert_eq!(set, wanted);
    }

    #[test]
    fn updating_twice_changes_nothing_more() {
        let old = env(&[("OPENROUTER_API_KEY", "sk-or"), ("RAK_PROVIDER", "openrouter")]);
        let once = update(TEMPLATE, &old);
        let twice = update(TEMPLATE, &parse_env(&once.text));
        assert_eq!(once.text, twice.text);
    }

    #[test]
    fn an_empty_file_gives_the_template() {
        assert_eq!(update(TEMPLATE, &HashMap::new()).text.trim_end(), TEMPLATE.trim_end());
    }

    #[test]
    fn changed_meanings_are_pointed_out_by_name() {
        let notes =
            update_notes(&env(&[("RAK_PROVIDER", "openai"), ("RAK_BASE_URL", "https://openrouter.ai/api/v1"), ("RAK_API_KEY", "k")]));
        assert_eq!(notes.len(), 2);
        assert!(notes[0].contains("RAK_PROVIDER=openai now means OpenAI itself"));
        assert!(notes.iter().all(|n| !n.contains("openrouter.ai") && !n.contains("=k")), "no values in the notes");
        assert!(update_notes(&env(&[("RAK_PROVIDER", "nope")]))[0].contains("doesn't know"));
        assert!(update_notes(&env(&[("RAK_PROVIDER", "groq")])).is_empty());
    }
}
