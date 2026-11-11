//! One run of rak: what it holds, how it is set up from the options, and
//! one request run and reported.

use super::args::{usage_error, Options, VERSION};
use super::editor::Editor;
use super::log::Log;
use super::output::{self, Ui};
use super::settings::{config_dir, key, load_keys, save_model, saved_model};
use super::style::{paint, Style};
use super::{approval, chat, commands, markdown, menu, signals, welcome};
use rak::agent::{system_prompt, Agent, Config, Outcome, RunResult};
use rak::interrupt::Stop;
use rak::mcp;
use rak::anthropic::Anthropic;
use rak::provider::{Model, OpenAICompatible};
use rak::tools::web::WebSearch;
use rak::tools::{core_tools, Ctx};
use serde_json::json;
use std::cell::{Cell, RefCell};
use std::io::IsTerminal;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

/// Everything one run of rak holds.
pub struct Session {
    pub agent: Agent,
    pub o: Options,
    pub log: Log,
    pub ui: Rc<RefCell<Ui>>,
    pub auto: Rc<Cell<bool>>,
    pub stop: Stop,
    pub cwd: PathBuf,
    pub model: String,
    pub backend: Backend,
    pub window: usize,
    /// Where the window's number came from, for /usage.
    pub window_source: &'static str,
    pub notes: Option<String>,
    pub requests: usize,
    /// The built-in tools' names, and what the MCP servers brought.
    pub core_tools: Vec<String>,
    pub mcp_servers: Vec<(String, usize)>,
    pub mcp_summary: Option<String>,
    pub mcp_problems: Vec<String>,
    pub server_of: std::collections::HashMap<String, String>,
}

/// Which API rak speaks, where, and with which key.
#[derive(Clone, Debug, PartialEq)]
pub struct Backend {
    pub anthropic: bool,
    pub base_url: String,
    pub api_key: Option<String>,
}

/// Pure — unit-tested. Anthropic's own API when asked for; when the address
/// is Anthropic's; or for a claude-* model when an Anthropic key is set and
/// no other address was given. Otherwise the chat-completions API.
pub fn uses_anthropic(provider: Option<&str>, base_url: Option<&str>, model: Option<&str>, has_anthropic_key: bool) -> bool {
    match (provider, base_url) {
        (Some(provider), _) => provider == "anthropic",
        (None, Some(url)) => url.contains("anthropic.com"),
        (None, None) => has_anthropic_key && model.is_some_and(|m| m.starts_with("claude-")),
    }
}

/// The backend for the options and model, or why there can't be one.
pub fn backend(o: &Options, model: Option<&str>) -> Result<Backend, String> {
    let env = |name: &str| key(name);
    let anthropic = uses_anthropic(o.provider.as_deref(), o.base_url.as_deref(), model, env("ANTHROPIC_API_KEY").is_some());
    let api_key = match &o.api_key_env {
        Some(name) => Some(env(name).ok_or(format!("{name} is not set"))?),
        None if anthropic => env("ANTHROPIC_API_KEY").or_else(|| env("RAK_API_KEY")),
        None => env("RAK_API_KEY").or_else(|| env("OPENAI_API_KEY")),
    };
    if anthropic && api_key.is_none() {
        return Err("Anthropic's API needs a key: set ANTHROPIC_API_KEY (or name another variable with --api-key-env)".to_string());
    }
    let default_url = if anthropic { rak::anthropic::DEFAULT_BASE_URL } else { "http://localhost:11434/v1" };
    Ok(Backend { anthropic, base_url: o.base_url.clone().unwrap_or_else(|| default_url.to_string()), api_key })
}

