//! The chat: the welcome panel, the prompt loop and the slash commands.

use super::editor::Editor;
use super::session::{backend, make_provider, run_one, window_for, Session};
use super::settings::{home, key_files, save_model};
use super::style::{cut, pad, paint, Style};
use super::welcome::grouped;
use super::{commands, menu, term, welcome, VERSION};
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
        notes: s
            .notes
            .iter()
            .map(|_| ("notes".to_string(), "AGENTS.md".to_string()))
            .chain(key_files().iter().map(|f| ("config".to_string(), welcome::short_path(&f.to_string_lossy(), &home(), 26))))
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

/// Lists the models and lets the person pick one by number or name.
pub fn pick_model(provider: &dyn Model, current: Option<&str>, editor: &mut Editor, color: bool) -> Option<String> {
    let models = list_models(provider, current, color)?;
    let hint =
        if current.is_some() { "Pick a number, or Enter to keep the current model:" } else { "Pick a number, or type a model's name:" };
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
    let provider = make_provider(&s.o, &backend, model, &s.stop);
    (s.window, s.window_source) = window_for(&s.o, provider.as_ref());
    s.agent.set_context_window(s.window);
    s.agent.set_model(provider);
    let moved = backend.anthropic != s.backend.anthropic;
    s.backend = backend;
    s.model = model.to_string();
    save_model(model);
    s.log.write(json!({ "event": "model", "model": model, "provider": if s.backend.anthropic { "anthropic" } else { "openai" } }));
    let via = if moved { format!(" at {}", welcome::server_name(&s.backend.base_url)) } else { String::new() };
    println!("Now using {}{via}.", paint(color, Style::Bold, model));
}

/// Runs a slash command. False means leave the chat.
pub fn run_command(s: &mut Session, editor: &mut Editor, line: &str) -> bool {
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
            println!("Model: {} at {}", paint(color, Style::Bold, &s.model), welcome::server_name(&s.backend.base_url));
            let provider = make_provider(&s.o, &s.backend, &s.model, &s.stop);
            if let Some(model) = pick_model(provider.as_ref(), Some(&s.model), editor, color) {
                switch_model(s, &model);
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
            println!("  model          {} ({})", s.model, welcome::server_name(&s.backend.base_url));
            println!("  requests       {}", s.requests);
            let cached = s.agent.usage.cached;
            let from_cache = if cached > 0 { format!(" ({} from cache)", grouped(cached)) } else { String::new() };
            println!("  input tokens   {}{from_cache}", grouped(s.agent.usage.input));
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
