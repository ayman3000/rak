//! The `rak` command: one request from the command line, or a plain chat when
//! started without one.

use rak::agent::{system_prompt, Agent, Approval, Config, Event, Outcome, RunResult};
use rak::provider::{one_line, OpenAICompatible};
use rak::tools::{core_tools, Ctx};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::HashSet;
use std::io::{BufRead, IsTerminal, Write};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

const VERSION: &str = env!("CARGO_PKG_VERSION");

const HELP: &str = "\
rak — Rust Agent Kit: a small coding agent for the terminal

USAGE
  rak [options] \"what to do\"      one request, then exit
  rak [options]                   chat (one request per line; /exit to leave)
  echo \"what to do\" | rak         the request from a pipe

MODEL (any server that speaks OpenAI's chat-completions API)
  -m, --model NAME        model to use                      [env RAK_MODEL]
      --base-url URL      server address                    [env RAK_BASE_URL]
                          default: http://localhost:11434/v1 (Ollama)
      --api-key-env NAME  environment variable holding the key
                          default: RAK_API_KEY, then OPENAI_API_KEY
      --effort LEVEL      reasoning effort: low, medium or high
      --max-tokens N      longest reply to ask for
      --context N         the model's context window in tokens (default 128000)

BEHAVIOUR
  -y, --yes               run every tool without asking
      --read-only         no edits and no commands: read and report
      --any-path          let file tools reach outside the project folder
  -C, --cwd DIR           the project folder (default: where rak was started)
      --max-turns N       model calls allowed per request (default 30)

OUTPUT
      --json              print one JSON object with the answer and the numbers
      --log FILE          append every step to FILE as JSON lines
  -q, --quiet             no progress lines, only the answer
  -h, --help              this text
  -V, --version           the version

EXIT CODES
  0 done   1 failed   2 bad usage   3 a tool was not approved   4 turn limit   5 stuck repeating a call
";

struct Options {
    task: Option<String>,
    model: Option<String>,
    base_url: String,
    api_key_env: Option<String>,
    effort: Option<String>,
    max_tokens: Option<u64>,
    context: usize,
    yes: bool,
    read_only: bool,
    any_path: bool,
    cwd: Option<PathBuf>,
    max_turns: usize,
    json: bool,
    log: Option<PathBuf>,
    quiet: bool,
}

fn usage_error(message: &str) -> ! {
    eprintln!("rak: {message}\nTry `rak --help`.");
    std::process::exit(2);
}

fn parse_args(args: Vec<String>) -> Options {
    let env = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    let mut o = Options {
        task: None,
        model: env("RAK_MODEL"),
        base_url: env("RAK_BASE_URL").unwrap_or_else(|| "http://localhost:11434/v1".to_string()),
        api_key_env: None,
        effort: None,
        max_tokens: None,
        context: 128_000,
        yes: false,
        read_only: false,
        any_path: false,
        cwd: None,
        max_turns: 30,
        json: false,
        log: None,
        quiet: false,
    };
    let mut words: Vec<String> = Vec::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().unwrap_or_else(|| usage_error(&format!("{name} needs a value")));
        fn number<T: std::str::FromStr>(name: &str, text: String) -> T {
            text.parse().unwrap_or_else(|_| usage_error(&format!("{name} needs a number, not `{text}`")))
        }
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{HELP}");
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!("rak {VERSION}");
                std::process::exit(0);
            }
            "-m" | "--model" => o.model = Some(value("--model")),
            "--base-url" => o.base_url = value("--base-url"),
            "--api-key-env" => o.api_key_env = Some(value("--api-key-env")),
            "--effort" => {
                let level = value("--effort");
                if !["low", "medium", "high"].contains(&level.as_str()) {
                    usage_error("--effort is low, medium or high");
                }
                o.effort = Some(level);
            }
            "--max-tokens" => o.max_tokens = Some(number("--max-tokens", value("--max-tokens"))),
            "--context" => o.context = number("--context", value("--context")),
            "-y" | "--yes" => o.yes = true,
            "--read-only" => o.read_only = true,
            "--any-path" => o.any_path = true,
            "-C" | "--cwd" => o.cwd = Some(PathBuf::from(value("--cwd"))),
            "--max-turns" => o.max_turns = number::<usize>("--max-turns", value("--max-turns")).max(1),
            "--json" => o.json = true,
            "--log" => o.log = Some(PathBuf::from(value("--log"))),
            "-q" | "--quiet" => o.quiet = true,
            "--" => words.extend(args.by_ref()),
            flag if flag.starts_with('-') && flag.len() > 1 => usage_error(&format!("unknown option {flag}")),
            _ => words.push(arg),
        }
    }
    if !words.is_empty() {
        o.task = Some(words.join(" "));
    }
    o
}

/// Appends one JSON object per line. Errors are ignored: a log that cannot be
/// written must not stop the work.
#[derive(Clone)]
struct Log(Option<Rc<RefCell<std::fs::File>>>);

