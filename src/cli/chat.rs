//! The chat: the welcome panel, the prompt loop and the slash commands.

use super::args::Options;
use super::editor::Editor;
use super::picker::{self, Choice, Picked};
use super::session::{backend, make_provider, provider_label, run_one, window_for, Backend, Session};
use super::settings::{home, key, key_files, last_model, remember_model};
use super::style::{cut, pad, paint, Style};
use super::welcome::grouped;
use super::{commands, menu, skills_command, term, welcome, VERSION};
use rak::provider::Model;
use rak::text::one_line;
use serde_json::json;

pub fn mode_label(s: &Session) -> &'static str {
    if s.o.read_only {
        "read-only"
    } else if s.auto.get() {
        "auto mode"
    } else {
        "asks before changes"
    }
}

pub fn banner(s: &Session) {
    let info = welcome::Info {
        version: VERSION.to_string(),
        model: s.model.clone(),
        server: welcome::server_name(&s.backend.base_url),
        folder: welcome::short_path(&s.cwd.to_string_lossy(), &home(), 24),
        branch: welcome::git_branch(&s.cwd),
        mode: mode_label(s).to_string(),
        window: s.window,
        notes: (!s.instruction_files.is_empty())
            .then(|| ("rules".to_string(), s.instruction_files.join(", ")))
            .into_iter()
            .chain(s.has_notes.then(|| ("notes".to_string(), "rak/ (/notes)".to_string())))
            .chain(key_files().iter().map(|f| ("config".to_string(), welcome::short_path(&f.to_string_lossy(), &home(), 26))))
            .chain(
                (s.memory_notes > 0)
                    .then(|| ("memory".to_string(), format!("{} note{}", s.memory_notes, if s.memory_notes == 1 { "" } else { "s" }))),
            )
            .chain({
                let count = s.skills.read().unwrap().len();
                (count > 0).then(|| ("skills".to_string(), format!("{count} (/skills)")))
            })
            .collect(),
        tool_groups: welcome::group(&s.core_tools.iter().map(String::as_str).collect::<Vec<_>>(), &s.mcp_servers),
        tool_count: s.agent.tool_names().len(),
        warnings: s.mcp_problems.clone(),
    };
    for line in welcome::panel(&info, term::size().0, s.ui.borrow().out_color) {
        println!("{line}");
    }
}

