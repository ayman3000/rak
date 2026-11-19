//! `rak mcp add` and `/mcp add` from what a person has in hand: the address
//! of a server's page, the JSON that page shows, or nothing (a few
//! questions). Whatever the start, the server is shown and asked about
//! before it is saved: it is a program that will run on this machine.
//!
//! `choose` is the whole conversation and saves nothing; its questions,
//! answers and page fetches go through `Hands`, so tests drive it.

use rak::mcp::find::{self, Blank};
use rak::mcp::{ServerConfig, Transport};
use std::collections::BTreeMap;

/// How `choose` talks and reads: a terminal in rak, a script in tests.
pub struct Hands<'a> {
    /// Shows the question and returns the line typed; None when there is no more input.
    pub ask: &'a mut dyn FnMut(&str) -> Option<String>,
    /// The same for a value that must not show on screen.
    pub secret: &'a mut dyn FnMut(&str) -> Option<String>,
    /// The text at a web address.
    pub fetch: &'a dyn Fn(&str) -> Result<String, String>,
    pub say: &'a mut dyn FnMut(&str),
}

pub struct Request {
    /// A page's address, JSON, or empty: ask.
    pub source: String,
    /// --project was given: this project's file, and no question about where.
    pub project: bool,
    /// -y: nothing is asked. Needs one server with nothing left to fill in.
    pub yes: bool,
    /// Someone is there to answer.
    pub can_ask: bool,
    pub windows: bool,
    /// Names already in the files, to say when one would be replaced.
    pub have: Vec<String>,
}

/// A server the person said yes to, and where it goes.
#[derive(Debug, PartialEq)]
pub struct Chosen {
    pub server: ServerConfig,
    pub project: bool,
}

const FIRST: &str = "Paste the JSON from the server's page, or the page's address, or press Enter to answer questions › ";

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// How many braces are still open: a paste is read line by line until none are.
fn open_braces(text: &str) -> i64 {
    let (mut depth, mut in_string, mut escaped) = (0i64, false, false);
    for c in text.chars() {
        match c {
            _ if escaped => escaped = false,
            '\\' if in_string => escaped = true,
            '"' => in_string = !in_string,
            '{' if !in_string => depth += 1,
            '}' if !in_string => depth -= 1,
            _ => {}
        }
    }
    depth
}

/// What the person typed or pasted at a question, a paste of several lines
/// included: lines are read until its braces close.
fn read_source(h: &mut Hands, question: &str) -> Option<String> {
    let mut text = (h.ask)(question)?;
    let mut lines = 0;
    while text.contains('{') && open_braces(&text) > 0 && lines < 500 {
        let Some(more) = (h.ask)("") else { break };
        text.push('\n');
        text.push_str(&more);
        lines += 1;
    }
    Some(text.trim().to_string())
}

/// The servers at `source`, with the name to give one that has none.
fn candidates(source: &str, windows: bool, h: &mut Hands) -> Result<(Vec<ServerConfig>, String), String> {
    if source.starts_with("http://") || source.starts_with("https://") {
        let mut last = String::new();
        for place in find::places_to_read(source) {
            match (h.fetch)(&place) {
                Ok(text) => return Ok((find::servers_in(&text, windows), find::tidy_name(source))),
                Err(why) => last = why,
            }
        }
        return Err(last);
    }
    if source.contains('{') {
        return Ok((find::servers_in(source, windows), String::new()));
    }
    Err("that is neither JSON nor a web address (http or https)".to_string())
}

