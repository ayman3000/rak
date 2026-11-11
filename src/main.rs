//! The `rak` command: one request from the command line, or a chat when
//! started without one. The chat's interface (src/cli) follows sak's.

mod cli;

use cli::editor::Editor;
use cli::style::{pad, paint, Style};
use cli::{commands, markdown, menu, term, welcome};
use rak::agent::{system_prompt, Agent, Approval, Config, Event, Outcome, RunResult};
use rak::interrupt::Stop;
use rak::provider::{one_line, OpenAICompatible};
use rak::tools::{core_tools, Ctx};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
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
  rak [options]                   chat (type / for the commands, /exit to leave)
  echo \"what to do\" | rak         the request from a pipe

MODEL (any server that speaks OpenAI's chat-completions API)
  -m, --model NAME        model to use                      [env RAK_MODEL]
      --base-url URL      server address                    [env RAK_BASE_URL]
                          default: http://localhost:11434/v1 (Ollama)
      --api-key-env NAME  environment variable holding the key
                          default: RAK_API_KEY, then OPENAI_API_KEY
      --effort LEVEL      reasoning effort: low, medium or high
      --max-tokens N      longest reply to ask for
      --context N         the model's context window in tokens
                          default: what an Ollama server reports, else 128000
      --no-stream         wait for each whole reply instead of showing it as it arrives

  In a chat, the model can be left out: rak uses the one picked last with
  /model, or lists the server's models to pick from.

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
  130 stopped with Ctrl-C
";

struct Options {
    task: Option<String>,
    model: Option<String>,
    base_url: String,
    api_key_env: Option<String>,
    effort: Option<String>,
    max_tokens: Option<u64>,
    context: Option<usize>,
    no_stream: bool,
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
        context: None,
        no_stream: false,
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
            "--context" => o.context = Some(number("--context", value("--context"))),
            "--no-stream" => o.no_stream = true,
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
        Outcome::Stopped => 130,
    }
}

fn outcome_name(outcome: &Outcome) -> &'static str {
    match outcome {
        Outcome::Done => "done",
        Outcome::Failed(_) => "failed",
        Outcome::Refused { .. } => "refused",
        Outcome::MaxTurns => "max_turns",
        Outcome::Loop => "loop",
        Outcome::Stopped => "stopped",
    }
}

/// What is on screen, shared by everything that prints during a request.
struct Ui {
    /// Chat: progress goes to stdout with the answer. One request: to stderr,
    /// so `rak "…" > answer.txt` holds only the answer.
    chat: bool,
    /// Progress lines are shown at all.
    show: bool,
    out_color: bool,
    err_color: bool,
    /// The answer's text is printed as it arrives.
    live: bool,
    renderer: Option<markdown::Renderer>,
    /// The last text printed did not end its line.
    line_open: bool,
}

impl Ui {
    /// A piece of the answer, as it arrives.
    fn delta(&mut self, piece: &str) {
        if !self.live {
            return;
        }
        match &mut self.renderer {
            // Markdown is shown a whole line at a time.
            Some(renderer) => print!("{}", renderer.push(piece)),
            None => {
                print!("{piece}");
                self.line_open = !piece.ends_with('\n');
            }
        }
        let _ = std::io::stdout().flush();
    }

    /// Writes out answer text still held back and ends its line. Before a
    /// tool runs or asks, so what the model said before the call is shown
    /// before it, and at the end of the answer.
    fn end_line(&mut self) {
        if let Some(renderer) = &mut self.renderer {
            let rest = renderer.finish();
            if !rest.is_empty() {
                println!("{rest}");
            }
        }
        if self.line_open {
            println!();
            self.line_open = false;
        }
        let _ = std::io::stdout().flush();
    }

    /// A progress line, styled for whichever stream it lands on.
    fn progress(&mut self, style: Option<Style>, text: &str) {
        if !self.show {
            return;
        }
        self.end_line();
        let styled = |color: bool| style.map(|s| paint(color, s, text)).unwrap_or_else(|| text.to_string());
        if self.chat {
            println!("{}", styled(self.out_color));
        } else {
            eprintln!("{}", styled(self.err_color));
        }
    }

    /// A whole answer that was not shown as it arrived.
    fn answer(&mut self, text: &str) {
        match &mut self.renderer {
            Some(_) => {
                let mut renderer = markdown::Renderer::default();
                print!("{}", renderer.push(&format!("{text}\n")));
            }
            None => println!("{text}"),
        }
    }
}

