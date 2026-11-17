//! Adding one key without opening the key file: `rak keys set NAME`, `/key`
//! in a chat, and `/provider` when the provider picked has no key yet.
//!
//! The key is typed or pasted without being shown, checked where the provider
//! has a check that costs nothing, and written to ~/.config/rak/.env (readable
//! by its owner only), on its own line, the rest of the file kept. A key that
//! replaces another leaves the old file next to it as .env.bak. The key is
//! never an argument (it would stay in the shell's history), never printed,
//! and never part of a conversation, a saved chat or a log.

use super::keys_command::{backup_path, write_private};
use super::picker::{self, Choice, Picked};
use super::settings::{global_key_file, home, parse_env, TEMPLATE};
use super::setup::{ask_yes, check_key, read_hidden, set_lines, Check};
use super::style::{paint, Style};
use super::welcome::short_path;
use std::io::Write;

/// A key rak can be given: a provider's, or web search's.
#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    /// The setting's name, e.g. OPENROUTER_API_KEY.
    pub key: String,
    /// Who it is for, e.g. "OpenRouter".
    pub label: String,
    /// An address that says whether a key is good, free of charge.
    pub check: Option<String>,
}

const TAVILY: (&str, &str) = ("TAVILY_API_KEY", "web search (Tavily)");

/// Every key that can be given: each provider's (one row per key, so Z.ai's
/// two providers share theirs), then web search's.
pub fn targets() -> Vec<Target> {
    let mut out: Vec<Target> = Vec::new();
    for p in rak::providers::all() {
        let Some(key) = &p.key else { continue };
        match out.iter_mut().find(|t| &t.key == key) {
            Some(same) => same.label = format!("{}, {}", same.label, p.name),
            None => out.push(Target { key: key.clone(), label: p.label.clone(), check: p.key_check.clone() }),
        }
    }
    out.push(Target { key: TAVILY.0.to_string(), label: TAVILY.1.to_string(), check: None });
    out
}

/// Pure — unit-tested. The key a word names: a provider ("openrouter"), the
/// key's own name ("OPENROUTER_API_KEY"), or "tavily".
pub fn target(word: &str) -> Result<Target, String> {
    let word = word.trim();
    if word.eq_ignore_ascii_case("tavily") || word.eq_ignore_ascii_case("web_search") {
        return Ok(Target { key: TAVILY.0.to_string(), label: TAVILY.1.to_string(), check: None });
    }
    if let Some(found) = targets().into_iter().find(|t| t.key.eq_ignore_ascii_case(word)) {
        return Ok(found);
    }
    match rak::providers::find(word) {
        Some(p) => match &p.key {
            Some(key) => Ok(Target { key: key.clone(), label: p.label.clone(), check: p.key_check.clone() }),
            None => Err(format!("{} needs no key.", p.label)),
        },
        None => Err(format!("No provider named `{word}`. They are: {}, and tavily for web search.", rak::providers::names())),
    }
}

/// Lets the person pick which key to give; each row says whether it is set.
/// None when they cancel (Esc).
pub fn pick(set_at: &dyn Fn(&str) -> Option<String>, color: bool) -> Option<Target> {
    let all = targets();
    let choices: Vec<Choice> = all
        .iter()
        .map(|t| {
            let state = match set_at(&t.key) {
                Some(_) => "set",
                None => "not set",
            };
            Choice { name: t.key.clone(), note: format!("{} — {state}", t.label), dim: false }
        })
        .collect();
    match picker::pick(&choices, None, false, color) {
        Picked::Item(i) => Some(all[i].clone()),
        _ => None,
    }
}

fn ask_no(question: &str, color: bool) -> bool {
    eprint!("{}", paint(color, Style::Bold, question));
    let _ = std::io::stderr().flush();
    let mut typed = String::new();
    let _ = std::io::stdin().read_line(&mut typed);
    super::chain::is_yes(typed.trim())
}

/// Pure — unit-tested. The key file's text with this key on its line, and
/// whether a value was there before (so the old file is worth keeping).
pub fn with_key(old: Option<&str>, name: &str, value: &str) -> (String, bool) {
    let replaced = old.is_some_and(|text| parse_env(text).get(name).is_some_and(|v| !v.is_empty()));
    (set_lines(old.unwrap_or(TEMPLATE), &[(name, value)]), replaced)
}