/// A server from a few questions. None: the person stopped.
fn guided(h: &mut Hands) -> Option<ServerConfig> {
    let name = loop {
        let typed = (h.ask)("A short name for it (letters, digits, - and _) › ")?;
        match typed.trim() {
            "" => return None,
            name if valid_name(name) => break name.to_string(),
            _ => (h.say)("A name is letters, digits, - and _ only."),
        }
    };
    let start = (h.ask)("What starts it? The command (npx -y some-mcp) or its web address (https://…) › ")?;
    let start = start.trim();
    if start.is_empty() {
        return None;
    }
    let web = start.starts_with("http://") || start.starts_with("https://");
    let mut values = BTreeMap::new();
    let what = if web {
        "A header it needs, by name (Enter when there are none left) › "
    } else {
        "A variable it needs, by name (Enter when there are none left) › "
    };
    loop {
        let key = (h.ask)(what)?;
        let key = key.trim().trim_end_matches([':', '=']).to_string();
        if key.is_empty() {
            break;
        }
        let value = (h.secret)(&format!("Value for {key} (it is not shown) › "))?;
        values.insert(key, value);
    }
    let transport = if web {
        Transport::Http { url: start.to_string(), headers: values }
    } else {
        let mut words = find::words(start);
        let command = words.remove(0);
        Transport::Stdio { command, args: words, env: values }
    };
    Some(ServerConfig { name, transport, timeout: None })
}

/// Shows one server the way the person should read it before saying yes.
fn show(server: &ServerConfig, h: &mut Hands) {
    (h.say)(&format!("  {}", if server.name.is_empty() { "(no name yet)" } else { &server.name }));
    for line in find::describe(server) {
        (h.say)(&format!("    {line}"));
    }
}

/// The conversation: finds or asks for the server, has its blanks filled,
/// shows it and asks. What comes back is what to save; empty when the
/// answer was no. An error is something the person must fix first.
pub fn choose(req: &Request, h: &mut Hands) -> Result<Vec<Chosen>, String> {
    let needs_someone = "rak mcp add asks before it saves: run it in a terminal, or pass -y with a source that names one server";
    let mut source = req.source.trim().to_string();
    let (mut found, mut spare_name, mut from) = (Vec::new(), String::new(), String::new());
    for _ in 0..3 {
        if source.is_empty() {
            if !req.can_ask {
                return Err(needs_someone.to_string());
            }
            source = match read_source(h, FIRST) {
                Some(typed) => typed,
                None => return Ok(Vec::new()),
            };
            if source.is_empty() {
                // Enter: the questions.
                return Ok(match guided(h) {
                    Some(server) => confirm(vec![server], "your answers", req, h)?,
                    None => Vec::new(),
                });
            }
        }
        let (servers, name) = candidates(&source, req.windows, h)?;
        from = if source.contains('{') { "what you pasted".to_string() } else { source.clone() };
        if !servers.is_empty() {
            (found, spare_name) = (servers, name);
            break;
        }
        let nothing = format!(
            "No MCP config found in {from}. Look on the server's page for the JSON it says to add to your MCP client (it has \"mcpServers\" in it)."
        );
        if !req.can_ask {
            return Err(nothing);
        }
        (h.say)(&nothing);
        source.clear();
    }
    if found.is_empty() {
        return Ok(Vec::new());
    }
    if found.len() > 1 {
        (h.say)(&format!("Found {} in {from}:", found.len()));
        for (i, server) in found.iter().enumerate() {
            (h.say)(&format!("{}.", i + 1));
            show(server, h);
        }
        if req.yes || !req.can_ask {
            return Err(format!("{from} describes {} servers; run it without -y to pick one", found.len()));
        }
        let typed = (h.ask)("Which one? A number, a for all, or Enter for none › ").unwrap_or_default();
        found = match typed.trim() {
            "a" | "A" | "all" => found,
            number => match number.parse::<usize>().ok().filter(|n| (1..=found.len()).contains(n)) {
                Some(n) => vec![found.remove(n - 1)],
                None => return Ok(Vec::new()),
            },
        };
    }
    // A name for one that has none, and values for what the page left blank.
    let mut ready = Vec::new();
    for mut server in found {
        if !valid_name(&server.name) {
            let tidied = find::tidy_name(&server.name);
            let offer = if valid_name(&tidied) { tidied } else { spare_name.clone() };
            if req.yes || !req.can_ask {
                if !valid_name(&offer) {
                    return Err("the JSON gives the server no name; run it without -y to name it".to_string());
                }
                server.name = offer;
            } else {
                let question = if valid_name(&offer) {
                    format!("A short name for it (Enter for {offer}) › ")
                } else {
                    "A short name for it › ".to_string()
                };
                let typed = (h.ask)(&question).unwrap_or_default();
                let typed = if typed.trim().is_empty() { offer } else { typed.trim().to_string() };
                if !valid_name(&typed) {
                    (h.say)("A name is letters, digits, - and _ only. Not added.");
                    continue;
                }
                server.name = typed;
            }
        }
        let blanks = find::blanks(&server);
        if !blanks.is_empty() && (req.yes || !req.can_ask) {
            return Err(format!("{} has values to fill in ({} of them); run it without -y", server.name, blanks.len()));
        }
        let mut filled = true;
        for blank in blanks {
            let value = match &blank {
                Blank::Env(key) | Blank::Header(key) => (h.secret)(&format!("{} needs {key}. Its value (it is not shown) › ", server.name)),
                Blank::Arg(i) => {
                    let shown = match &server.transport {
                        Transport::Stdio { args, .. } => args.get(*i).cloned().unwrap_or_default(),
                        Transport::Http { .. } => String::new(),
                    };
                    (h.ask)(&format!("{} has a stand-in, `{shown}`. What should it be? › ", server.name))
                }
            };
            match value.map(|v| v.trim().to_string()).filter(|v| !v.is_empty()) {
                Some(value) => find::fill(&mut server, &blank, &value),
                None => {
                    (h.say)(&format!("{} was left as it is: not added.", server.name));
                    filled = false;
                    break;
                }
            }
        }
        if filled {
            ready.push(server);
        }
    }
    confirm(ready, &from, req, h)
}

