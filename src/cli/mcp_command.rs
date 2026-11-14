//! `rak mcp add|list|remove`: edits the list of MCP servers from the shell.

use super::args::usage_error;
use super::settings::config_dir;
use super::style::cut;
use rak::interrupt::Stop;
use rak::mcp;
use std::path::PathBuf;

pub const MCP_HELP: &str = "\
rak mcp — tools from MCP servers

  rak mcp add NAME [options] -- COMMAND [ARG…]    a program rak starts
  rak mcp add NAME [options] --url URL            a server at a web address
  rak mcp list                                    the servers, and whether each connects
  rak mcp remove NAME [--project]

OPTIONS FOR add
  -e, --env KEY=VALUE     an environment variable for the program (repeatable)
  -H, --header 'N: V'     a header sent to the web address (repeatable)
      --timeout SECONDS   how long one tool call may take (default 120)
      --project           write ./.rak/mcp.json (this project only) instead of
                          the file for all projects

EXAMPLES
  rak mcp add files -- npx -y @modelcontextprotocol/server-filesystem .
  rak mcp add docs --url https://example.com/mcp -H 'Authorization: Bearer KEY'

The files are in the shape Claude Code and Cursor use: an entry can be pasted
across. A project's file wins over the one for all projects.
";

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
                println!("No MCP servers yet. Add one with:\n  rak mcp add NAME -- COMMAND [ARG…]\n  rak mcp add NAME --url URL");
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