/// Ctrl-C while a request runs asks it to stop; a second one quits.
#[cfg(unix)]
mod sigint {
    use rak::interrupt::Stop;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::OnceLock;

    static STOP: OnceLock<Stop> = OnceLock::new();
    pub static RUNNING: AtomicBool = AtomicBool::new(false);

    extern "C" fn handle(_: libc::c_int) {
        let first = STOP.get().filter(|stop| RUNNING.load(Ordering::SeqCst) && !stop.requested());
        match first {
            Some(stop) => {
                stop.request();
                let note = "\n⏹ stopping… (Ctrl-C again quits rak)\n".as_bytes();
                unsafe { libc::write(2, note.as_ptr() as *const libc::c_void, note.len()) };
            }
            None => unsafe { libc::_exit(130) },
        }
    }

    pub fn install(stop: Stop) {
        let _ = STOP.set(stop);
        unsafe { libc::signal(libc::SIGINT, handle as extern "C" fn(libc::c_int) as libc::sighandler_t) };
    }
}

fn set_running(_on: bool) {
    #[cfg(unix)]
    sigint::RUNNING.store(_on, std::sync::atomic::Ordering::SeqCst);
}

/// Where rak keeps the history and the model picked last.
fn config_dir() -> Option<PathBuf> {
    let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    var("XDG_CONFIG_HOME")
        .or_else(|| var("HOME").map(|home| home.join(".config")))
        .or_else(|| var("APPDATA"))
        .map(|base| base.join("rak"))
}

fn home() -> String {
    std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_default()
}

fn saved_model() -> Option<String> {
    let text = std::fs::read_to_string(config_dir()?.join("model")).ok()?;
    Some(text.trim().to_string()).filter(|m| !m.is_empty())
}

fn save_model(model: &str) {
    if let Some(dir) = config_dir() {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("model"), format!("{model}\n"));
    }
}

/// A progress line cut to the terminal's width, so it stays one line.
fn fit(line: &str) -> String {
    cut(line, term::size().0.saturating_sub(1).max(20))
}