pub fn run(s: &mut Session, editor: &mut Editor) {
    banner(s);
    if let Some(warning) = super::session::broad_folder_warning(&s.cwd, &home(), true) {
        println!("{}", paint(s.ui.borrow().out_color, Style::Yellow, &warning));
    }
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
            if !run_command(s, &input) {
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
pub fn list_models(provider: &dyn Model, current: Option<&str>, color: bool) -> Option<Vec<String>> {
    match provider.list_models() {
        Ok(mut models) => {
            // Only models that can chat.
            models.retain(|m| !m.to_lowercase().contains("embed"));
            if models.len() > 60 {
                println!(
                    "{}",
                    paint(color, Style::Dim, &format!("({} models — showing the first 60; type any name to use it)", models.len()))
                );
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

/// Lets the person pick one of the provider's models with the arrow keys, or
/// type a name it doesn't list. None when they cancel (Esc) or the provider
/// can't list its models.
pub fn pick_model(provider: &dyn Model, current: Option<&str>, color: bool) -> Option<String> {
    let mut models = match provider.list_models() {
        Ok(models) => models,
        Err(why) => {
            println!("{}", paint(color, Style::Yellow, &format!("Couldn't list the models: {why}")));
            println!("{}", paint(color, Style::Dim, "You can still switch by name: /model NAME"));
            return None;
        }
    };
    // Only models that can chat.
    models.retain(|m| !m.to_lowercase().contains("embed"));
    if models.is_empty() {
        println!("{}", paint(color, Style::Dim, "The provider lists no models. Switch by name: /model NAME"));
        return None;
    }
    let choices: Vec<Choice> = models.iter().map(|m| Choice { name: m.clone(), note: String::new(), dim: false }).collect();
    let start = current.and_then(|c| models.iter().position(|m| m == c));
    match picker::pick(&choices, start, true, color) {
        Picked::Item(i) => Some(models[i].clone()),
        Picked::Typed(name) => Some(name),
        Picked::Cancelled => None,
    }
}

pub fn switch_model(s: &mut Session, model: &str) {
    let color = s.ui.borrow().out_color;
    // A claude-* model may move the chat to Anthropic's API, and back.
    let backend = match backend(&s.o, Some(model)) {
        Ok(backend) => backend,
        Err(why) => {
            println!("{}", paint(color, Style::Yellow, &format!("Can't switch: {why}")));
            return;
        }
    };
    let moved = backend.base_url != s.backend.base_url;
    let o = s.o.clone();
    use_backend(s, o, backend, model, moved);
}

/// Lets the person pick a provider with the arrow keys; each row says whether
/// its key is set. None when they cancel (Esc).
fn pick_provider(s: &Session, color: bool) -> Option<String> {
    let all = rak::providers::all();
    let choices: Vec<Choice> = all
        .iter()
        .map(|p| {
            let (state, usable) = match &p.key {
                None => ("no key needed".to_string(), true),
                Some(name) if key(name).is_some() => (format!("{name} set"), true),
                Some(name) => (format!("{name} not set"), false),
            };
            Choice { name: p.name.clone(), note: format!("{}{}", pad(&p.label, 42), state), dim: !usable }
        })
        .collect();
    let current = all.iter().position(|p| s.backend.provider.as_deref() == Some(p.name.as_str()));
    match picker::pick(&choices, current, false, color) {
        Picked::Item(i) => Some(all[i].name.clone()),
        _ => None,
    }
}

/// Moves the chat to another provider: its own address and key, and the model
/// last picked there (or RAK_MODEL for the default provider, or one picked now).
pub fn switch_provider(s: &mut Session, name: &str) {
    let color = s.ui.borrow().out_color;
    let warn = |text: &str| println!("{}", paint(color, Style::Yellow, text));
    let Some(provider) = rak::providers::find(name) else {
        warn(&format!("No provider named `{name}`. They are: {}", rak::providers::names()));
        return;
    };
    let mut o: Options = s.o.clone();
    // The default provider keeps the address and model the key file gives it.
    let is_default = key("RAK_PROVIDER").is_some_and(|d| d.eq_ignore_ascii_case(&provider.name));
    o.provider = Some(provider.name.clone());
    o.provider_flag = !is_default;
    o.base_url = if is_default { key("RAK_BASE_URL") } else { None };
    o.base_url_flag = false;
    o.api_key_env = None;
    let backend_now = match backend(&o, None) {
        Ok(b) => b,
        Err(why) => return warn(&format!("Can't switch: {why}")),
    };
    let remembered = last_model(&backend_now.memory_key()).or_else(|| if is_default { key("RAK_MODEL") } else { None });
    let model = match remembered {
        Some(model) => model,
        None => {
            println!("No model picked yet for {}. These are its models:", provider.label);
            match pick_model(make_provider(&o, &backend_now, "", &s.stop).as_ref(), None, color) {
                Some(model) => model,
                None => return println!("Staying with {} at {}.", s.model, provider_label(&s.backend)),
            }
        }
    };
    use_backend(s, o, backend_now, &model, true);
}

/// From here on the chat uses this model at this backend; remembered as the
/// pick for that provider.
fn use_backend(s: &mut Session, o: Options, backend: Backend, model: &str, say_where: bool) {
    let color = s.ui.borrow().out_color;
    let provider = make_provider(&o, &backend, model, &s.stop);
    (s.window, s.window_source) = window_for(&o, provider.as_ref());
    s.agent.set_context_window(s.window);
    s.agent.set_model(provider);
    s.o = o;
    s.backend = backend;
    s.model = model.to_string();
    *s.helper_model.borrow_mut() = (s.backend.clone(), s.model.clone(), s.window);
    remember_model(&s.backend.memory_key(), model);
    s.log.write(json!({ "event": "model", "model": model, "provider": s.backend.provider.as_deref().unwrap_or("custom") }));
    let via = if say_where { format!(" at {}", provider_label(&s.backend)) } else { String::new() };
    println!("Now using {}{via}.", paint(color, Style::Bold, model));
}

/// Runs a slash command. False means leave the chat.
pub fn run_command(s: &mut Session, line: &str) -> bool {
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
        "provider" if !args.is_empty() => switch_provider(s, args),
        "provider" => {
            println!("Provider: {}", paint(color, Style::Bold, &provider_label(&s.backend)));
            match pick_provider(s, color) {
                Some(name) if Some(name.as_str()) == s.backend.provider.as_deref() => println!("{}", dim("Kept it.")),
                Some(name) => switch_provider(s, &name),
                None => println!("{}", dim("Kept it.")),
            }
        }
        "model" | "m" => {
            println!("Model: {} at {}", paint(color, Style::Bold, &s.model), provider_label(&s.backend));
            let provider = make_provider(&s.o, &s.backend, &s.model, &s.stop);
            match pick_model(provider.as_ref(), Some(&s.model), color) {
                Some(model) if model == s.model => println!("{}", dim("Kept it.")),
                Some(model) => switch_model(s, &model),
                None => println!("{}", dim("Kept it.")),
            }
        }
        "models" => {
            let provider = make_provider(&s.o, &s.backend, &s.model, &s.stop);
            if list_models(provider.as_ref(), Some(&s.model), color).is_none() {
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
        "mcp" => {
            if s.mcp_servers.is_empty() && s.mcp_problems.is_empty() {
                println!("{}", dim("No MCP servers. Add one from the shell, then start rak again:"));
                println!("{}", dim("  rak mcp add files -- npx -y @modelcontextprotocol/server-filesystem ."));
                println!("{}", dim("  rak mcp add docs --url https://example.com/mcp"));
            }
            for (server, count) in &s.mcp_servers {
                println!(
                    "  {}  {}",
                    paint(color, Style::Bold, server),
                    dim(&format!("{count} tool{}", if *count == 1 { "" } else { "s" }))
                );
                for tool in s.agent.tools().iter().filter(|t| s.server_of.get(t.name()) == Some(server)) {
                    let gate = if tool.needs_approval() && !s.auto.get() { paint(color, Style::Yellow, " (asks)") } else { String::new() };
                    println!("    {}{gate}  {}", paint(color, Style::Cyan, tool.name()), dim(&cut(&one_line(tool.description(), 200), 70)));
                }
            }
            for problem in &s.mcp_problems {
                println!("  {}", paint(color, Style::Yellow, problem));
            }
        }
        "skills" => match args.split_once(' ').unwrap_or((args, "")) {
            ("add", source) if !source.trim().is_empty() => {
                if s.o.no_skills {
                    println!("{}", dim("Skills are off for this run (--no-skills)."));
                } else {
                    match skills_command::add(source.trim(), &s.skill_folder, false, color) {
                        Ok(Some(skill)) => {
                            println!("{}", dim(&format!("The agent can load \"{}\" now.", skill.name)));
                            rak::skills::save::share(&s.skills, skill);
                        }
                        Ok(None) => {}
                        Err(why) => println!("{}", paint(color, Style::Yellow, &why)),
                    }
                }
            }
            ("add", _) => println!("{}", dim("usage: /skills add URL-or-FILE")),
            _ => skills_command::print_list(&s.skills.read().unwrap(), color),
        },
        "notes" => {
            let snapshot = rak::notes::read(&s.cwd);
            if let Some(rules) = snapshot.instructions_text() {
                println!("{}", rules.trim());
            }
            if let Some(project) = &snapshot.project {
                println!("\n### rak/project.md\n{}", project.trim());
            }
            if let Some(roadmap) = &snapshot.roadmap {
                println!("\n### rak/roadmap.md (without Done)\n{}", rak::notes::working_sections(roadmap, 5_000));
            }
            if let Some(journal) = &snapshot.journal {
                let lines: Vec<&str> = journal.lines().filter(|l| l.starts_with("- ")).collect();
                println!("\n### rak/journal.md (latest)\n{}", lines[lines.len().saturating_sub(5)..].join("\n"));
            }
            if snapshot.instructions.is_empty() && !snapshot.exists() {
                println!(
                    "{}",
                    dim("No project rules or notes here. Rules: rak/rules.md, AGENTS.md or CLAUDE.md. Notes: ask rak to set up rak/project.md and rak/roadmap.md.")
                );
            }
        }
        "yes" | "auto" => {
            s.auto.set(!s.auto.get());
            if s.auto.get() {
                println!("{}", paint(color, Style::Yellow, "Auto-approve ON: edits and shell commands run without asking."));
            } else {
                println!("Auto-approve off: rak asks before edits and shell commands.");
            }
        }
        "usage" | "cost" => {
            println!("  model          {} ({})", s.model, welcome::server_name(&s.backend.base_url));
            println!("  requests       {}", s.requests);
            let cached = s.agent.usage.cached;
            let from_cache = if cached > 0 { format!(" ({} from cache)", grouped(cached)) } else { String::new() };
            println!("  input tokens   {}{from_cache}", grouped(s.agent.usage.input));
            println!("  output tokens  {}", grouped(s.agent.usage.output));
            println!("  tool calls     {}", s.agent.tool_calls);
            println!("  context        ~{} tokens in the last request", grouped(s.agent.last_prompt_tokens));
            println!("  window         {} tokens ({})", grouped(s.window as u64), s.window_source);
            if !s.agent.switches.is_empty() {
                println!("  fallbacks      {} this chat, last to {}", s.agent.switches.len(), s.agent.switches.last().unwrap());
            }
        }
        "clear" | "reset" | "new" => {
            s.agent.clear();
            s.chat_id = rak::sessions::new_id();
            println!("{}", dim("(new conversation — the last one is saved: /resume)"));
        }
        "resume" | "load" => resume(s, args.trim(), color),
        "exit" | "quit" | "q" => return false,
        other => println!("{}", paint(color, Style::Yellow, &format!("Unknown command /{other}. Type /help."))),
    }
    true
}

/// `/resume`: the folder's saved chats; `/resume N` picks one up.
fn resume(s: &mut Session, args: &str, color: bool) {
    let dim = |t: &str| paint(color, Style::Dim, t);
    let Some(dir) = s.sessions_dir.clone() else {
        println!("{}", dim("No config folder, so no saved chats."));
        return;
    };
    let saved: Vec<_> = rak::sessions::list(&dir, &s.project_key)
        .into_iter()
        .filter(|c| c.path.file_stem().is_some_and(|id| id.to_string_lossy() != s.chat_id))
        .collect();
    if args.is_empty() {
        if saved.is_empty() {
            println!("{}", dim("No other saved chats in this folder. Chats are saved after every request."));
            return;
        }
        let now = rak::sessions::now_seconds();
        for (n, chat) in saved.iter().take(15).enumerate() {
            let when = format!("{}, {} messages", rak::sessions::age(chat.saved_at, now), chat.messages);
            println!("  {}  {}  {}", paint(color, Style::Cyan, &format!("{:>2}", n + 1)), cut(&chat.title, 70), dim(&when));
        }
        println!("{}", dim("Pick one: /resume N. The current chat stays saved."));
        return;
    }
    let Some(chat) = args.parse::<usize>().ok().and_then(|n| n.checked_sub(1)).and_then(|i| saved.get(i)) else {
        println!("{}", paint(color, Style::Yellow, "usage: /resume N, with N from the list /resume shows"));
        return;
    };
    s.save_chat();
    match s.resume(&chat.path) {
        Ok(count) => println!("Resumed {} ({count} messages).", paint(color, Style::Bold, &chat.title)),
        Err(why) => println!("{}", paint(color, Style::Yellow, &why)),
    }
}
