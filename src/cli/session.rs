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
use rak::anthropic::Anthropic;
use rak::interrupt::Stop;
use rak::mcp;
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
    pub skills: rak::skills::Shared,
    /// Where `/skills add` saves: the user's skills folder, else the project's.
    pub skill_folder: PathBuf,
    pub requests: usize,
    /// The built-in tools' names, and what the MCP servers brought.
    pub core_tools: Vec<String>,
    pub mcp_servers: Vec<(String, usize)>,
    pub mcp_summary: Option<String>,
    pub mcp_problems: Vec<String>,
    pub server_of: std::collections::HashMap<String, String>,
    /// Where chats are saved (`~/.config/rak/sessions`), this project's folder
    /// in it, and this chat's id.
    pub sessions_dir: Option<PathBuf>,
    pub project_key: String,
    pub chat_id: String,
    pub todo: rak::tools::todo::TodoList,
    /// The model delegate's helpers use, and what they spent since it was last counted.
    pub helper_model: Rc<RefCell<(Backend, String, usize)>>,
    pub helper_usage: Rc<Cell<rak::provider::Usage>>,
}

impl Session {
    /// Saves the chat; a no-op without a config folder or before the first request.
    pub fn save_chat(&self) {
        let Some(dir) = &self.sessions_dir else { return };
        if self.agent.history().is_empty() {
            return;
        }
        if let Err(why) = rak::sessions::save(dir, &self.project_key, &self.chat_id, self.agent.history(), &self.model) {
            self.ui.borrow_mut().progress(Some(Style::Yellow), &format!("chat not saved: {why}"));
        }
    }

    /// Picks up a saved chat; it is saved under its own id from then on.
    pub fn resume(&mut self, path: &std::path::Path) -> Result<usize, String> {
        let history = rak::sessions::load(path)?;
        let count = history.len();
        self.agent.restore(history);
        self.chat_id = rak::sessions::id_of(path);
        Ok(count)
    }
}