/// Shows each server and asks whether, and where, to add it.
fn confirm(servers: Vec<ServerConfig>, from: &str, req: &Request, h: &mut Hands) -> Result<Vec<Chosen>, String> {
    let mut chosen = Vec::new();
    for server in servers {
        (h.say)(&format!("From {from}:"));
        show(&server, h);
        (h.say)(&find::caution(&server));
        if req.have.contains(&server.name) {
            (h.say)(&format!("You already have a server named {}: this replaces it.", server.name));
        }
        if req.yes {
            chosen.push(Chosen { server, project: req.project });
            continue;
        }
        if !req.can_ask {
            return Err("rak mcp add asks before it saves: run it in a terminal, or pass -y".to_string());
        }
        let question = if req.project {
            "Add it to this project? [y/N] › "
        } else {
            "Add it? y: for all projects · p: this project only · n: no › "
        };
        let typed = (h.ask)(question).unwrap_or_default();
        match typed.trim().to_lowercase().as_str() {
            "y" | "yes" => chosen.push(Chosen { server, project: req.project }),
            "p" if !req.project => chosen.push(Chosen { server, project: true }),
            _ => (h.say)("Not added."),
        }
    }
    Ok(chosen)
}

/// The same conversation at a terminal: questions on stderr, answers from
/// stdin, pages from the web.
pub fn choose_at_terminal(req: &Request, color: bool) -> Result<Vec<Chosen>, String> {
    use super::style::{paint, Style};
    use std::io::Write;
    let mut ask = |question: &str| -> Option<String> {
        if !question.is_empty() {
            eprint!("{}", paint(color, Style::Bold, question));
            let _ = std::io::stderr().flush();
        }
        let mut line = String::new();
        match std::io::stdin().read_line(&mut line) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(line.trim_end_matches(['\n', '\r']).to_string()),
        }
    };
    let mut secret = |question: &str| -> Option<String> { super::setup::read_hidden(question, color) };
    let fetch = |address: &str| -> Result<String, String> {
        let bytes = rak::skills::fetch::fetch_bytes(address, 1024 * 1024).map_err(|why| why.replace(", too big for a skill", ""))?;
        String::from_utf8(bytes).map_err(|_| format!("{address} is not text"))
    };
    let mut say = |line: &str| println!("{line}");
    choose(req, &mut Hands { ask: &mut ask, secret: &mut secret, fetch: &fetch, say: &mut say })
}

