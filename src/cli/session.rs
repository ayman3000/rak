//! One run of rak: what it holds, how it is set up from the options, and
//! one request run and reported.

use super::args::{usage_error, Options, VERSION};
use super::editor::Editor;
use super::log::Log;
use super::output::{self, Ui};
use super::settings::{config_dir, key, last_model, load_keys, remember_model};
use super::style::{paint, Style};
use super::{approval, chain, chat, commands, markdown, menu, signals};
use rak::agent::{system_prompt, Agent, Config, Outcome, RunResult};
use rak::anthropic::Anthropic;
use rak::interrupt::Stop;
use rak::mcp;
use rak::provider::{Model, OpenAICompatible};
use rak::providers::{self, Provider};
use rak::tools::web::WebSearch;
use rak::tools::{core_tools, Ctx};
use serde_json::json;
use std::cell::{Cell, RefCell};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
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
    /// Notes from earlier sessions put in this session's prompt.
    pub memory_notes: usize,
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
    /// Where the fallback chain stands, and what its hook needs.
    pub chain: Rc<RefCell<chain::State>>,
    pub asker: chain::Asker,
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
    /// The provider's name ("openrouter"), or None for a server named only by
    /// its address.
    pub provider: Option<String>,
    pub anthropic: bool,
    pub base_url: String,
    pub api_key: Option<String>,
}

impl Backend {
    /// Where the last model picked here is remembered: the provider's name,
    /// or the address of a server rak doesn't know by name.
    pub fn memory_key(&self) -> String {
        self.provider.clone().unwrap_or_else(|| self.base_url.clone())
    }
}

/// Pure — unit-tested. The model to start with:
/// 1. `-m` on the command line;
/// 2. in a chat, the model last picked for this provider;
/// 3. RAK_MODEL, when this is the default provider it was set for (not one
///    chosen with --provider or --base-url);
/// 4. outside a chat, the model last picked for this provider.
///
/// None: in a chat, list the provider's models to pick from.
pub fn choose_model(o: &Options, chat: bool, last_pick: Option<String>) -> Option<String> {
    let flag = o.model.clone().filter(|_| o.model_flag);
    let default = o.model.clone().filter(|_| !o.model_flag && !o.provider_flag && !o.base_url_flag);
    if chat {
        flag.or(last_pick).or(default)
    } else {
        flag.or(default).or(last_pick)
    }
}

/// Where the last pick is remembered for these options, before a model is
/// chosen (a claude-* model can move a run to Anthropic, so the candidate counts).
fn memory_key_for(o: &Options, candidate: Option<&str>) -> Option<String> {
    let route = route(o, candidate, key("ANTHROPIC_API_KEY").is_some()).ok()?;
    Some(route.provider.map(|p| p.name.clone()).unwrap_or(route.base_url))
}

