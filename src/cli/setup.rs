//! First-run setup, and `rak setup` to run it again: pick a provider, paste
//! its key (not shown as it is typed), pick a model from the provider's own
//! list. The answers go into ~/.config/rak/.env (readable by its owner only),
//! the rest of that file kept as it was.
//!
//! A chat started with no key file and nothing chosen (no --provider, -m,
//! --base-url, RAK_PROVIDER or RAK_MODEL) runs it first. Scripts never see
//! it: without a terminal rak goes on as before and says what is missing.

use super::keys_command::{quote, write_private};
use super::picker::{self, Choice, Picked};
use super::settings::{global_key_file, home, parse_env, remember_model, TEMPLATE};
use super::style::{paint, Style};
use super::welcome::short_path;
use rak::providers::Provider;
use std::io::Write;
use std::time::Duration;

const USAGE: &str = "rak setup — pick a provider, give its key and pick a model

usage: rak setup     asks for each, then saves them in ~/.config/rak/.env
                     (other lines in that file are kept as they are)";

/// Pure — unit-tested. Whether a chat should start with the setup: nothing
/// is chosen anywhere and there is no key file yet.
pub fn needed(can_ask: bool, has_key_file: bool, chosen: bool) -> bool {
    can_ask && !has_key_file && !chosen
}

/// Pure — unit-tested. host and port of an http(s) address, for checking
/// whether a local server is running.
pub fn host_port(url: &str) -> Option<(String, u16)> {
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split('/').next()?;
    let default = if scheme == "https" { 443 } else { 80 };
    match authority.rsplit_once(':') {
        Some((host, port)) => Some((host.to_string(), port.parse().ok()?)),
        None => Some((authority.to_string(), default)),
    }
}

fn is_local(p: &Provider) -> bool {
    p.key.is_none()
}

fn running(p: &Provider) -> bool {
    let Some((host, port)) = host_port(&p.base_url) else { return false };
    let host = if host == "localhost" { "127.0.0.1".to_string() } else { host };
    let Ok(address) = format!("{host}:{port}").parse::<std::net::SocketAddr>() else { return false };
    std::net::TcpStream::connect_timeout(&address, Duration::from_millis(300)).is_ok()
}

/// One row of the provider list, before it is drawn.
#[derive(Debug, PartialEq)]
pub struct Row {
    pub index: usize,
    pub note: String,
    pub dim: bool,
}

/// Pure — unit-tested. The providers in the order offered: local servers that
/// are running first (no key needed), then the cloud providers as listed,
/// then the local servers that are not running, dim.
/// `key_at`: where a key is already set ("your environment", the key file), if it is.
pub fn rows(providers: &[Provider], up: &dyn Fn(&Provider) -> bool, key_at: &dyn Fn(&str) -> Option<String>) -> Vec<Row> {
    let mut first = Vec::new();
    let mut middle = Vec::new();
    let mut last = Vec::new();
    for (index, p) in providers.iter().enumerate() {
        match &p.key {
            None if up(p) => {
                first.push(Row { index, note: format!("{} — running on this computer, no key needed", p.label), dim: false })
            }
            None => last.push(Row { index, note: format!("{} — not running on this computer", p.label), dim: true }),
            Some(name) => match key_at(name) {
                Some(at) => middle.push(Row { index, note: format!("{} — {name} set in {at}", p.label), dim: false }),
                None => middle.push(Row { index, note: format!("{} — needs {name}", p.label), dim: false }),
            },
        }
    }
    first.into_iter().chain(middle).chain(last).collect()
}

/// Pure — unit-tested. `text` with each name set to its value: on the line
/// that sets it (its comment kept), or added at the end when no line does.
pub fn set_lines(text: &str, values: &[(&str, &str)]) -> String {
    let mut done = vec![false; values.len()];
    let mut lines: Vec<String> = Vec::new();
    for line in text.lines() {
        let assignment = (!line.trim_start().starts_with('#')).then(|| line.split_once('=')).flatten();
        let found = assignment.and_then(|(key, rest)| {
            let key = key.trim().trim_start_matches("export ").trim();
            values.iter().position(|(name, _)| *name == key).map(|i| (i, rest))
        });
        match found {
            Some((i, _)) if done[i] => {} // a later line for the same name: dropped, so one value is read
            Some((i, rest)) => {
                let (name, value) = values[i];
                // A comment after the value: only after whitespace, as parse_env reads it.
                let comment = rest.find(" #").map(|at| rest[at..].trim()).unwrap_or("");
                let line = format!("{name}={}  {comment}", quote(value));
                lines.push(line.trim_end().to_string());
                done[i] = true;
            }
            None => lines.push(line.to_string()),
        }
    }
    let missing: Vec<String> = values.iter().zip(&done).filter(|(_, d)| !**d).map(|((n, v), _)| format!("{n}={}", quote(v))).collect();
    if !missing.is_empty() {
        if lines.last().is_some_and(|l| !l.trim().is_empty()) {
            lines.push(String::new());
        }
        lines.extend(missing);
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// Pure — unit-tested. A key as typed or pasted: the bytes of a terminal's
/// escape sequences (bracketed paste) and anything not printable left out.
pub fn clean_key(bytes: &[u8]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b {
            // ESC [ … final byte
            i += 1;
            if bytes.get(i) == Some(&b'[') {
                i += 1;
                while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                    i += 1;
                }
            }
            i += 1;
            continue;
        }
        if bytes[i].is_ascii_graphic() {
            out.push(bytes[i] as char);
        }
        i += 1;
    }
    out
}