fn cut(line: &str, columns: usize) -> String {
    if cli::style::text_width(line) <= columns {
        return line.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in line.chars() {
        used += cli::style::char_width(c);
        if used >= columns {
            break;
        }
        out.push(c);
    }
    out + "…"
}

/// 1234567 → "1,234,567"
fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Everything one run of rak holds.
struct Session {
    agent: Agent,
    o: Options,
    log: Log,
    ui: Rc<RefCell<Ui>>,
    auto: Rc<Cell<bool>>,
    stop: Stop,
    cwd: PathBuf,
    model: String,
    api_key: Option<String>,
    window: usize,
    /// Where the window's number came from, for /usage.
    window_source: &'static str,
    notes: Option<String>,
    requests: usize,
}

fn make_provider(o: &Options, api_key: &Option<String>, model: &str, stop: &Stop) -> OpenAICompatible {
    let mut provider = OpenAICompatible::new(&o.base_url, api_key.clone(), model);
    provider.effort = o.effort.clone();
    provider.max_tokens = o.max_tokens;
    provider.stop = stop.clone();
    provider
}

/// --context wins; then what an Ollama server says; then a guess.
fn window_for(o: &Options, provider: &OpenAICompatible) -> (usize, &'static str) {
    match (o.context, provider.context_window()) {
        (Some(n), _) => (n, "set with --context"),
        (None, Some(n)) => (n, "reported by the server"),
        (None, None) => (128_000, "assumed — the server didn't say; set it with --context"),
    }
}

fn main() {
    let o = parse_args(std::env::args().skip(1).collect());

    let cwd = o.cwd.clone().unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    if !cwd.is_dir() {
        usage_error(&format!("{} is not a folder", cwd.display()));
    }
    let api_key = match &o.api_key_env {
        Some(name) => Some(std::env::var(name).unwrap_or_else(|_| usage_error(&format!("{name} is not set")))),
        None => ["RAK_API_KEY", "OPENAI_API_KEY"].iter().find_map(|n| std::env::var(n).ok().filter(|v| !v.is_empty())),
    };

    let stdin_tty = std::io::stdin().is_terminal();
    let stdout_tty = std::io::stdout().is_terminal();
    let chat = o.task.is_none() && stdin_tty && !o.json;
    let no_color = std::env::var_os("NO_COLOR").is_some();
    let out_color = stdout_tty && !no_color;
    let err_color = std::io::stderr().is_terminal() && !no_color;

    // One stop flag for the loop, the model stream and a running command.
    let ctx = Ctx::new(&cwd, o.any_path);
    let stop = ctx.stop.clone();
    #[cfg(unix)]
    sigint::install(stop.clone());

    let mut editor = Editor::new(config_dir().map(|d| d.join("history")), menu::commands(commands::HELP), out_color);

    // The model: named, or in a chat the one picked last, or picked now.
    let model_name = match o.model.clone().or_else(|| if chat { saved_model() } else { None }) {
        Some(name) => name,
        None if chat => {
            println!("No model named yet. These are the models at {}:", welcome::server_name(&o.base_url));
            let picked = pick_model(&make_provider(&o, &api_key, "", &stop), None, &mut editor, out_color);
            let Some(name) = picked else { usage_error("no model named. Pass --model NAME or set RAK_MODEL") };
            save_model(&name);
            name
        }
        None => usage_error("no model named. Pass --model NAME or set RAK_MODEL"),
    };

    let provider = make_provider(&o, &api_key, &model_name, &stop);
    let (window, window_source) = window_for(&o, &provider);

    let notes = std::fs::read_to_string(ctx.cwd.join("AGENTS.md")).ok().map(|n| n.chars().take(8_000).collect::<String>());
    let config = Config {
        system_prompt: system_prompt(&ctx.cwd, o.read_only, notes.as_deref()),
        max_turns: o.max_turns,
        context_window: window,
    };
    let mut agent = Agent::new(Box::new(provider), core_tools(&ctx, o.read_only), config);
    agent.stop = stop.clone();

    let log = Log::open(o.log.as_ref());
    let live = stdout_tty && !o.json && !o.no_stream;
    agent.stream = live;
    let ui = Rc::new(RefCell::new(Ui {
        chat,
        show: !o.quiet && !o.json,
        out_color,
        err_color,
        live,
        renderer: (out_color && !o.json).then(markdown::Renderer::default),
        line_open: false,
    }));

    // Approvals: -y (or /yes) runs everything; otherwise ask at the terminal,
    // and with no terminal to ask, stop rather than let the model look for
    // another way.
    let auto = Rc::new(Cell::new(o.yes));
    let interactive = stdin_tty && std::io::stderr().is_terminal();
    let mut always: HashSet<String> = HashSet::new();
    let mut everything = false;
    let (approval_ui, approval_auto, approval_stop) = (ui.clone(), auto.clone(), stop.clone());
    agent.on_approval(move |name, args| {
        if approval_auto.get() || everything || always.contains(name) {
            return Approval::Yes;
        }
        approval_ui.borrow_mut().end_line();
        if !interactive {
            eprintln!("  {name} needs approval and there's no terminal to ask — stopping (use -y to allow)");
            return Approval::No;
        }
        let question = format!("? allow {}\n  [y]es  [n]o  [a]lways for {name}  [A] everything this session › ", describe(name, args));
        eprint!("{}", paint(err_color, Style::Yellow, &question));
        let _ = std::io::stderr().flush();
        let mut typed = String::new();
        let _ = std::io::stdin().lock().read_line(&mut typed);
        if approval_stop.requested() {
            return Approval::No;
        }
        match typed.trim() {
            "y" | "Y" | "yes" => Approval::Yes,
            "a" | "always" => {
                always.insert(name.to_string());
                Approval::Yes
            }
            "A" | "all" | "everything" => {
                everything = true;
                Approval::Yes
            }
            _ => Approval::No,
        }
    });

    let (event_log, event_ui) = (log.clone(), ui.clone());
    agent.on_event(move |event| {
        let mut ui = event_ui.borrow_mut();
        match event {
            Event::ModelCalled { turn, usage, millis } => {
                event_log.write(json!({ "event": "model_call", "turn": turn, "input_tokens": usage.input, "output_tokens": usage.output, "ms": millis }));
            }
            Event::Retrying { attempt, error } => {
                event_log.write(json!({ "event": "retry", "attempt": attempt, "error": error }));
                ui.progress(Some(Style::Yellow), &format!("  retrying model call ({attempt}): {}", one_line(error, 120)));
            }
            Event::Delta(piece) => ui.delta(piece),
            Event::Text(text) => {
                event_log.write(json!({ "event": "text", "text": text }));
                if !ui.live {
                    ui.progress(None, text.trim());
                }
            }
            Event::ToolStarted { name, args } => {
                event_log.write(json!({ "event": "tool_call", "tool": name, "arguments": args }));
                ui.progress(Some(Style::Cyan), &fit(&format!("→ {}", describe(name, args))));
            }
            Event::ToolFinished { name, ok, denied, text, millis } => {
                event_log.write(json!({ "event": "tool_result", "tool": name, "ok": ok, "denied": denied, "ms": millis, "result": text }));
                if *denied {
                    ui.progress(Some(Style::Yellow), "  ✗ not allowed");
                } else {
                    let line = format!("  {} {}", if *ok { "✓" } else { "✗" }, one_line(text, 110));
                    ui.progress(Some(if *ok { Style::Dim } else { Style::Red }), &fit(&line));
                }
            }
            Event::Note(note) => {
                event_log.write(json!({ "event": "note", "note": note }));
                ui.progress(Some(Style::Dim), &format!("  {note}"));
            }
        }
    });

    log.write(json!({ "event": "start", "version": VERSION, "model": model_name, "base_url": o.base_url, "cwd": ctx.cwd,
        "effort": o.effort, "auto_approve": o.yes, "read_only": o.read_only, "tools": agent.tool_names() }));

    let mut session = Session {
        agent,
        o,
        log,
        ui,
        auto,
        stop,
        cwd: ctx.cwd.clone(),
        model: model_name,
        api_key,
        window,
        window_source,
        notes,
        requests: 0,
    };

    if chat {
        run_chat(&mut session, &mut editor);
        return;
    }

    // One request: from the arguments, or from a pipe.
    let piped = (!stdin_tty && session.o.task.is_none()).then(|| {
        let mut text = String::new();
        let _ = std::io::Read::read_to_string(&mut std::io::stdin(), &mut text);
        text.trim().to_string()
    });
    let Some(task) = session.o.task.clone().or(piped).filter(|t| !t.is_empty()) else {
        usage_error("nothing to do: give a request, or pipe one in");
    };
    let outcome = run_one(&mut session, &task);
    std::process::exit(exit_code(&outcome));
}

/// Runs one request and prints what came of it.
fn run_one(s: &mut Session, task: &str) -> Outcome {
    let started = Instant::now();
    let (usage_before, calls_before) = (s.agent.usage, s.agent.tool_calls);
    s.log.write(json!({ "event": "request", "text": task }));
    s.requests += 1;
    s.stop.clear();
    set_running(true);
    let RunResult { outcome, answer, turns } = s.agent.run(task);
    set_running(false);
    let seconds = (started.elapsed().as_secs_f64() * 10.0).round() / 10.0;
    let input = s.agent.usage.input - usage_before.input;
    let output = s.agent.usage.output - usage_before.output;
    let tool_calls = s.agent.tool_calls - calls_before;

    let problem = match &outcome {
        Outcome::Done => None,
        Outcome::Failed(why) => Some(why.clone()),
        Outcome::Refused { tool } if s.ui.borrow().chat => {
            Some(format!("stopped: you said no to {tool}. Nothing was changed by it — say what to do instead."))
        }
        Outcome::Refused { tool } => Some(format!("stopped: {tool} was not approved. Nothing was changed by it.")),
        Outcome::MaxTurns => Some(format!("stopped at the turn limit ({}). Raise it with --max-turns.", s.o.max_turns)),
        Outcome::Loop => Some("stopped: the model kept repeating the same call with the same result.".to_string()),
        Outcome::Stopped => Some("stopped".to_string()),
    };
    let summary = json!({
        "outcome": outcome_name(&outcome), "answer": answer, "error": problem, "turns": turns, "tool_calls": tool_calls,
        "input_tokens": input, "output_tokens": output, "seconds": seconds,
    });
    let mut entry = summary.clone();
    entry["event"] = json!("finish");
    s.log.write(entry);

    if s.o.json {
        println!("{summary}");
        return outcome;
    }
    let mut ui = s.ui.borrow_mut();
    ui.end_line();
    if !ui.live && !answer.trim().is_empty() {
        ui.answer(answer.trim());
    }
    if let Some(problem) = &problem {
        if ui.chat {
            let failed = matches!(outcome, Outcome::Failed(_));
            let text = if failed { format!("error: {problem}") } else { format!("({problem})") };
            println!("{}", paint(ui.out_color, if failed { Style::Red } else { Style::Yellow }, &text));
        } else {
            eprintln!("rak: {problem}");
        }
    }
    ui.progress(Some(Style::Dim), &format!("  {turns} turns · {tool_calls} tool calls · {input} tokens in · {output} out · {seconds} s"));
    outcome
}

// MARK: - Chat

fn mode_label(s: &Session) -> &'static str {
    if s.o.read_only {
        "read-only"
    } else if s.auto.get() {
        "auto mode"
    } else {
        "asks before changes"
    }
}

