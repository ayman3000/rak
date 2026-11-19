//! `rak mcp add|list|remove`: edits the list of MCP servers from the shell.

use super::args::usage_error;
use super::settings::config_dir;
use super::style::cut;
use rak::interrupt::Stop;
use rak::mcp;
use std::path::PathBuf;

pub const MCP_HELP: &str = "\
rak mcp — tools from MCP servers

  rak mcp add                                     paste the JSON from a server's page,
                                                  or its address, or answer questions
  rak mcp add ADDRESS                             read the JSON from a page (a GitHub
                                                  repository, a README, a docs page)
  rak mcp add 'JSON'                              the JSON itself (- reads it from a pipe)
  rak mcp add NAME [options] -- COMMAND [ARG…]    a program rak starts
  rak mcp add NAME [options] --url URL            a server at a web address
  rak mcp list                                    the servers, and whether each connects
  rak mcp remove NAME [--project]

rak shows what it found and asks before it saves: a server is a program that
runs on your machine. In a chat, /mcp add does the same and the server's tools
are usable at once.

OPTIONS FOR add
  -e, --env KEY=VALUE     an environment variable for the program (repeatable)
  -H, --header 'N: V'     a header sent to the web address (repeatable)
      --timeout SECONDS   how long one tool call may take (default 120)
      --project           write ./.rak/mcp.json (this project only) instead of
                          the file for all projects
  -y, --yes               with an address or JSON: don't ask (one server, nothing
                          left to fill in)

EXAMPLES
  rak mcp add https://github.com/ChromeDevTools/chrome-devtools-mcp
  rak mcp add files -- npx -y @modelcontextprotocol/server-filesystem .
  rak mcp add docs --url https://example.com/mcp -H 'Authorization: Bearer KEY'

You can also edit the files yourself: ~/.config/rak/mcp.json for all projects
and ./.rak/mcp.json for one, in the shape Claude Code and Cursor use, so an
entry can be pasted across. A project's file wins over the one for all projects.
";

/// Pure — unit-tested. Whether `rak mcp add …` is the form that takes what
/// the person has in hand (an address, JSON, a pipe, or nothing) and not the
/// older `NAME -- COMMAND` / `NAME --url URL`. Returns that source.
pub fn in_hand(args: &[String]) -> Option<String> {
    let words: Vec<&str> = args.iter().map(String::as_str).filter(|a| !matches!(*a, "--project" | "-y" | "--yes")).collect();
    match words.as_slice() {
        [] => Some(String::new()),
        [one] if *one == "-" || one.starts_with("http://") || one.starts_with("https://") || one.trim_start().starts_with('{') => {
            Some(one.to_string())
        }
        _ => None,
    }
}

/// Saves each chosen server in its file. Returns what was saved and where.
pub fn save(
    chosen: Vec<super::mcp_add::Chosen>,
    project_file: &std::path::Path,
    user_file: &std::path::Path,
    cwd: &std::path::Path,
) -> Result<Vec<mcp::ServerConfig>, String> {
    let mut saved = Vec::new();
    for pick in chosen {
        let file = if pick.project { project_file } else { user_file };
        if pick.project {
            // The person put it there: this folder is theirs to trust.
            super::trust::remember(cwd);
        }
        let replaced = mcp::add_to_file(file, &pick.server)?;
        println!("{} {} in {}", if replaced { "Replaced" } else { "Added" }, pick.server.name, file.display());
        saved.push(pick.server);
    }
    Ok(saved)
}

/// The names in these files, for "this replaces it".
pub fn names_in(files: &[PathBuf]) -> Vec<String> {
    mcp::load(files).0.into_iter().map(|(server, _)| server.name).collect()
}