#[cfg(test)]
mod tests {
    use super::*;

    const JSON: &str = r#"{ "mcpServers": { "chrome-devtools": { "command": "npx", "args": ["-y", "chrome-devtools-mcp@latest"] } } }"#;

    /// Runs `choose` with these answers in order; returns its result and
    /// everything that was said or asked.
    fn run(req: Request, answers: &[&str], page: Result<&str, &str>) -> (Result<Vec<Chosen>, String>, String) {
        let mut left: Vec<String> = answers.iter().rev().map(|a| a.to_string()).collect();
        let heard = std::cell::RefCell::new(String::new());
        let left = std::cell::RefCell::new(&mut left);
        let mut ask = |q: &str| {
            heard.borrow_mut().push_str(&format!("ASK {q}\n"));
            left.borrow_mut().pop()
        };
        let mut secret = |q: &str| {
            heard.borrow_mut().push_str(&format!("SECRET {q}\n"));
            left.borrow_mut().pop()
        };
        let fetch = |address: &str| {
            heard.borrow_mut().push_str(&format!("FETCH {address}\n"));
            page.map(str::to_string).map_err(str::to_string)
        };
        let mut say = |line: &str| heard.borrow_mut().push_str(&format!("{line}\n"));
        let result = choose(&req, &mut Hands { ask: &mut ask, secret: &mut secret, fetch: &fetch, say: &mut say });
        (result, heard.into_inner())
    }

    fn request(source: &str) -> Request {
        Request { source: source.into(), project: false, yes: false, can_ask: true, windows: false, have: Vec::new() }
    }

    #[test]
    fn a_page_address_is_read_shown_and_asked_about() {
        let readme = format!("# Server\n\n```json\n{JSON}\n```\n");
        let (result, heard) = run(request("https://github.com/ChromeDevTools/chrome-devtools-mcp"), &["y"], Ok(&readme));
        let chosen = result.unwrap();
        assert_eq!(chosen.len(), 1);
        assert_eq!((chosen[0].server.name.as_str(), chosen[0].project), ("chrome-devtools", false));
        assert!(heard.contains("FETCH https://raw.githubusercontent.com/ChromeDevTools/chrome-devtools-mcp/HEAD/README.md"), "{heard}");
        assert!(heard.contains("runs:  npx -y chrome-devtools-mcp@latest") && heard.contains("downloaded when it starts"), "{heard}");
        // "p" puts it in this project; anything else is a no.
        let (result, _) = run(request(JSON), &["p"], Err("unused"));
        assert!(result.unwrap()[0].project);
        let (result, heard) = run(request(JSON), &[""], Err("unused"));
        assert!(result.unwrap().is_empty() && heard.contains("Not added."));
    }

    #[test]
    fn pasted_json_is_read_over_several_lines_and_a_replacement_is_said() {
        let mut req = request("");
        req.have = vec!["chrome-devtools".into()];
        let lines =
            ["{", "  \"mcpServers\": {", "    \"chrome-devtools\": { \"command\": \"npx\", \"args\": [\"-y\", \"x\"] }", "  }", "}", "y"];
        let (result, heard) = run(req, &lines, Err("unused"));
        assert_eq!(result.unwrap()[0].server.name, "chrome-devtools");
        assert!(
            heard.contains("Paste the JSON") && heard.contains("this replaces it") && heard.contains("From what you pasted:"),
            "{heard}"
        );
    }