/// Reads a key without showing it. None: cancelled (Esc, Ctrl-C) or nothing typed.
fn read_hidden(prompt: &str, color: bool) -> Option<String> {
    eprint!("{}", paint(color, Style::Bold, prompt));
    let _ = std::io::stderr().flush();
    let Some(raw) = super::term::Raw::enter() else {
        // No raw mode: a plain line (shown as typed).
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).ok()?;
        let key = clean_key(line.trim().as_bytes());
        return (!key.is_empty()).then_some(key);
    };
    let mut bytes: Vec<u8> = Vec::new();
    let cancelled = loop {
        let Some(byte) = super::term::read_byte(None) else { break true };
        match byte {
            b'\r' | b'\n' => break false,
            3 => break true,
            0x1b => {
                // Esc alone cancels; an escape sequence (a paste's markers) is kept to be cleaned.
                match super::term::read_byte(Some(40)) {
                    None => break true,
                    Some(next) => bytes.extend([0x1b, next]),
                }
            }
            127 | 8 => {
                bytes.pop();
            }
            other => bytes.push(other),
        }
    };
    drop(raw);
    eprintln!();
    if cancelled {
        return None;
    }
    let key = clean_key(&bytes);
    (!key.is_empty()).then_some(key)
}

fn ask_yes(question: &str, color: bool) -> bool {
    eprint!("{}", paint(color, Style::Bold, question));
    let _ = std::io::stderr().flush();
    let mut typed = String::new();
    let _ = std::io::stdin().read_line(&mut typed);
    let typed = typed.trim();
    typed.is_empty() || super::chain::is_yes(typed)
}