impl Log {
    fn open(path: Option<&PathBuf>) -> Log {
        let file = path.map(|p| {
            std::fs::OpenOptions::new().create(true).append(true).open(p).unwrap_or_else(|e| {
                eprintln!("rak: cannot open the log {}: {e}", p.display());
                std::process::exit(1);
            })
        });
        Log(file.map(|f| Rc::new(RefCell::new(f))))
    }
    fn write(&self, mut entry: Value) {
        if let Some(file) = &self.0 {
            let seconds = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
            entry["time"] = json!((seconds * 1000.0).round() / 1000.0);
            let _ = writeln!(file.borrow_mut(), "{entry}");
        }
    }
}

/// `edit_file src/a.rs (3 → 5 lines)`: enough to follow along, never the
/// whole text of an edit.
fn describe(name: &str, args: &Value) -> String {
    let text = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or("");
    let lines = |key: &str| text(key).split('\n').count();
    match name {
        "edit_file" => format!("edit_file {}  ({} → {} lines)", text("path"), lines("old_text"), lines("new_text")),
        "write_file" => format!("write_file {}  ({} bytes)", text("path"), text("content").len()),
        "run_shell" => format!("run_shell {}", one_line(text("command"), 110)),
        _ => {
            let mut parts: Vec<String> = args
                .as_object()
                .into_iter()
                .flatten()
                .map(|(k, v)| format!("{k}={}", one_line(&v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()), 60)))
                .collect();
            parts.sort();
            format!("{name} {}", parts.join(" "))
        }
    }
}

fn exit_code(outcome: &Outcome) -> i32 {
    match outcome {
        Outcome::Done => 0,
        Outcome::Failed(_) => 1,
        Outcome::Refused { .. } => 3,
        Outcome::MaxTurns => 4,
        Outcome::Loop => 5,
    }
}

fn outcome_name(outcome: &Outcome) -> &'static str {
    match outcome {
        Outcome::Done => "done",
        Outcome::Failed(_) => "failed",
        Outcome::Refused { .. } => "refused",
        Outcome::MaxTurns => "max_turns",
        Outcome::Loop => "loop",
    }
}