/// Asks for the key (not shown), checks it where that is free, and saves it.
/// `set_at`: where it is set now, if it is; replacing asks first.
/// The key itself comes back for this run to use; None when nothing changed.
pub fn add(target: &Target, set_at: Option<String>, color: bool) -> Option<String> {
    let file = match global_key_file() {
        Some(file) => file,
        None => {
            eprintln!("rak: no home folder to keep keys in; set {} in the environment instead", target.key);
            return None;
        }
    };
    let shown = short_path(&file.to_string_lossy(), &home(), 60);
    let name = &target.key;
    if let Some(at) = &set_at {
        let at = if at.starts_with('/') { short_path(at, &home(), 60) } else { at.clone() };
        if !ask_no(&format!("{name} is already set (in {at}). Replace it? [y/N] › "), color) {
            println!("{}", paint(color, Style::Dim, "Kept it."));
            return None;
        }
    }
    let key = loop {
        let typed = read_hidden(&format!("Paste your {name} for {} (it is not shown), then Enter › ", target.label), color)?;
        println!("{}", paint(color, Style::Dim, "Key received."));
        match target.check.as_deref().map(|url| check_key(url, &typed)) {
            Some(Check::Refused(code)) => {
                println!("{}", paint(color, Style::Yellow, &format!("{} did not accept that key ({code}).", target.label)));
                // "n" saves it as it is: a check is a help, never a lock.
                if ask_yes("Type it again? [Y/n] › ", color) {
                    continue;
                }
                println!("{}", paint(color, Style::Dim, "Saving it as it is."));
            }
            Some(Check::Unknown(why)) => println!("{}", paint(color, Style::Dim, &format!("Couldn't check the key ({why}); saving it."))),
            Some(Check::Accepted) => println!("{}", paint(color, Style::Dim, &format!("{} accepted it.", target.label))),
            None => {}
        }
        break typed;
    };
    let old = std::fs::read_to_string(&file).ok();
    let (text, replaced) = with_key(old.as_deref(), name, &key);
    let mut kept = String::new();
    if let (true, Some(old)) = (replaced, &old) {
        let backup = backup_path(&file);
        match write_private(&backup, old) {
            Ok(()) => kept = format!(" The old file is {}.", short_path(&backup.to_string_lossy(), &home(), 60)),
            Err(e) => {
                eprintln!("{}", paint(color, Style::Yellow, &format!("rak: cannot keep the old file as {}: {e}", backup.display())));
                return None;
            }
        }
    }
    if let Err(e) = write_private(&file, &text) {
        eprintln!("{}", paint(color, Style::Yellow, &format!("rak: cannot save {}: {e}", file.display())));
        return None;
    }
    println!("{}", paint(color, Style::Green, &format!("✓ Saved {name} in {shown}.{kept}")));
    if set_at.as_deref() == Some("the environment") {
        println!("{}", paint(color, Style::Dim, &format!("{name} is also set in your environment, which wins when rak starts.")));
    }
    Some(key)
}

const USAGE: &str = "rak keys set — give a key without opening the key file

usage: rak keys set NAME   NAME is a provider (openrouter, anthropic, …), a key's own name
                           (OPENROUTER_API_KEY), or tavily for web search
       rak keys set        pick which key from a list

The key is pasted without being shown and saved in ~/.config/rak/.env. It is never
given on the command line, so it does not stay in your shell's history.";

/// `rak keys set [NAME]`.
pub fn command(args: &[String]) -> ! {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        std::process::exit(0);
    }
    if args.len() > 1 {
        eprintln!("rak: rak keys set takes one name; the key itself is asked for, never given here\n\n{USAGE}");
        std::process::exit(2);
    }
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() {
        eprintln!("rak: rak keys set asks for the key, so it needs a terminal. Without one, set the key in the environment or in ~/.config/rak/.env.");
        std::process::exit(2);
    }
    let color = std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    let cwd = std::env::current_dir().unwrap_or_default();
    let keys = super::settings::Keys::load(&cwd, super::trust::is_trusted(&cwd) || super::trust::project_settings(&cwd).is_empty());
    let set_at = |name: &str| keys.source(name);
    let chosen = match args.first() {
        Some(word) => target(word).unwrap_or_else(|why| {
            eprintln!("rak: {why}");
            std::process::exit(2);
        }),
        None => {
            println!("{}", paint(color, Style::Bold, "Which key?"));
            match pick(&set_at, color) {
                Some(target) => target,
                None => {
                    println!("Nothing changed.");
                    std::process::exit(1);
                }
            }
        }
    };
    match add(&chosen, set_at(&chosen.key), color) {
        Some(_) => std::process::exit(0),
        None => {
            println!("Nothing changed.");
            std::process::exit(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_named_by_its_provider_its_own_name_or_tavily() {
        assert_eq!(target("openrouter").unwrap().key, "OPENROUTER_API_KEY");
        assert_eq!(target(" OpenRouter ").unwrap().key, "OPENROUTER_API_KEY");
        assert_eq!(target("openrouter_api_key").unwrap().key, "OPENROUTER_API_KEY");
        assert!(target("openrouter").unwrap().check.is_some(), "its free check comes along");
        assert_eq!(target("huggingface").unwrap().key, "HF_TOKEN");
        assert_eq!(target("tavily").unwrap().key, "TAVILY_API_KEY");
        assert!(target("ollama").unwrap_err().contains("needs no key"));
        assert!(target("nope").unwrap_err().contains("No provider named"));
    }

    #[test]
    fn each_key_is_listed_once() {
        let all = targets();
        let mut names: Vec<&str> = all.iter().map(|t| t.key.as_str()).collect();
        let count = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), count, "no key twice");
        assert!(all.iter().any(|t| t.key == "ZAI_API_KEY" && t.label.contains("zai-coding")), "both Z.ai providers on one row");
        assert_eq!(all.last().unwrap().key, "TAVILY_API_KEY");
    }

    #[test]
    fn a_key_goes_on_its_own_line_and_a_replaced_one_is_noticed() {
        let (text, replaced) = with_key(None, "GROQ_API_KEY", "gsk-1");
        assert!(!replaced && parse_env(&text).get("GROQ_API_KEY").map(String::as_str) == Some("gsk-1"));
        assert!(text.contains("GROQ_API_KEY=gsk-1") && text.contains("# groq"), "on the template's line, comment kept:\n{text}");
        let (text, replaced) = with_key(Some("A=1\nGROQ_API_KEY=old\n"), "GROQ_API_KEY", "new");
        assert!(replaced);
        assert_eq!(text, "A=1\nGROQ_API_KEY=new\n");
        let (_, replaced) = with_key(Some("GROQ_API_KEY=\n"), "GROQ_API_KEY", "new");
        assert!(!replaced, "an empty line held nothing worth keeping");
    }
}