/// Asks for the provider, its key and a model, and saves them. None when
/// cancelled; nothing is written then.
pub fn run(color: bool) -> Option<()> {
    let file = global_key_file()?;
    let shown = short_path(&file.to_string_lossy(), &home(), 60);
    let saved = std::fs::read_to_string(&file).map(|t| parse_env(&t)).unwrap_or_default();
    let in_env = |name: &str| std::env::var(name).is_ok_and(|v| !v.trim().is_empty());
    let has_key = |name: &str| in_env(name) || saved.get(name).is_some_and(|v| !v.is_empty());

    println!("{}", paint(color, Style::Bold, "Let's set rak up: a provider, its key, and a model."));
    println!("{}", paint(color, Style::Dim, "↑/↓ to move, type to narrow, Enter to pick, Esc to stop."));
    let providers = rak::providers::all();
    let key_at = |name: &str| {
        if in_env(name) {
            Some("your environment".to_string())
        } else {
            has_key(name).then(|| shown.clone())
        }
    };
    let rows = rows(providers, &running, &key_at);
    let choices: Vec<Choice> =
        rows.iter().map(|r| Choice { name: providers[r.index].name.clone(), note: r.note.clone(), dim: r.dim }).collect();
    let provider = match picker::pick(&choices, Some(0), false, color) {
        Picked::Item(i) => &providers[rows[i].index],
        _ => return None,
    };

    // The key: one already set is used as it is; otherwise asked for. After
    // a key that did not work, asked for again.
    let mut ask_key = false;
    loop {
        let mut typed_key: Option<String> = None;
        let api_key = match &provider.key {
            None => None,
            Some(name) if !ask_key && in_env(name) => {
                println!("Using {name} from your environment.");
                std::env::var(name).ok()
            }
            Some(name) if !ask_key && has_key(name) => {
                println!("Using the {name} already in {shown}.");
                saved.get(name).cloned()
            }
            Some(name) => {
                let typed = read_hidden(&format!("Paste your {name} (it is not shown), then Enter › "), color)?;
                println!("{}", paint(color, Style::Dim, "Key received."));
                typed_key = Some(typed.clone());
                Some(typed)
            }
        };
        // Where the model list answers anyone, the key is checked first.
        if let (Some(url), Some(key)) = (&provider.key_check, &api_key) {
            match check_key(url, key) {
                Check::Accepted => {}
                Check::Refused(code) => {
                    println!("{}", paint(color, Style::Yellow, &format!("{} did not accept that key ({code}).", provider.label)));
                    // "n" goes on with this key: a check is a help, never a lock.
                    if ask_yes("Type it again? [Y/n] › ", color) {
                        ask_key = true;
                        continue;
                    }
                    println!("{}", paint(color, Style::Dim, "Going on with it as it is."));
                }
                Check::Unknown(why) => {
                    println!("{}", paint(color, Style::Dim, &format!("Couldn't check the key ({why}); going on.")))
                }
            }
        }
        let model: Box<dyn rak::provider::Model> = if provider.anthropic {
            Box::new(rak::anthropic::Anthropic::new(&provider.base_url, api_key.unwrap_or_default(), ""))
        } else {
            Box::new(rak::provider::OpenAICompatible::new(&provider.base_url, api_key, ""))
        };
        println!("Asking {} for its models…", provider.label);
        let listed = match model.list_models() {
            Ok(models) if !models.is_empty() => Some(models),
            Ok(_) => {
                println!("{}", paint(color, Style::Yellow, &format!("{} lists no models.", provider.label)));
                None
            }
            Err(why) => {
                println!("{}", paint(color, Style::Yellow, &listing_failed(&provider.label, &why, provider.key.is_some())));
                None
            }
        };
        let picked = match listed {
            // Esc on the model list: stop.
            Some(models) => Some(super::chat::pick_from(models, None, color)?),
            None => None,
        };
        let Some(model_name) = picked else {
            if is_local(provider) {
                println!("Start {} and run `rak setup` again, or pick another provider then.", provider.label);
                return None;
            }
            if !ask_yes("Try another key? [Y/n] › ", color) {
                return None;
            }
            ask_key = true;
            continue;
        };
        let mut values: Vec<(&str, &str)> = vec![("RAK_PROVIDER", &provider.name), ("RAK_MODEL", &model_name)];
        if let (Some(name), Some(typed)) = (&provider.key, &typed_key) {
            values.push((name.as_str(), typed.as_str()));
        }
        let old = std::fs::read_to_string(&file).ok();
        let text = set_lines(old.as_deref().unwrap_or(TEMPLATE), &values);
        if let Err(e) = write_private(&file, &text) {
            eprintln!("{}", paint(color, Style::Yellow, &format!("rak: cannot save {}: {e}", file.display())));
            return None;
        }
        remember_model(&provider.name, &model_name);
        let what = if typed_key.is_some() { "provider, key and model" } else { "provider and model" };
        println!("{}", paint(color, Style::Green, &format!("✓ Saved the {what} in {shown}. `rak setup` changes them any time.")));
        return Some(());
    }
}

/// What a key check found.
#[derive(Debug, PartialEq)]
pub enum Check {
    Accepted,
    /// The provider said no: 401, 403 (or 400, Gemini's answer to a bad key).
    Refused(u16),
    /// Couldn't tell (no network, a server error): setup goes on.
    Unknown(String),
}

/// Pure — unit-tested. A check's HTTP status as an answer about the key.
pub fn classify(status: u16) -> Check {
    match status {
        200..=299 => Check::Accepted,
        400 | 401 | 403 => Check::Refused(status),
        other => Check::Unknown(format!("it answered {other}")),
    }
}

/// GET `url` with the key as a bearer token. Costs nothing: it reads the
/// account, it runs no model.
fn check_key(url: &str, key: &str) -> Check {
    let agent = rak::http::builder().timeout(Duration::from_secs(10)).build();
    match agent.get(url).set("Authorization", &format!("Bearer {key}")).call() {
        Ok(response) => classify(response.status()),
        Err(ureq::Error::Status(code, _)) => classify(code),
        Err(ureq::Error::Transport(t)) => Check::Unknown(t.kind().to_string()),
    }
}

/// Pure — unit-tested. What to say when the model list could not be had:
/// a refused key said as such, anything else as it came.
pub fn listing_failed(label: &str, why: &str, keyed: bool) -> String {
    let refused = [" 400", " 401", " 403"].iter().find_map(|code| why.find(code).map(|at| &why[at + 1..at + 4]));
    if let (true, Some(code)) = (keyed, refused) {
        format!("{label} did not accept that key ({code}).")
    } else {
        format!("Couldn't get {label}'s models: {why}")
    }
}