fn main() {
    let o = parse_args(std::env::args().skip(1).collect());

    let cwd = o.cwd.clone().unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    if !cwd.is_dir() {
        usage_error(&format!("{} is not a folder", cwd.display()));
    }
    let Some(model_name) = o.model.clone() else {
        usage_error("no model named. Pass --model NAME or set RAK_MODEL");
    };
    let api_key = match &o.api_key_env {
        Some(name) => Some(std::env::var(name).unwrap_or_else(|_| usage_error(&format!("{name} is not set")))),
        None => ["RAK_API_KEY", "OPENAI_API_KEY"].iter().find_map(|n| std::env::var(n).ok().filter(|v| !v.is_empty())),
    };

    let mut provider = OpenAICompatible::new(&o.base_url, api_key, &model_name);
    provider.effort = o.effort.clone();
    provider.max_tokens = o.max_tokens;

    let ctx = Ctx::new(&cwd, o.any_path);
    let notes = std::fs::read_to_string(ctx.cwd.join("AGENTS.md")).ok().map(|n| n.chars().take(8_000).collect::<String>());
    let config = Config {
        system_prompt: system_prompt(&ctx.cwd, o.read_only, notes.as_deref()),
        max_turns: o.max_turns,
        context_window: o.context,
    };
    let mut agent = Agent::new(Box::new(provider), core_tools(&ctx, o.read_only), config);

    let log = Log::open(o.log.as_ref());
    let show = !o.quiet && !o.json;
    let color = show && std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    let paint = move |code: &str, text: &str| if color { format!("\x1b[{code}m{text}\x1b[0m") } else { text.to_string() };

    // Approvals: -y runs everything; otherwise ask at the terminal, and with no
    // terminal to ask, stop rather than let the model look for another way.
    let (auto, interactive) = (o.yes, std::io::stdin().is_terminal() && std::io::stderr().is_terminal());
    let mut always: HashSet<String> = HashSet::new();
    let mut everything = false;
    let asked_via_pipe = !std::io::stdin().is_terminal();
    agent.on_approval(move |name, args| {
        if auto || everything || always.contains(name) {
            return Approval::Yes;
        }
        if !interactive || asked_via_pipe {
            eprintln!("  {name} needs approval and there's no terminal to ask — stopping (use -y to allow)");
            return Approval::No;
        }
        eprint!("? allow {}\n  [y]es  [n]o  [a]lways for {name}  [A] everything this session › ", describe(name, args));
        let _ = std::io::stderr().flush();
        let mut typed = String::new();
        let _ = std::io::stdin().lock().read_line(&mut typed);
        match typed.trim() {
            "y" | "Y" | "yes" => Approval::Yes,
            "a" | "always" => {
                always.insert(name.to_string());
                Approval::Yes
            }
            "A" | "all" => {
                everything = true;
                Approval::Yes
            }
            _ => Approval::No,
        }
    });

    let event_log = log.clone();
    agent.on_event(move |event| match event {
        Event::ModelCalled { turn, usage, millis } => {
            event_log.write(json!({ "event": "model_call", "turn": turn, "input_tokens": usage.input, "output_tokens": usage.output, "ms": millis }));
        }
        Event::Retrying { attempt, error } => {
            event_log.write(json!({ "event": "retry", "attempt": attempt, "error": error }));
            if show {
                eprintln!("{}", paint("33", &format!("  retrying model call ({attempt}): {}", one_line(error, 120))));
            }
        }
        Event::Text(text) => {
            event_log.write(json!({ "event": "text", "text": text }));
            if show {
                eprintln!("{}", text.trim());
            }
        }
        Event::ToolStarted { name, args } => {
            event_log.write(json!({ "event": "tool_call", "tool": name, "arguments": args }));
            if show {
                eprintln!("{}", paint("36", &format!("→ {}", describe(name, args))));
            }
        }
        Event::ToolFinished { name, ok, denied, text, millis } => {
            event_log.write(json!({ "event": "tool_result", "tool": name, "ok": ok, "denied": denied, "ms": millis, "result": text }));
            if show {
                let line = if *denied { "  ✗ not allowed".to_string() } else { format!("  {} {}", if *ok { "✓" } else { "✗" }, one_line(text, 110)) };
                eprintln!("{}", paint(if *denied { "33" } else if *ok { "2" } else { "31" }, &line));
            }
        }
        Event::Note(note) => {
            event_log.write(json!({ "event": "note", "note": note }));
            if show {
                eprintln!("{}", paint("2", &format!("  {note}")));
            }
        }
    });

    log.write(json!({ "event": "start", "version": VERSION, "model": model_name, "base_url": o.base_url, "cwd": ctx.cwd,
        "effort": o.effort, "auto_approve": o.yes, "read_only": o.read_only, "tools": agent.tool_names() }));

    // One request: from the arguments, or from a pipe.
    let piped = (!std::io::stdin().is_terminal() && o.task.is_none()).then(|| {
        let mut text = String::new();
        let _ = std::io::Read::read_to_string(&mut std::io::stdin(), &mut text);
        text.trim().to_string()
    });
    if let Some(task) = o.task.clone().or(piped) {
        if task.is_empty() {
            usage_error("nothing to do: the request was empty");
        }
        let code = run_one(&mut agent, &task, &o, &log, show);
        std::process::exit(code);
    }

    // Chat.
    eprintln!("rak {VERSION} · {model_name} · {}\nOne request per line. /exit to leave.", ctx.cwd.display());
    let stdin = std::io::stdin();
    loop {
        eprint!("{}", paint("36", "❯ "));
        let _ = std::io::stderr().flush();
        let mut line = String::new();
        if stdin.lock().read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        match line.trim() {
            "" => continue,
            "/exit" | "/quit" => break,
            task => {
                run_one(&mut agent, task, &o, &log, show);
            }
        }
    }
}

fn run_one(agent: &mut Agent, task: &str, o: &Options, log: &Log, show: bool) -> i32 {
    let started = Instant::now();
    let (usage_before, calls_before) = (agent.usage, agent.tool_calls);
    log.write(json!({ "event": "request", "text": task }));
    let RunResult { outcome, answer, turns } = agent.run(task);
    let seconds = (started.elapsed().as_secs_f64() * 10.0).round() / 10.0;
    let input = agent.usage.input - usage_before.input;
    let output = agent.usage.output - usage_before.output;
    let tool_calls = agent.tool_calls - calls_before;

    let problem = match &outcome {
        Outcome::Done => None,
        Outcome::Failed(why) => Some(why.clone()),
        Outcome::Refused { tool } => Some(format!("stopped: {tool} was not approved. Nothing was changed by it.")),
        Outcome::MaxTurns => Some(format!("stopped at the turn limit ({}). Raise it with --max-turns.", o.max_turns)),
        Outcome::Loop => Some("stopped: the model kept repeating the same call with the same result.".to_string()),
    };
    let summary = json!({
        "outcome": outcome_name(&outcome), "answer": answer, "error": problem, "turns": turns, "tool_calls": tool_calls,
        "input_tokens": input, "output_tokens": output, "seconds": seconds,
    });
    let mut entry = summary.clone();
    entry["event"] = json!("finish");
    log.write(entry);

    if o.json {
        println!("{summary}");
    } else {
        if !answer.trim().is_empty() {
            println!("{}", answer.trim());
        }
        if let Some(problem) = &problem {
            eprintln!("rak: {problem}");
        }
        if show {
            eprintln!("  {turns} turns · {tool_calls} tool calls · {input} tokens in · {output} out · {seconds} s");
        }
    }
    exit_code(&outcome)
}