/// Added to the system prompt with code_map, delegate and todo.
const BIG_PROJECTS: &str = "\n\nIn a codebase you don't know yet, start with code_map, then read only the parts you need \
     (read_file with start_line and end_line). Give a question that means reading many files to delegate, so your own context \
     keeps room for the work. For work with three or more steps, keep a todo list.";

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
    // Skills: the project's own (.rak/skills) win over the user's (~/.config/rak/skills).
    let skill_folders = rak::skills::folders(&ctx.cwd, config_dir().as_deref());
    // Plus the ones built into rak for this system; --no-skills leaves all of them out.
    let loaded = if o.no_skills { Vec::new() } else { rak::skills::load_with_built_in(&skill_folders, std::env::consts::OS) };
    let mut config = config;
    // The index is fixed for the session, so the prompt stays cacheable; a skill
    // saved now is loadable at once and listed from the next session.
    config.system_prompt.push_str(&rak::skills::index(&loaded));
    let skills: rak::skills::Shared = std::sync::Arc::new(std::sync::RwLock::new(loaded));
    let log = Log::open(o.log.as_ref());
    let live = stdout_tty && !o.json && !o.no_stream;
    let ui = Rc::new(RefCell::new(Ui {
        chat,
        show: !o.quiet && !o.json,
        out_color,
        err_color,
        live,
        renderer: (out_color && !o.json).then(markdown::Renderer::default),
        line_open: false,
    }));

    let mut tools = core_tools(&ctx, o.read_only);
    // Web search, when there is a Tavily key. It only reads, so read-only mode keeps it.
    if let Some(tavily) = key("TAVILY_API_KEY") {
        tools.push(Box::new(WebSearch::new(tavily)));
    }
    if !o.no_skills {
        tools.push(Box::new(rak::skills::UseSkill::new(skills.clone())));
        // Writing skills changes what later sessions do, so read-only leaves it out.
        if !o.read_only {
            config.system_prompt.push_str(rak::skills::save::LEARNING);
            let user = skill_folders.get(1).cloned();
            tools.push(Box::new(rak::skills::save::SaveSkill::new(skills.clone(), skill_folders[0].clone(), user)));
        }
    }
    // view_image, unless the server says the model can't see.
    if provider.sees_images() != Some(false) {
        tools.push(Box::new(rak::tools::image::ViewImage(ctx.clone())));
    }
    // delegate: a helper agent with read-only tools and a fresh context, on
    // the chat's current model (`helper_model`, kept up to date by /model).
    let helper_model = Rc::new(RefCell::new((backend.clone(), model_name.clone(), window)));
    let delegate = {
        let (current, o, stop, ui, ctx) = (helper_model.clone(), o.clone(), stop.clone(), ui.clone(), ctx.clone());
        let (notes, tavily) = (notes.clone(), key("TAVILY_API_KEY"));
        rak::delegate::Delegate::new(Box::new(move || {
            let (backend, model, window) = current.borrow().clone();
            let mut tools = core_tools(&ctx, true);
            if let Some(tavily) = &tavily {
                tools.push(Box::new(WebSearch::new(tavily.clone())));
            }
            let mut system_prompt = system_prompt(&ctx.cwd, true, notes.as_deref());
            system_prompt.push_str(rak::delegate::HELPER_NOTE);
            let config = Config { system_prompt, max_turns: 25, context_window: window };
            let mut helper = Agent::new(make_provider(&o, &backend, &model, &stop), tools, config);
            helper.stop = stop.clone();
            let ui = ui.clone();
            helper.on_event(move |event| {
                if let rak::agent::Event::ToolStarted { name, args } = event {
                    ui.borrow_mut().progress(Some(Style::Dim), &format!("    ↳ {}", output::describe(name, args)));
                }
            });
            helper
        }))
    };
    let helper_usage = delegate.usage.clone();
    tools.push(Box::new(delegate));
    // When to use code_map, delegate and todo: without this line, models
    // searched and read whole files of a 45-file project instead.
    config.system_prompt.push_str(BIG_PROJECTS);
    // The task list, for work with several steps.
    let todo = rak::tools::todo::TodoList::default();
    tools.push(Box::new(rak::tools::todo::Todo(todo.clone())));
    let core_tool_names: Vec<String> = tools.iter().map(|t| t.name().to_string()).collect();

    // Tools from MCP servers join the built-in ones.
    let (listed, mut mcp_problems) =
        if o.no_mcp { (Vec::new(), Vec::new()) } else { mcp::load(&mcp::config_files(&ctx.cwd, config_dir().as_deref())) };
    let servers: Vec<mcp::ServerConfig> = listed.into_iter().map(|(server, _)| server).collect();
    let mut connected = mcp::connect_all(&servers, &ctx.cwd, o.read_only, &stop);
    mcp_problems.append(&mut connected.problems);
    let mcp_summary = connected.summary();
    tools.append(&mut connected.tools);

    let learns = tools.iter().any(|t| t.name() == "save_skill");
    let mut agent = Agent::new(provider, tools, config);
    agent.stop = stop.clone();
    if learns {
        agent.refusal_note.push_str(rak::skills::save::AFTER_REFUSAL);
    }
    let open_steps = todo.clone();
    agent.finish_check = Some(Box::new(move || open_steps.open_note()));

    agent.stream = live;

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
        skill_folder: skill_folders.last().cloned().unwrap_or_default(),
        skills,
        requests: 0,
        core_tools: core_tool_names,
        mcp_servers: connected.servers,
        mcp_summary,
        mcp_problems,
        server_of: connected.server_of,
        sessions_dir: config_dir().map(|d| d.join("sessions")),
        project_key: rak::sessions::project_key(&ctx.cwd),
        chat_id: rak::sessions::new_id(),
        todo,
        helper_model,
        helper_usage,
    };

    if session.o.continue_last {
        let last = session.sessions_dir.as_ref().and_then(|dir| rak::sessions::list(dir, &session.project_key).into_iter().next());
        let note = match last {
            Some(saved) => match session.resume(&saved.path) {
                Ok(_) => format!("(continuing this folder's last chat: {})", saved.title),
                Err(why) => format!("(could not continue the last chat: {why})"),
            },
            None => "(no earlier chat in this folder — starting fresh)".to_string(),
        };
        session.ui.borrow_mut().progress(Some(Style::Dim), &note);
    }

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
        // usage_error exits without running destructors: stop the MCP
        // servers' programs first, or one that ignores its closed input
        // outlives rak.
        drop(session);
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
    s.todo.begin_request();
    signals::set_running(true);
    let RunResult { outcome, answer, turns } = s.agent.run(task);
    // What delegate's helpers spent counts as this request's too.
    let helpers = s.helper_usage.take();
    s.agent.usage.add(helpers);
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
    s.save_chat();

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
    ui.progress(
        Some(Style::Dim),
        &format!(
            "  {turns} turns · {tool_calls} tool calls · {input} tokens in{} · {output} out · {seconds} s",
            if cached > 0 { format!(" ({cached} cached)") } else { String::new() }
        ),
    );
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