    #[test]
    fn nothing_found_asks_again_and_enter_leads_to_the_questions() {
        // The page has no config: said, then the person presses Enter and answers.
        let answers = ["", "docs", "https://example.com/mcp", "Authorization", "Bearer t", "", "y"];
        let (result, heard) = run(request("https://example.com/readme"), &answers, Ok("nothing here"));
        let chosen = result.unwrap();
        assert!(heard.contains("No MCP config found in https://example.com/readme"), "{heard}");
        assert!(
            matches!(&chosen[0].server.transport, Transport::Http { url, headers } if url == "https://example.com/mcp" && headers["Authorization"] == "Bearer t")
        );
        assert!(heard.contains("sends:  Authorization") && !heard.contains("Bearer t\n"), "the value is never shown: {heard}");
        // A command, with a quoted argument and a variable.
        let answers = ["", "files", "npx -y fs-mcp 'my folder'", "TOKEN", "t-1", "", "y"];
        let (result, _) = run(request(""), &answers, Err("unused"));
        let server = &result.unwrap()[0].server;
        assert!(
            matches!(&server.transport, Transport::Stdio { command, args, env } if command == "npx" && args[2] == "my folder" && env["TOKEN"] == "t-1")
        );
        // A page that can't be fetched is an error, with the reason.
        let (result, _) = run(request("https://example.com/x"), &[], Err("https://example.com/x answered HTTP 404"));
        assert!(result.unwrap_err().contains("404"));
    }

    #[test]
    fn several_servers_are_listed_to_pick_from_and_blanks_are_asked_for() {
        let two = r#"{ "mcpServers": {
            "a": { "command": "a-server" },
            "b": { "command": "npx", "args": ["-y", "b-mcp", "/path/to/repo"], "env": { "KEY": "<YOUR_KEY>" } } } }"#;
        let (result, heard) = run(request(two), &["2", "/code", "k-1", "y"], Err("unused"));
        let chosen = result.unwrap();
        assert_eq!(chosen.len(), 1);
        assert!(heard.contains("Found 2 in what you pasted:") && heard.contains("Which one?"), "{heard}");
        assert!(heard.contains("SECRET b needs KEY") && heard.contains("stand-in, `/path/to/repo`"), "{heard}");
        assert!(matches!(&chosen[0].server.transport, Transport::Stdio { args, env, .. } if args[2] == "/code" && env["KEY"] == "k-1"));
        // "a" takes both; a blank left empty drops that server only.
        let (result, heard) = run(request(two), &["a", "y", "", "y"], Err("unused"));
        let names: Vec<String> = result.unwrap().into_iter().map(|c| c.server.name).collect();
        assert_eq!(names, ["a"]);
        assert!(heard.contains("b was left as it is: not added."), "{heard}");
        // Enter at the pick is none.
        assert!(run(request(two), &[""], Err("unused")).0.unwrap().is_empty());
    }

    #[test]
    fn a_bare_entry_is_named_and_y_asks_nothing() {
        let bare = r#"{ "command": "npx", "args": ["-y", "some-mcp"] }"#;
        let (result, heard) = run(request(bare), &["some", "y"], Err("unused"));
        assert_eq!(result.unwrap()[0].server.name, "some");
        assert!(heard.contains("A short name for it"), "{heard}");
        // From a repository's page the repository's name is offered.
        let (result, heard) = run(request("https://github.com/o/some-mcp"), &["", "y"], Ok(bare));
        assert_eq!(result.unwrap()[0].server.name, "some-mcp");
        assert!(heard.contains("Enter for some-mcp"), "{heard}");

        let mut yes = request(JSON);
        (yes.yes, yes.can_ask, yes.project) = (true, false, true);
        let (result, heard) = run(yes, &[], Err("unused"));
        assert_eq!(result.unwrap(), [Chosen { server: find::servers_in(JSON, false).remove(0), project: true }]);
        assert!(!heard.contains("ASK"), "{heard}");
        // -y with something to fill in, or several servers, or nobody to ask: said, nothing saved.
        let mut blank = request(r#"{ "mcpServers": { "b": { "command": "x", "env": { "K": "<KEY>" } } } }"#);
        blank.yes = true;
        assert!(run(blank, &[], Err("unused")).0.unwrap_err().contains("values to fill in"));
        let mut nobody = request(JSON);
        nobody.can_ask = false;
        assert!(run(nobody, &[], Err("unused")).0.unwrap_err().contains("asks before it saves"));
        let mut empty = request("");
        empty.can_ask = false;
        assert!(run(empty, &[], Err("unused")).0.is_err());
        assert!(run(request("just some words"), &[], Err("unused")).0.unwrap_err().contains("neither JSON nor a web address"));
    }
}