/// `rak setup`.
pub fn command(args: &[String]) -> ! {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        std::process::exit(0);
    }
    if !args.is_empty() {
        eprintln!("rak: rak setup takes no arguments\n\n{USAGE}");
        std::process::exit(2);
    }
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() {
        eprintln!("rak: rak setup asks questions, so it needs a terminal. Without one, set RAK_PROVIDER, RAK_MODEL and the key in ~/.config/rak/.env (`rak keys` creates it).");
        std::process::exit(2);
    }
    let color = std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    match run(color) {
        Some(_) => {
            println!("Start a chat with: rak");
            std::process::exit(0)
        }
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
    fn setup_runs_only_in_a_terminal_with_nothing_chosen_and_no_key_file() {
        assert!(needed(true, false, false));
        assert!(!needed(false, false, false), "a script");
        assert!(!needed(true, true, false), "already set up");
        assert!(!needed(true, false, true), "-m, --provider or RAK_* given");
    }

    #[test]
    fn addresses_give_host_and_port() {
        assert_eq!(host_port("http://localhost:11434/v1"), Some(("localhost".into(), 11434)));
        assert_eq!(host_port("https://api.openai.com/v1"), Some(("api.openai.com".into(), 443)));
        assert_eq!(host_port("http://x"), Some(("x".into(), 80)));
        assert_eq!(host_port("nonsense"), None);
    }

    #[test]
    fn running_local_servers_come_first_and_stopped_ones_last() {
        let providers = rak::providers::all();
        let up = |p: &Provider| p.name == "ollama";
        let key_at = |name: &str| (name == "ANTHROPIC_API_KEY").then(|| "your environment".to_string());
        let rows = rows(providers, &up, &key_at);
        assert_eq!(rows.len(), providers.len());
        assert_eq!(providers[rows[0].index].name, "ollama");
        assert!(rows[0].note.contains("no key needed") && !rows[0].dim);
        let last = &rows[rows.len() - 1];
        assert!(last.dim && providers[last.index].key.is_none(), "a stopped local server is last, dim");
        let anthropic = rows.iter().find(|r| providers[r.index].name == "anthropic").unwrap();
        assert!(anthropic.note.contains("ANTHROPIC_API_KEY set in your environment"));
        let openai = rows.iter().find(|r| providers[r.index].name == "openai").unwrap();
        assert!(openai.note.contains("needs OPENAI_API_KEY"));
    }

    #[test]
    fn values_go_on_their_lines_and_the_rest_of_the_file_is_kept() {
        let text = set_lines(TEMPLATE, &[("RAK_PROVIDER", "openrouter"), ("RAK_MODEL", "x/y z"), ("OPENROUTER_API_KEY", "sk-or-1")]);
        let read = parse_env(&text);
        assert_eq!(read["RAK_PROVIDER"], "openrouter");
        assert_eq!(read["RAK_MODEL"], "x/y z");
        assert_eq!(read["OPENROUTER_API_KEY"], "sk-or-1");
        assert!(text.contains("OPENROUTER_API_KEY=sk-or-1  # openrouter"), "the line keeps its comment");
        assert_eq!(text.lines().count(), TEMPLATE.lines().count(), "nothing added: the template has every line");

        // A file of the person's own: other values kept, a missing name added at the end.
        let own = "# mine\nTAVILY_API_KEY=tv\nRAK_PROVIDER=ollama\nRAK_PROVIDER=groq\n";
        let text = set_lines(own, &[("RAK_PROVIDER", "anthropic"), ("RAK_MODEL", "claude-haiku-5-5")]);
        assert_eq!(text, "# mine\nTAVILY_API_KEY=tv\nRAK_PROVIDER=anthropic\n\nRAK_MODEL=claude-haiku-5-5\n");
    }

    #[test]
    fn a_refused_key_is_said_plainly() {
        assert_eq!(
            listing_failed("OpenAI", "https://api.openai.com/v1/models answered 401", true),
            "OpenAI did not accept that key (401)."
        );
        assert!(listing_failed("Ollama", "connection refused", false).starts_with("Couldn't get Ollama's models"));
    }

    #[test]
    fn a_key_check_s_answer_is_read_as_accepted_refused_or_unknown() {
        assert_eq!(classify(200), Check::Accepted);
        assert_eq!(classify(401), Check::Refused(401));
        assert_eq!(classify(400), Check::Refused(400), "Gemini answers a bad key with 400");
        assert!(matches!(classify(503), Check::Unknown(_)), "a server error is no verdict on the key");
        assert_eq!(
            listing_failed("Gemini", "https://x/models answered 400: Please pass a valid API key", true),
            "Gemini did not accept that key (400)."
        );
    }

    #[test]
    fn a_pasted_key_loses_the_terminal_s_paste_markers() {
        assert_eq!(clean_key(b"\x1b[200~sk-ant-abc\x1b[201~"), "sk-ant-abc");
        assert_eq!(clean_key(b"  sk 1\t"), "sk1");
    }
}