/// Which variable the key comes from.
#[derive(Debug, PartialEq)]
pub enum KeySource {
    /// No key: a server on this machine.
    None,
    /// This variable, and it must be set; the text says who wants it.
    Required(String, String),
    /// This variable when it is set, otherwise no key.
    IfSet(&'static str),
}

/// Where a run goes, before any key is read.
#[derive(Debug, PartialEq)]
pub struct Route {
    pub provider: Option<&'static Provider>,
    pub base_url: String,
    pub key: KeySource,
}

/// Pure — unit-tested. Where a run goes and which key it sends.
///
/// - `--provider NAME`: that provider's own address and its own key.
/// - Nothing named: Ollama on this machine, or Anthropic for a claude-*
///   model when ANTHROPIC_API_KEY is set.
/// - An address alone (`--base-url`): that server, with RAK_API_KEY if set.
/// - `--api-key-env NAME` always picks the key.
///
/// Defaults from the .env files never mix with flags: `--provider` on the
/// command line ignores RAK_BASE_URL, and `--base-url` ignores RAK_PROVIDER.
/// So one provider's key never goes to another's server.
pub fn route(o: &Options, model: Option<&str>, has_anthropic_key: bool) -> Result<Route, String> {
    let (provider, base_url) = match (o.provider_flag, o.base_url_flag) {
        (true, false) => (o.provider.as_deref(), None),
        (false, true) => (None, o.base_url.as_deref()),
        _ => (o.provider.as_deref(), o.base_url.as_deref()),
    };
    let named = match provider {
        Some(name) => Some(providers::find(name).ok_or(format!("unknown provider `{name}`. Known: {}", providers::names()))?),
        None => match base_url {
            Some(url) if url.contains("anthropic.com") => providers::find("anthropic"),
            Some(_) => None,
            None if has_anthropic_key && model.is_some_and(|m| m.starts_with("claude-")) => providers::find("anthropic"),
            None => providers::find("ollama"),
        },
    };
    let key = match (o.api_key_env.as_deref(), named) {
        (Some(name), _) => KeySource::Required(name.to_string(), "named with --api-key-env".to_string()),
        (None, Some(p)) => match &p.key {
            Some(name) => KeySource::Required(name.clone(), format!("{} needs it", p.label)),
            None => KeySource::None,
        },
        (None, None) => KeySource::IfSet("RAK_API_KEY"),
    };
    let base_url = base_url.map(str::to_string).or_else(|| named.map(|p| p.base_url.clone())).unwrap_or_default();
    Ok(Route { provider: named, base_url, key })
}

/// For people: "OpenRouter", or the address of a server rak doesn't know by name.
pub fn provider_label(backend: &Backend) -> String {
    match backend.provider.as_deref().and_then(providers::find) {
        Some(p) => p.label.clone(),
        None => backend.base_url.clone(),
    }
}

/// The backend for the options and model, or why there can't be one.
pub fn backend(o: &Options, model: Option<&str>) -> Result<Backend, String> {
    let route = route(o, model, key("ANTHROPIC_API_KEY").is_some())?;
    let api_key = match &route.key {
        KeySource::None => None,
        KeySource::IfSet(name) => key(name),
        KeySource::Required(name, why) => {
            Some(key(name).ok_or(format!("{name} is not set ({why}). Add it to ~/.config/rak/.env; `rak keys` shows which keys are set"))?)
        }
    };
    Ok(Backend {
        provider: route.provider.map(|p| p.name.clone()),
        anthropic: route.provider.is_some_and(|p| p.anthropic),
        base_url: route.base_url,
        api_key,
    })
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
    // Asking the server is a request: only when --context didn't say.
    match o.context {
        Some(n) => (n, "set with --context"),
        None => match provider.context_window() {
            Some(n) => (n, "reported by the server"),
            None => (128_000, "assumed — the server didn't say; set it with --context"),
        },
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

    let stdin_tty = std::io::stdin().is_terminal();
    let stdout_tty = std::io::stdout().is_terminal();
    let chat = o.task.is_none() && stdin_tty && !o.json;
    let no_color = std::env::var_os("NO_COLOR").is_some();
    let out_color = stdout_tty && !no_color;
    let err_color = std::io::stderr().is_terminal() && !no_color;

    // One stop flag for the loop, the model stream and a running command.
    let ctx = Ctx::new(&cwd, o.any_path);
    output::set_project(&ctx.cwd);
    let stop = ctx.stop.clone();
    #[cfg(unix)]
    signals::install(stop.clone());

    let mut editor = Editor::new(config_dir().map(|d| d.join("history")), menu::commands(commands::HELP), out_color);

    // The model: named with -m, or the last one picked for this provider, or
    // RAK_MODEL, or (in a chat) picked now from the provider's list.
    let candidate = o.model.clone().filter(|_| o.model_flag || (!o.provider_flag && !o.base_url_flag));
    let memory = memory_key_for(&o, candidate.as_deref());
    let model_name = match choose_model(&o, chat, memory.as_deref().and_then(last_model)) {
        Some(name) => name,
        None if chat => {
            let backend = backend(&o, None).unwrap_or_else(|why| usage_error(&why));
            println!("No model picked yet for {}. These are its models:", provider_label(&backend));
            let picked = chat::pick_model(make_provider(&o, &backend, "", &stop).as_ref(), None, out_color);
            let Some(name) = picked else { usage_error("no model named. Pass --model NAME or set RAK_MODEL") };
            remember_model(&backend.memory_key(), &name);
            name
        }
        None => usage_error("no model named. Pass --model NAME or set RAK_MODEL"),
    };

    let backend = backend(&o, Some(&model_name)).unwrap_or_else(|why| usage_error(&why));
    let provider = make_provider(&o, &backend, &model_name, &stop);
    // Developer runs (from SwiftAgentKit's replay harness): play a recording
    // instead of a model, or record this one's replies.
    let provider: Box<dyn rak::provider::Model> = match (&o.replay, &o.record) {
        (Some(file), _) => match rak::replay::Replay::from_file(file) {
            Ok(replay) => Box::new(replay),
            Err(why) => usage_error(&why),
        },
        (None, Some(file)) => Box::new(rak::replay::Recording::new(provider, file)),
        (None, None) => provider,
    };
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
    // Notes from earlier sessions, read once: the prompt stays the same all session.
    let (memory_file, user_memory_file) = rak::memory::files(&ctx.cwd, config_dir().as_deref());
    let memory_notes = if o.no_memory {
        0
    } else {
        let (project_notes, user_notes) = rak::memory::load(&memory_file, user_memory_file.as_deref());
        config.system_prompt.push_str(&rak::memory::prompt_block(&project_notes, &user_notes));
        project_notes.len() + user_notes.len()
    };
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
        asked: false,
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
    // Routines: saved steps that run with no model between them. Saving one
    // changes what later sessions can do, so read-only leaves them out.
    let routine_folders = rak::routines::folders(&ctx.cwd, config_dir().as_deref());
    if !o.read_only {
        tools.push(Box::new(rak::routines::SaveRoutine { project: routine_folders.0.clone(), user: routine_folders.1.clone() }));
    }
    // Keeping a note changes what later sessions do, so read-only leaves it out.
    if !o.no_memory && !o.read_only {
        tools.push(Box::new(rak::memory::Remember { project_file: memory_file.clone(), user_file: user_memory_file.clone() }));
    }
    // view_image, unless the server says the model can't see.
    if provider.sees_images() != Some(false) {
        tools.push(Box::new(rak::tools::image::ViewImage(ctx.clone())));
    }
    // delegate: a helper agent with read-only tools and a fresh context, on
    // the chat's current model (`helper_model`, kept up to date by /model).
    let helper_model = Rc::new(RefCell::new((backend.clone(), model_name.clone(), window)));
    // Approvals and fallbacks: -y (or /yes) runs everything; otherwise ask at
    // the terminal, and with no terminal to ask, approvals stop.
    let auto = Rc::new(Cell::new(o.yes));
    let interactive = stdin_tty && std::io::stderr().is_terminal();
    let asker = chain::Asker { o: o.clone(), stop: stop.clone(), ui: ui.clone(), auto: auto.clone(), interactive, color: err_color };
    let delegate = {
        let (current, o, stop, ui, ctx) = (helper_model.clone(), o.clone(), stop.clone(), ui.clone(), ctx.clone());
        let asker = asker.clone();
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
            // A helper falls back along the same chain, on its own.
            let links = fallback_chain(&backend);
            if !links.is_empty() {
                let state = chain::State { links, current_label: chain::label(&backend, &model), ..Default::default() };
                helper.fallback = Some(chain::hook(asker.clone(), Rc::new(RefCell::new(state))));
            }
            let ui = ui.clone();
            helper.on_event(move |event| match event {
                rak::agent::Event::ToolStarted { name, args } => {
                    ui.borrow_mut().progress(Some(Style::Dim), &format!("    ↳ {}", output::describe(name, args)));
                }
                rak::agent::Event::FellBack { to, why, failure } => {
                    let mut ui = ui.borrow_mut();
                    let asked = std::mem::take(&mut ui.asked);
                    for (i, line) in output::fallback_note(to, why, *failure, asked).iter().enumerate() {
                        let (style, mark) = if i == 0 { (Style::Yellow, "    ↪ ") } else { (Style::Dim, "      ") };
                        ui.progress(Some(style), &output::fit(&format!("{mark}{line}")));
                    }
                }
                _ => {}
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
    // Settings set twice in a .env file: shown with the MCP warnings.
    mcp_problems.extend(super::settings::key_notes());
    let mcp_summary = connected.summary();
    let deferred = connected.to_defer();
    tools.append(&mut connected.tools);

    let learns = tools.iter().any(|t| t.name() == "save_skill");
    let mut agent = Agent::new(provider, tools, config);
    agent.defer(deferred);
    if !o.read_only {
        agent.enable_routines(std::iter::once(routine_folders.0.clone()).chain(routine_folders.1.clone()).collect());
    }
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
    agent.on_approval(approval::prompter(approval::Prompter {
        ui: ui.clone(),
        auto: auto.clone(),
        stop: stop.clone(),
        server_of: connected.server_of.clone(),
        interactive,
        color: err_color,
    }));

    agent.on_event(output::reporter(log.clone(), ui.clone()));

    log.write(json!({ "event": "start", "version": VERSION, "model": model_name, "base_url": backend.base_url, "provider": backend.provider.as_deref().unwrap_or("custom"), "cwd": ctx.cwd,
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
        memory_notes,
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
        chain: Rc::new(RefCell::new(chain::State::default())),
        asker,
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
        if let Some(warning) = broad_folder_warning(&session.cwd, &super::settings::home(), false) {
            ui.progress(Some(Style::Yellow), &warning);
        }
    }
    exit_code(&run_one(&mut session, &task))
    // Dropping the session stops the MCP servers' programs.
}

/// Pure — unit-tested. A warning when rak was started in a folder far wider
/// than a project — the home folder or the top of the disk — where its tools
/// can reach everything the person owns.
pub fn broad_folder_warning(cwd: &Path, home: &str, chat: bool) -> Option<String> {
    let same = |a: &Path, b: &Path| a == b || matches!((std::fs::canonicalize(a), std::fs::canonicalize(b)), (Ok(x), Ok(y)) if x == y);
    let what = if !home.is_empty() && same(cwd, Path::new(home)) {
        "your home folder"
    } else if cwd.parent().is_none() {
        "the top of the disk"
    } else {
        return None;
    };
    let instead = if chat {
        "For a project, quit (Ctrl-D) and start rak inside that project's folder."
    } else {
        "For a project, run rak inside that project's folder, or name it with -C."
    };
    Some(format!("⚠ rak started in {what}: its tools can reach every file in it. {instead}"))
}

/// Runs one request and prints what came of it.
/// The providers to fall back to from this backend (see chain.rs).
pub fn fallback_chain(from: &Backend) -> Vec<chain::Link> {
    let order = super::settings::key_order();
    chain::chain(providers::all(), from.provider.as_deref(), &order, &|k| key(k).is_some(), &|p| last_model(p))
}

/// Before a request: back on the chosen provider if the last request fell
/// back for a passing reason, and the chain built afresh from it.
fn prepare_fallback(s: &mut Session) {
    let state = s.chain.clone();
    let mut st = state.borrow_mut();
    if st.current.take().is_some() {
        s.agent.set_model(make_provider(&s.o, &s.backend, &s.model, &s.stop));
        s.agent.set_context_window(s.window);
        s.ui.borrow_mut().progress(Some(Style::Dim), &format!("(back on {})", chain::label(&s.backend, &s.model)));
    }
    st.links = fallback_chain(&s.backend);
    st.next = 0;
    st.for_good = false;
    st.current_label = chain::label(&s.backend, &s.model);
    let mut asker = s.asker.clone();
    asker.o = s.o.clone();
    s.agent.fallback = (!st.links.is_empty()).then(|| chain::hook(asker, state.clone()));
}

/// After a request: a fallback made because the chosen provider can't work
/// (a bad key, a missing model) becomes the chat's provider and model.
fn settle_fallback(s: &mut Session) {
    let state = s.chain.clone();
    let mut st = state.borrow_mut();
    if !st.for_good {
        return;
    }
    let Some((o, backend, model, window)) = st.current.take() else { return };
    let chosen = chain::label(&s.backend, &s.model);
    (s.o, s.backend, s.model, s.window) = (o, backend, model, window);
    s.window_source = "reported by the server";
    *s.helper_model.borrow_mut() = (s.backend.clone(), s.model.clone(), s.window);
    st.for_good = false;
    let mut ui = s.ui.borrow_mut();
    let note = if ui.chat {
        format!("Staying with {} for this chat: {chosen} needs its key or model fixed (rak keys).", st.current_label)
    } else {
        format!("{chosen} needs its key or model fixed (rak keys).")
    };
    ui.progress(Some(Style::Yellow), &note);
}

pub fn run_one(s: &mut Session, task: &str) -> Outcome {
    let started = Instant::now();
    let (usage_before, calls_before) = (s.agent.usage, s.agent.tool_calls);
    s.log.write(json!({ "event": "request", "text": task }));
    s.requests += 1;
    s.stop.clear();
    s.todo.begin_request();
    prepare_fallback(s);
    let switches_before = s.agent.switches.len();
    signals::set_running(true);
    let RunResult { outcome, answer, turns } = s.agent.run(task);
    let fell_back_to: Vec<String> = s.agent.switches[switches_before..].to_vec();
    settle_fallback(s);
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
        "fell_back_to": fell_back_to,
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
            let text = if problem.ends_with(rak::fallback::DECLINED) {
                // The question just above showed the error.
                rak::fallback::DECLINED.to_string()
            } else if failed {
                format!("error: {problem}")
            } else {
                format!("({problem})")
            };
            println!("{}", paint(ui.out_color, if failed { Style::Red } else { Style::Yellow }, &text));
        } else {
            eprintln!("rak: {problem}");
        }
    }
    ui.progress(
        Some(Style::Dim),
        &format!(
            "  {} · {} · {input} tokens in{} · {output} out · {seconds} s",
            output::plural(turns, "turn"),
            output::plural(tool_calls, "tool call"),
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
    #[test]
    fn a_home_or_disk_top_start_is_warned_about() {
        let home = std::env::temp_dir();
        let warning = super::broad_folder_warning(&home, &home.to_string_lossy(), true).unwrap();
        assert!(warning.contains("your home folder") && warning.contains("Ctrl-D"), "{warning}");
        let project = home.join("project");
        assert_eq!(super::broad_folder_warning(&project, &home.to_string_lossy(), true), None);
        assert!(super::broad_folder_warning(std::path::Path::new("/"), "", false).unwrap().contains("-C"));
        assert_eq!(super::broad_folder_warning(&project, "", false), None, "no home known");
    }

    use super::*;

    fn options(provider: Option<&str>, provider_flag: bool, base_url: Option<&str>, base_url_flag: bool) -> Options {
        let mut o = super::super::args::parse_args(Vec::new());
        (o.provider, o.provider_flag) = (provider.map(str::to_string), provider_flag);
        (o.base_url, o.base_url_flag) = (base_url.map(str::to_string), base_url_flag);
        o
    }

    fn required(name: &str) -> KeySource {
        let label = providers::all().iter().find(|p| p.key.as_deref() == Some(name)).map(|p| p.label.clone()).unwrap();
        KeySource::Required(name.into(), format!("{label} needs it"))
    }

    #[test]
    fn the_starting_model_follows_the_flag_then_the_last_pick_then_rak_model() {
        let with = |model: Option<&str>, model_flag: bool, provider_flag: bool| {
            let mut o = options(Some("openrouter"), provider_flag, None, false);
            (o.model, o.model_flag) = (model.map(str::to_string), model_flag);
            o
        };
        let last = || Some("last/pick".to_string());
        // -m wins everywhere.
        assert_eq!(choose_model(&with(Some("flag/m"), true, false), true, last()).as_deref(), Some("flag/m"));
        // In a chat the last pick beats RAK_MODEL; in a one-off run RAK_MODEL wins.
        assert_eq!(choose_model(&with(Some("env/m"), false, false), true, last()).as_deref(), Some("last/pick"));
        assert_eq!(choose_model(&with(Some("env/m"), false, false), false, last()).as_deref(), Some("env/m"));
        // RAK_MODEL belongs to the default provider, not one chosen with --provider.
        assert_eq!(choose_model(&with(Some("env/m"), false, true), true, None), None);
        assert_eq!(choose_model(&with(Some("env/m"), false, true), false, last()).as_deref(), Some("last/pick"));
        // Nothing at all: a chat lists the models.
        assert_eq!(choose_model(&with(None, false, false), true, None), None);
    }

    #[test]
    fn a_named_provider_gets_its_own_address_and_key() {
        let r = route(&options(Some("openrouter"), true, None, false), Some("x/y"), true).unwrap();
        assert_eq!((r.base_url.as_str(), r.key), ("https://openrouter.ai/api/v1", required("OPENROUTER_API_KEY")));
        let r = route(&options(Some("ollama-cloud"), false, None, false), None, false).unwrap();
        assert_eq!((r.base_url.as_str(), r.key), ("https://ollama.com/v1", required("OLLAMA_API_KEY")));
        let r = route(&options(Some("lmstudio"), true, None, false), None, false).unwrap();
        assert_eq!(r.key, KeySource::None, "a server on this machine takes no key");
        assert!(route(&options(Some("nope"), true, None, false), None, false).unwrap_err().contains("Known: ollama"));
    }

    #[test]
    fn with_nothing_named_rak_uses_ollama_or_anthropic_for_claude() {
        let r = route(&options(None, false, None, false), Some("kimi-k2.6:cloud"), true).unwrap();
        assert_eq!((r.provider.unwrap().name.as_str(), r.key), ("ollama", KeySource::None));
        let r = route(&options(None, false, None, false), Some("claude-haiku-5-5"), true).unwrap();
        assert!(r.provider.unwrap().anthropic);
        let r = route(&options(None, false, None, false), Some("claude-haiku-5-5"), false).unwrap();
        assert_eq!(r.provider.unwrap().name, "ollama", "no Anthropic key: stays local");
    }

    #[test]
    fn defaults_from_the_file_never_mix_with_flags() {
        // The file says OpenRouter; --provider groq on the command line: Groq's own address.
        let o = options(Some("groq"), true, Some("https://openrouter.ai/api/v1"), false);
        let r = route(&o, None, false).unwrap();
        assert_eq!((r.base_url.as_str(), r.key), ("https://api.groq.com/openai/v1", required("GROQ_API_KEY")));
        // The file says OpenRouter; --base-url names another server: no OpenRouter key goes there.
        let o = options(Some("openrouter"), false, Some("https://example.com/v1"), true);
        let r = route(&o, None, false).unwrap();
        assert_eq!((r.provider, r.key), (None, KeySource::IfSet("RAK_API_KEY")));
        // Both from the file: they go together.
        let o = options(Some("ollama"), false, Some("http://gpu-box:11434/v1"), false);
        assert_eq!(route(&o, None, false).unwrap().base_url, "http://gpu-box:11434/v1");
        // --api-key-env always wins.
        let mut o = options(None, false, Some("https://example.com/v1"), true);
        o.api_key_env = Some("MY_KEY".into());
        assert_eq!(route(&o, None, false).unwrap().key, KeySource::Required("MY_KEY".into(), "named with --api-key-env".into()));
        // Anthropic's address alone means Anthropic.
        let o = options(None, false, Some("https://api.anthropic.com"), true);
        assert_eq!(route(&o, None, false).unwrap().key, required("ANTHROPIC_API_KEY"));
    }
}