fn banner(s: &Session) {
    let info = welcome::Info {
        version: VERSION.to_string(),
        model: s.model.clone(),
        server: welcome::server_name(&s.o.base_url),
        folder: welcome::short_path(&s.cwd.to_string_lossy(), &home(), 24),
        branch: welcome::git_branch(&s.cwd),
        mode: mode_label(s).to_string(),
        window: s.window,
        notes: s.notes.iter().map(|_| ("notes".to_string(), "AGENTS.md".to_string())).collect(),
        tool_groups: welcome::group(&s.agent.tool_names()),
        tool_count: s.agent.tool_names().len(),
        warnings: Vec::new(),
    };
    for line in welcome::panel(&info, term::size().0, s.ui.borrow().out_color) {
        println!("{line}");
    }
}

fn run_chat(s: &mut Session, editor: &mut Editor) {
    banner(s);
    let slash_commands = menu::commands(commands::HELP);
    let color = s.ui.borrow().out_color;
    loop {
        let folder = s.cwd.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        println!();
        println!(
            "{}",
            welcome::status_rule(
                &s.model,
                s.agent.last_prompt_tokens as usize,
                s.window,
                &folder,
                welcome::git_branch(&s.cwd).as_deref(),
                mode_label(s),
                term::size().0,
                color
            )
        );
        let Some(mut line) = editor.read_line("❯ ", Style::C45, true) else { break };
        // A trailing backslash continues the request on the next line.
        while line.ends_with('\\') {
            line.pop();
            match editor.read_line("… ", Style::Dim, false) {
                Some(more) => line = format!("{line}\n{more}"),
                None => break,
            }
        }
        // "/mo" + Enter runs the menu's first match, "/model".
        let input = menu::resolve(line.trim(), &slash_commands, commands::KNOWN);
        if input.is_empty() {
            continue;
        }
        if input.starts_with('/') {
            if !run_command(s, editor, &input) {
                break;
            }
            continue;
        }
        println!();
        run_one(s, &input);
    }
    println!("{}", paint(color, Style::Dim, "bye"));
}