/// `rak mcp add|list|remove`: edits the list of MCP servers. Never returns.
pub fn run(args: &[String]) -> ! {
    // Ctrl-C while servers are being tried quits, stopping them first.
    super::signals::install(Stop::new());
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let files = mcp::config_files(&cwd, config_dir().as_deref());
    let project_file = files[0].clone();
    let Some(user_file) = files.get(1).cloned() else { usage_error("no home folder to keep mcp.json in") };
    let fail = |why: String| -> ! {
        eprintln!("rak: {why}");
        std::process::exit(1);
    };
    let plural = |n: usize| if n == 1 { "" } else { "s" };

    match args.first().map(String::as_str) {
        Some("add") if in_hand(&args[1..]).is_some() => {
            use std::io::{IsTerminal, Read};
            let mut source = in_hand(&args[1..]).unwrap_or_default();
            let piped = !std::io::stdin().is_terminal();
            if source == "-" || (source.is_empty() && piped) {
                source.clear();
                let _ = std::io::stdin().read_to_string(&mut source);
                if source.trim().is_empty() {
                    usage_error("rak mcp add: nothing came in on the pipe");
                }
            }
            let request = super::mcp_add::Request {
                source,
                project: args[1..].iter().any(|a| a == "--project"),
                yes: args[1..].iter().any(|a| a == "-y" || a == "--yes"),
                can_ask: !piped && std::io::stderr().is_terminal(),
                windows: cfg!(windows),
                have: names_in(&files),
            };
            let color = std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
            let chosen = super::mcp_add::choose_at_terminal(&request, color).unwrap_or_else(|why| fail(why));
            if chosen.is_empty() {
                std::process::exit(0);
            }
            let saved = save(chosen, &project_file, &user_file, &cwd).unwrap_or_else(|why| fail(why));
            // Try them now, so a mistake shows here and not in the next chat.
            let connected = mcp::connect_all(&saved, &cwd, false, &Stop::new());
            for (name, count) in &connected.servers {
                println!("{name} connected: {count} tool{}", plural(*count));
            }
            for problem in &connected.problems {
                println!("Saved, but it did not connect: {problem}");
            }
            println!("To change it by hand, the files are {} and {}.", user_file.display(), project_file.display());
            let code = if connected.problems.is_empty() { 0 } else { 1 };
            // exit skips destructors: stop the servers' programs first.
            drop(connected);
            std::process::exit(code);
        }
        Some("add") => {
            let (mut name, mut command, mut url) = (None::<String>, Vec::<String>::new(), None::<String>);
            let (mut env, mut headers) = (std::collections::BTreeMap::new(), std::collections::BTreeMap::new());
            let (mut project, mut timeout) = (false, None);
            let mut rest = args[1..].iter().cloned();
            while let Some(arg) = rest.next() {
                let mut value = |flag: &str| rest.next().unwrap_or_else(|| usage_error(&format!("{flag} needs a value")));
                match arg.as_str() {
                    "--project" => project = true,
                    "--url" => url = Some(value("--url")),
                    "--timeout" => {
                        timeout =
                            Some(value("--timeout").parse::<u64>().unwrap_or_else(|_| usage_error("--timeout needs a number of seconds")))
                    }
                    "-e" | "--env" => match value("--env").split_once('=') {
                        Some((key, v)) if !key.is_empty() => {
                            env.insert(key.to_string(), v.to_string());
                        }
                        _ => usage_error("--env needs KEY=VALUE"),
                    },
                    "-H" | "--header" => match value("--header").split_once(':') {
                        Some((key, v)) if !key.trim().is_empty() => {
                            headers.insert(key.trim().to_string(), v.trim().to_string());
                        }
                        _ => usage_error("--header needs 'Name: value'"),
                    },
                    // Everything after -- is the program and its own options.
                    "--" => command.extend(rest.by_ref()),
                    flag if flag.starts_with('-') && name.is_none() => usage_error(&format!("unknown option {flag} (see rak mcp --help)")),
                    _ if name.is_none() => name = Some(arg),
                    // No --: the program starts at the first word after the name.
                    _ => {
                        command.push(arg);
                        command.extend(rest.by_ref());
                    }
                }
            }
            let Some(name) = name else { usage_error("rak mcp add needs a name (see rak mcp --help)") };
            if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                usage_error("a server's name is letters, digits, - and _ only");
            }
            let transport = match (command.split_first(), url) {
                (Some(_), Some(_)) => usage_error("give a program or --url, not both"),
                (Some((program, arguments)), None) => mcp::Transport::Stdio { command: program.clone(), args: arguments.to_vec(), env },
                (None, Some(url)) => mcp::Transport::Http { url, headers },
                (None, None) => usage_error("rak mcp add needs a program to run or --url (see rak mcp --help)"),
            };
            let server = mcp::ServerConfig { name: name.clone(), transport, timeout };
            let file = if project { project_file } else { user_file };
            if project {
                // The person put it there: this folder is theirs to trust.
                super::trust::remember(&cwd);
            }
            let replaced = mcp::add_to_file(&file, &server).unwrap_or_else(|why| fail(why));
            println!("{} {name} in {}", if replaced { "Replaced" } else { "Added" }, file.display());

            // Try it now, so a typo shows here and not in the next chat.
            let connected = mcp::connect_all(&[server], &cwd, false, &Stop::new());
            let code = match connected.problems.first() {
                Some(problem) => {
                    println!("It is saved, but it did not connect: {problem}");
                    1
                }
                None => {
                    let names: Vec<&str> = connected.tools.iter().map(|t| t.name()).collect();
                    println!("Connected: {} tool{} — {}", names.len(), plural(names.len()), cut(&names.join(", "), 160));
                    0
                }
            };
            // exit skips destructors: stop the server's program first.
            drop(connected);
            std::process::exit(code);
        }
        Some("list" | "ls") => {
            let (servers, problems) = mcp::load(&files);
            for problem in &problems {
                println!("{problem}");
            }
            if servers.is_empty() {
                println!("No MCP servers yet. Add one with:\n  rak mcp add            (paste the JSON from the server's page, or its address)\n  rak mcp add NAME -- COMMAND [ARG…]\n  rak mcp add NAME --url URL");
                std::process::exit(0);
            }
            // A project's servers are programs: started only in a trusted folder.
            let trusted = super::trust::is_trusted(&cwd);
            let configs: Vec<mcp::ServerConfig> =
                servers.iter().filter(|(_, file)| trusted || *file != project_file).map(|(server, _)| server.clone()).collect();
            let connected = mcp::connect_all(&configs, &cwd, false, &Stop::new());
            for (server, file) in &servers {
                // Keys stay out of the listing: names of variables and headers only.
                let what = match &server.transport {
                    mcp::Transport::Stdio { command, args, env } => {
                        let keys: Vec<&str> = env.keys().map(String::as_str).collect();
                        format!(
                            "{command} {}{}",
                            args.join(" "),
                            if keys.is_empty() { String::new() } else { format!("  (env: {})", keys.join(", ")) }
                        )
                    }
                    mcp::Transport::Http { url, headers } => {
                        let keys: Vec<&str> = headers.keys().map(String::as_str).collect();
                        format!("{url}{}", if keys.is_empty() { String::new() } else { format!("  (headers: {})", keys.join(", ")) })
                    }
                };
                let status = match connected.servers.iter().find(|(name, _)| name == &server.name) {
                    _ if !trusted && *file == project_file => {
                        "not started: this folder is not trusted (start rak here to be asked)".to_string()
                    }
                    Some((_, count)) => format!("✓ {count} tool{}", plural(*count)),
                    None => {
                        let prefix = format!("{}: ", server.name);
                        let why = connected.problems.iter().find_map(|p| p.strip_prefix(&prefix)).unwrap_or("couldn't connect");
                        format!("✗ {why}")
                    }
                };
                let scope = if *file == project_file { "this project" } else { "all projects" };
                println!("{}  {status}\n    {}\n    {scope}: {}", server.name, what.trim(), file.display());
            }
            // exit skips destructors: stop the servers' programs first.
            drop(connected);
            std::process::exit(0);
        }
        Some("remove" | "rm") => {
            let project = args[1..].iter().any(|a| a == "--project");
            let Some(name) = args[1..].iter().find(|a| !a.starts_with('-')) else { usage_error("rak mcp remove needs a name") };
            let (file, other, other_flag) =
                if project { (project_file, user_file, "without --project") } else { (user_file, project_file, "with --project") };
            if mcp::remove_from_file(&file, name).unwrap_or_else(|why| fail(why)) {
                println!("Removed {name} from {}", file.display());
                std::process::exit(0);
            }
            let elsewhere = std::fs::read_to_string(&other)
                .ok()
                .and_then(|text| mcp::parse_config(&text).ok())
                .is_some_and(|list| list.iter().any(|s| &s.name == name));
            if elsewhere {
                fail(format!("{name} is not in {}, but it is in {}. Run it again {other_flag}.", file.display(), other.display()));
            }
            fail(format!("no server named {name} in {}", file.display()));
        }
        Some("-h" | "--help" | "help") | None => {
            print!("{MCP_HELP}");
            std::process::exit(0);
        }
        Some(other) => usage_error(&format!("rak mcp has add, list and remove, not `{other}` (see rak mcp --help)")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_json_a_pipe_or_nothing_is_the_new_form_and_a_name_is_the_old() {
        let args = |words: &[&str]| words.iter().map(|w| w.to_string()).collect::<Vec<String>>();
        assert_eq!(in_hand(&args(&[])), Some(String::new()));
        assert_eq!(in_hand(&args(&["--project", "-y"])), Some(String::new()));
        assert_eq!(in_hand(&args(&["https://github.com/o/r", "--project"])).as_deref(), Some("https://github.com/o/r"));
        assert_eq!(in_hand(&args(&["{\"mcpServers\":{}}"])).as_deref(), Some("{\"mcpServers\":{}}"));
        assert_eq!(in_hand(&args(&["-"])).as_deref(), Some("-"));
        assert_eq!(in_hand(&args(&["files", "--", "npx", "-y", "fs"])), None);
        assert_eq!(in_hand(&args(&["docs", "--url", "https://example.com/mcp"])), None);
        assert_eq!(in_hand(&args(&["files"])), None);
    }
}