pub fn make_provider(o: &Options, backend: &Backend, model: &str, stop: &Stop) -> Box<dyn Model> {
    if backend.anthropic {
        let mut provider = Anthropic::new(&backend.base_url, backend.api_key.clone().unwrap_or_default(), model);
        provider.effort = o.effort.clone();
        provider.max_tokens = o.max_tokens;
        provider.stop = stop.clone();
        return Box::new(provider);
    }
    let mut provider = OpenAICompatible::new(&backend.base_url, backend.api_key.clone(), model);
    provider.effort = o.effort.clone();
    provider.max_tokens = o.max_tokens;
    provider.stop = stop.clone();
    Box::new(provider)
}

/// --context wins; then what the server says; then a guess.
pub fn window_for(o: &Options, provider: &dyn Model) -> (usize, &'static str) {
    match (o.context, provider.context_window()) {
        (Some(n), _) => (n, "set with --context"),
        (None, Some(n)) => (n, "reported by the server"),
        (None, None) => (128_000, "assumed — the server didn't say; set it with --context"),
    }
}

/// Sets up a run from the options and does it: a chat, or one request.
/// Returns the exit code.
pub fn run(mut o: Options) -> i32 {
    let cwd = o.cwd.clone().unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    if !cwd.is_dir() {
        usage_error(&format!("{} is not a folder", cwd.display()));
    }
    load_keys(&cwd);
    // Defaults may also come from the .env files.
    o.model = o.model.or_else(|| key("RAK_MODEL"));
    o.provider = o.provider.or_else(|| key("RAK_PROVIDER"));
    o.base_url = o.base_url.or_else(|| key("RAK_BASE_URL"));
    if o.provider.as_deref().is_some_and(|p| p != "openai" && p != "anthropic") {
        usage_error("RAK_PROVIDER is openai or anthropic");
    }

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
    signals::install(stop.clone());

    let mut editor = Editor::new(config_dir().map(|d| d.join("history")), menu::commands(commands::HELP), out_color);

    // The model: named, or in a chat the one picked last, or picked now.
    let model_name = match o.model.clone().or_else(|| if chat { saved_model() } else { None }) {
        Some(name) => name,
        None if chat => {
            let backend = backend(&o, None).unwrap_or_else(|why| usage_error(&why));
            println!("No model named yet. These are the models at {}:", welcome::server_name(&backend.base_url));
            let picked = chat::pick_model(make_provider(&o, &backend, "", &stop).as_ref(), None, &mut editor, out_color);
            let Some(name) = picked else { usage_error("no model named. Pass --model NAME or set RAK_MODEL") };
            save_model(&name);
            name
        }
        None => usage_error("no model named. Pass --model NAME or set RAK_MODEL"),
    };

    let backend = backend(&o, Some(&model_name)).unwrap_or_else(|why| usage_error(&why));
    let provider = make_provider(&o, &backend, &model_name, &stop);
    let (window, window_source) = window_for(&o, provider.as_ref());

    let notes = std::fs::read_to_string(ctx.cwd.join("AGENTS.md")).ok().map(|n| n.chars().take(8_000).collect::<String>());
    let config =
        Config { system_prompt: system_prompt(&ctx.cwd, o.read_only, notes.as_deref()), max_turns: o.max_turns, context_window: window };
    let mut tools = core_tools(&ctx, o.read_only);
    // Web search, when there is a Tavily key. It only reads, so read-only mode keeps it.
    if let Some(tavily) = key("TAVILY_API_KEY") {
        tools.push(Box::new(WebSearch::new(tavily)));
    }
    let core_tool_names: Vec<String> = tools.iter().map(|t| t.name().to_string()).collect();

    // Tools from MCP servers join the built-in ones.
    let (listed, mut mcp_problems) =
        if o.no_mcp { (Vec::new(), Vec::new()) } else { mcp::load(&mcp::config_files(&ctx.cwd, config_dir().as_deref())) };
    let servers: Vec<mcp::ServerConfig> = listed.into_iter().map(|(server, _)| server).collect();
    let mut connected = mcp::connect_all(&servers, &ctx.cwd, o.read_only, &stop);
    mcp_problems.append(&mut connected.problems);
    let mcp_summary = connected.summary();
    tools.append(&mut connected.tools);

    let mut agent = Agent::new(provider, tools, config);
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
    agent.on_approval(approval::prompter(approval::Prompter {
        ui: ui.clone(),
        auto: auto.clone(),
        stop: stop.clone(),
        server_of: connected.server_of.clone(),
        interactive,
        color: err_color,
    }));

    agent.on_event(output::reporter(log.clone(), ui.clone()));

    log.write(json!({ "event": "start", "version": VERSION, "model": model_name, "base_url": backend.base_url, "provider": if backend.anthropic { "anthropic" } else { "openai" }, "cwd": ctx.cwd,
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
        backend,
        window,
        window_source,
        notes,
        requests: 0,
        core_tools: core_tool_names,
        mcp_servers: connected.servers,
        mcp_summary,
        mcp_problems,
        server_of: connected.server_of,
    };

    if chat {
        chat::run(&mut session, &mut editor);
        return 0;
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
    {
        let mut ui = session.ui.borrow_mut();
        if let Some(summary) = &session.mcp_summary {
            ui.progress(Some(Style::Dim), summary);
        }
        for problem in &session.mcp_problems {
            ui.progress(Some(Style::Yellow), problem);
        }
    }
    exit_code(&run_one(&mut session, &task))
    // Dropping the session stops the MCP servers' programs.
}

/// Runs one request and prints what came of it.
pub fn run_one(s: &mut Session, task: &str) -> Outcome {
    let started = Instant::now();
    let (usage_before, calls_before) = (s.agent.usage, s.agent.tool_calls);
    s.log.write(json!({ "event": "request", "text": task }));
    s.requests += 1;
    s.stop.clear();
    signals::set_running(true);
    let RunResult { outcome, answer, turns } = s.agent.run(task);
    signals::set_running(false);
    let seconds = (started.elapsed().as_secs_f64() * 10.0).round() / 10.0;
    let input = s.agent.usage.input - usage_before.input;
    let output = s.agent.usage.output - usage_before.output;
    let cached = s.agent.usage.cached - usage_before.cached;
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
        "input_tokens": input, "cached_tokens": cached, "output_tokens": output, "seconds": seconds,
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
    ui.progress(Some(Style::Dim), &format!("  {turns} turns · {tool_calls} tool calls · {input} tokens in{} · {output} out · {seconds} s", if cached > 0 { format!(" ({cached} cached)") } else { String::new() }));
    outcome
}

// MARK: - Chat

pub fn exit_code(outcome: &Outcome) -> i32 {
    match outcome {
        Outcome::Done => 0,
        Outcome::Failed(_) => 1,
        Outcome::Refused { .. } => 3,
        Outcome::MaxTurns => 4,
        Outcome::Loop => 5,
        Outcome::Stopped => 130,
    }
}

pub fn outcome_name(outcome: &Outcome) -> &'static str {
    match outcome {
        Outcome::Done => "done",
        Outcome::Failed(_) => "failed",
        Outcome::Refused { .. } => "refused",
        Outcome::MaxTurns => "max_turns",
        Outcome::Loop => "loop",
        Outcome::Stopped => "stopped",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_is_chosen_when_asked_for_or_when_nothing_else_was_given() {
        assert!(uses_anthropic(Some("anthropic"), None, Some("kimi"), false));
        assert!(!uses_anthropic(Some("openai"), None, Some("claude-opus-5-5"), true));
        assert!(uses_anthropic(None, Some("https://api.anthropic.com"), None, false));
        assert!(uses_anthropic(None, None, Some("claude-haiku-5-5"), true));
        assert!(!uses_anthropic(None, None, Some("claude-haiku-5-5"), false), "no key: stays with the default server");
        assert!(!uses_anthropic(None, Some("https://openrouter.ai/api/v1"), Some("claude-opus-5-5"), true), "an address given wins");
        assert!(!uses_anthropic(None, None, Some("kimi-k2.6:cloud"), true));
    }
}