/// Lists the server's models, numbered. None when the server wouldn't say.
fn list_models(provider: &OpenAICompatible, current: Option<&str>, color: bool) -> Option<Vec<String>> {
    match provider.list_models() {
        Ok(mut models) => {
            // Only models that can chat.
            models.retain(|m| !m.to_lowercase().contains("embed"));
            if models.len() > 60 {
                println!("{}", paint(color, Style::Dim, &format!("({} models — showing the first 60; type any name to use it)", models.len())));
                models.truncate(60);
            }
            for (i, model) in models.iter().enumerate() {
                let marker = if current == Some(model.as_str()) { paint(color, Style::Green, " ← current") } else { String::new() };
                println!("  {:>3}. {model}{marker}", i + 1);
            }
            Some(models)
        }
        Err(why) => {
            println!("{}", paint(color, Style::Yellow, &format!("Couldn't list the models: {why}")));
            None
        }
    }
}

/// Lists the models and lets the person pick one by number or name.
fn pick_model(provider: &OpenAICompatible, current: Option<&str>, editor: &mut Editor, color: bool) -> Option<String> {
    let models = list_models(provider, current, color)?;
    let hint = if current.is_some() { "Pick a number, or Enter to keep the current model:" } else { "Pick a number, or type a model's name:" };
    println!("{}", paint(color, Style::Dim, hint));
    let answer = editor.read_line("model › ", Style::C45, false)?.trim().to_string();
    if answer.is_empty() {
        return None;
    }
    match answer.parse::<usize>() {
        Ok(n) if (1..=models.len()).contains(&n) => Some(models[n - 1].clone()),
        _ => Some(answer),
    }
}

fn switch_model(s: &mut Session, model: &str) {
    let color = s.ui.borrow().out_color;
    let provider = make_provider(&s.o, &s.api_key, model, &s.stop);
    (s.window, s.window_source) = window_for(&s.o, &provider);
    s.agent.set_context_window(s.window);
    s.agent.set_model(Box::new(provider));
    s.model = model.to_string();
    save_model(model);
    s.log.write(json!({ "event": "model", "model": model }));
    println!("Now using {}.", paint(color, Style::Bold, model));
}

/// Runs a slash command. False means leave the chat.
fn run_command(s: &mut Session, editor: &mut Editor, line: &str) -> bool {
    let color = s.ui.borrow().out_color;
    let dim = |text: &str| paint(color, Style::Dim, text);
    let (name, args) = line[1..].split_once(' ').unwrap_or((&line[1..], ""));
    let args = args.trim();
    match name.to_lowercase().as_str() {
        "help" | "?" => {
            for (usage, about) in commands::HELP {
                println!("  {} {about}", paint(color, Style::Cyan, &pad(usage, 16)));
            }
            println!("{}", dim("\n  Ctrl-C stops a running request · ↑/↓ history · Tab completes a command\n  Multi-line: end a line with \\ to continue it, or paste"));
        }
        "model" | "m" if !args.is_empty() => switch_model(s, args),
        "model" | "m" => {
            println!("Model: {} at {}", paint(color, Style::Bold, &s.model), welcome::server_name(&s.o.base_url));
            let provider = make_provider(&s.o, &s.api_key, &s.model, &s.stop);
            if let Some(model) = pick_model(&provider, Some(&s.model), editor, color) {
                switch_model(s, &model);
            }
        }
        "models" => {
            let provider = make_provider(&s.o, &s.api_key, &s.model, &s.stop);
            if list_models(&provider, Some(&s.model), color).is_none() {
                println!("{}", dim("You can still switch by name: /model <name>"));
            }
        }
        "tools" => {
            let mut tools: Vec<_> = s.agent.tools().iter().collect();
            tools.sort_by_key(|t| t.name());
            for tool in tools {
                let gate = if tool.needs_approval() && !s.auto.get() { paint(color, Style::Yellow, " (asks)") } else { String::new() };
                let first = tool.description().split('.').next().unwrap_or("");
                println!("  {}{gate}  {}", paint(color, Style::Cyan, tool.name()), dim(&one_line(first, 80)));
            }
            if s.o.read_only {
                println!("{}", dim("  read-only mode: no edit or shell tools"));
            }
        }
        "notes" => match &s.notes {
            Some(notes) => println!("{}", notes.trim()),
            None => println!("{}", dim("No project notes (no AGENTS.md in this folder).")),
        },
        "yes" | "auto" => {
            s.auto.set(!s.auto.get());
            if s.auto.get() {
                println!("{}", paint(color, Style::Yellow, "Auto-approve ON: edits and shell commands run without asking."));
            } else {
                println!("Auto-approve off: rak asks before edits and shell commands.");
            }
        }
        "usage" | "cost" => {
            println!("  model          {} ({})", s.model, welcome::server_name(&s.o.base_url));
            println!("  requests       {}", s.requests);
            println!("  input tokens   {}", grouped(s.agent.usage.input));
            println!("  output tokens  {}", grouped(s.agent.usage.output));
            println!("  tool calls     {}", s.agent.tool_calls);
            println!("  context        ~{} tokens in the last request", grouped(s.agent.last_prompt_tokens));
            println!("  window         {} tokens ({})", grouped(s.window as u64), s.window_source);
        }
        "clear" | "reset" | "new" => {
            s.agent.clear();
            println!("{}", dim("(conversation cleared)"));
        }
        "exit" | "quit" | "q" => return false,
        other => println!("{}", paint(color, Style::Yellow, &format!("Unknown command /{other}. Type /help."))),
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_grouped_in_threes() {
        for (n, text) in [(0, "0"), (999, "999"), (1_000, "1,000"), (12_345, "12,345"), (1_234_567, "1,234,567")] {
            assert_eq!(grouped(n), text);
        }
    }

    #[test]
    fn long_progress_lines_are_cut_to_fit() {
        assert_eq!(cut("short", 20), "short");
        assert_eq!(cut("exactly ten", 11), "exactly ten");
        let line = cut(&"x".repeat(200), 40);
        assert_eq!(cli::style::text_width(&line), 40);
        assert!(line.ends_with('…'));
    }

    #[test]
    fn every_listed_command_is_known() {
        for (usage, _) in commands::HELP {
            let name = usage.split(' ').next().unwrap();
            assert!(commands::KNOWN.contains(&name), "{name} is in /help but the chat would not take it");
        }
    }
}
