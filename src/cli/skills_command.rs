//! `rak skills list|add|remove`, and the `add` that `/skills add` in a chat
//! shares. A skill from elsewhere is shown and asked about before it is saved:
//! it is instructions the agent will follow in every later session.

use super::args::usage_error;
use super::settings::{config_dir, home};
use super::style::{cut, paint, Style};
use super::welcome::short_path;
use rak::skills::{self, fetch, save, Skill};
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

pub const SKILLS_HELP: &str = "\
rak skills — know-how the agent loads when a task needs it

  rak skills [list]                       the skills, and where each comes from
  rak skills add URL|FILE [--project] [-y]
                                          show a skill, then save it if you say yes
  rak skills remove NAME [--project]      delete one you saved, with its files

OPTIONS
  --project   ./.rak/skills (this project only) instead of ~/.config/rak/skills
  -y, --yes   save without asking (only for skills you have read); each skill
              is then summed up in a few lines, not shown

A skill is a NAME.md file starting with \"# Name\" and a \"Description:\" line
(SwiftAgentKit's format, which sak and Naseem read too), or a folder with a
SKILL.md (Agent Skills, Claude's format). For GitHub, paste the page's address:
a file's page, a skill folder's page, or a whole repository's page (every skill
in it is shown and asked about, one at a time).

In a chat, /skills add URL and /skills remove NAME do the same (remove asks
first, and finds the skill in this project or in yours), and the agent can write skills with
you: ask it for one, or it offers one after it learns something the hard way.
";

/// Never returns.
pub fn run(args: &[String]) -> ! {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{SKILLS_HELP}");
        std::process::exit(0);
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let folders = skills::folders(&cwd, config_dir().as_deref());
    let project = args.iter().any(|a| a == "--project");
    let yes = args.iter().any(|a| a == "-y" || a == "--yes");
    let words: Vec<&String> = args.iter().skip(1).filter(|a| !a.starts_with('-')).collect();
    let folder =
        if project { folders[0].clone() } else { folders.get(1).cloned().unwrap_or_else(|| usage_error("no home folder for skills")) };
    let color = std::io::stdout().is_terminal();
    let fail = |why: String| -> ! {
        eprintln!("rak: {why}");
        std::process::exit(1);
    };

    match args.first().map(String::as_str) {
        None | Some("list" | "ls") => {
            let all = skills::load_with_built_in(&folders, std::env::consts::OS);
            print_list(&all, color);
        }
        Some("add") => {
            let [source] = words.as_slice() else { usage_error("rak skills add needs one URL or file (see rak skills --help)") };
            if project {
                // The person adds it to this folder: it is theirs to trust.
                super::trust::remember(&cwd);
            }
            match add_all(source, &folder, yes, color) {
                Ok(saved) if !saved.is_empty() => {}
                Ok(_) => std::process::exit(1),
                Err(why) => fail(why),
            }
        }
        Some("remove" | "rm") => {
            let [name] = words.as_slice() else { usage_error("rak skills remove needs a name") };
            match save::remove(&folder, name) {
                Ok(path) => println!("Removed {}", short_path(&path.to_string_lossy(), &home(), 70)),
                Err(why) => {
                    let built_in = skills::built_in(std::env::consts::OS).into_iter().any(|s| s.name.eq_ignore_ascii_case(name));
                    if built_in {
                        fail(format!("\"{name}\" is built into rak. Run with --no-skills to leave skills out, or save your own with that name to replace it"));
                    }
                    let elsewhere = if project { "" } else { " (for a project's skill, add --project)" };
                    fail(format!("{why}{elsewhere}"))
                }
            }
        }
        Some(other) => usage_error(&format!("rak skills has list, add and remove, not `{other}` (see rak skills --help)")),
    }
    std::process::exit(0);
}

/// `/skills remove NAME` in a chat: deletes a saved skill wherever it lives
/// (this project's or yours), after a yes. Ok(true): removed. A built-in
/// skill can't be removed.
pub fn remove_in_chat(name: &str, all: &[Skill]) -> Result<bool, String> {
    let Some(skill) = all.iter().find(|s| s.name.eq_ignore_ascii_case(name)) else {
        let names: Vec<&str> = all.iter().filter(|s| !s.builtin).map(|s| s.name.as_str()).collect();
        let have = if names.is_empty() { "you have saved none".to_string() } else { format!("yours are: {}", names.join(", ")) };
        return Err(format!("no skill named \"{name}\" ({have})"));
    };
    if skill.builtin {
        return Err(format!("\"{}\" is built into rak, so there is nothing to delete. --no-skills leaves skills out of a run", skill.name));
    }
    let folder = save::home_folder(skill).ok_or_else(|| format!("cannot tell where \"{}\" is saved", skill.name))?;
    print!("Remove \"{}\" ({})? Its files are deleted. [y/N] ", skill.name, short_path(&skill.file.to_string_lossy(), &home(), 60));
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    let _ = std::io::stdin().lock().read_line(&mut answer);
    if !matches!(answer.trim(), "y" | "Y" | "yes") {
        println!("Not removed.");
        return Ok(false);
    }
    let path = save::remove(&folder, &skill.name)?;
    println!("Removed {}", short_path(&path.to_string_lossy(), &home(), 70));
    Ok(true)
}

/// The list `rak skills` and `/skills` print.
pub fn print_list(skills: &[Skill], color: bool) {
    let dim = |text: &str| paint(color, Style::Dim, text);
    if skills.is_empty() {
        println!("{}", dim("No skills. A skill is a Markdown file of know-how the agent loads when a task needs it:"));
        println!("{}", dim("  .rak/skills/NAME.md in a project, or ~/.config/rak/skills/NAME.md for every project."));
    }
    for skill in skills {
        println!("  {}  {}", paint(color, Style::Cyan, &skill.name), dim(&cut(&skill.description, 90)));
        let place = if skill.builtin { "built into rak".to_string() } else { short_path(&skill.file.to_string_lossy(), &home(), 70) };
        println!("    {}", dim(&place));
    }
    println!("{}", dim("Add one: rak skills add URL, or ask the agent to write one with you. Remove one: rak skills remove NAME."));
}

/// `add` for everything an address stands for: one skill, or every skill in
/// a GitHub repository (or folder of skills), each shown and asked about in
/// turn. Returns the ones saved. One that can't be read is said and skipped
/// when there are several.
pub fn add_all(source: &str, folder: &Path, yes: bool, color: bool) -> Result<Vec<Skill>, String> {
    let sources = if source.starts_with("https://") { fetch::expand(source)? } else { vec![source.to_string()] };
    if let [only] = sources.as_slice() {
        return Ok(add(only, folder, yes, color)?.into_iter().collect());
    }
    let names: Vec<&str> = sources.iter().map(|s| s.rsplit('/').next().unwrap_or(s)).collect();
    println!("{} skills there: {}.", sources.len(), names.join(", "));
    if !yes {
        println!("{}", paint(color, Style::Dim, "Each is shown and asked about in turn."));
    }
    let mut saved = Vec::new();
    for (i, one) in sources.iter().enumerate() {
        if yes {
            println!();
        } else {
            println!("\n{}", paint(color, Style::Dim, &format!("── {} of {} ──", i + 1, sources.len())));
        }
        match add(one, folder, yes, color) {
            Ok(Some(skill)) => saved.push(skill),
            Ok(None) => {}
            Err(why) => println!("{}", paint(color, Style::Yellow, &format!("Skipped: {why}"))),
        }
    }
    println!("\nSaved {} of {} skills.", saved.len(), sources.len());
    Ok(saved)
}

/// Reads a skill from a web address or a file, shows it, asks, and saves it
/// into `folder`. Ok(None): not saved (the answer was no, or nobody could be asked).
pub fn add(source: &str, folder: &Path, yes: bool, color: bool) -> Result<Option<Skill>, String> {
    let text = if source.starts_with("http://") || source.starts_with("https://") {
        fetch::fetch(source)?
    } else {
        std::fs::read_to_string(source).map_err(|e| format!("cannot read {source}: {e}"))?
    };
    let Some((name, description, instructions)) = skills::parse(&text) else {
        return Err(format!(
            "{source} is not a skill: a skill starts with \"# Name\" and a \"Description:\" line, or with a ---\\nname: ...\\n--- block"
        ));
    };
    let bold = |t: &str| paint(color, Style::Bold, t);
    let dim = |t: &str| paint(color, Style::Dim, t);

    println!("{}  {}", bold(&name), dim(&description));
    let lines: Vec<&str> = instructions.lines().collect();
    if yes {
        // Nothing is asked, so the text is not shown to be read: a line says
        // where it is from and how long it is. Flagged lines still follow.
        println!("{}", dim(&format!("from {source} · {} lines", lines.len())));
    } else {
        println!("{}", dim(&format!("from {source}")));
        const SHOWN: usize = 40;
        for line in lines.iter().take(SHOWN) {
            println!("  │ {line}");
        }
        if lines.len() > SHOWN {
            println!("  │ {}", dim(&format!("… {} more lines. Read them all at {source} before saying yes.", lines.len() - SHOWN)));
        }
    }
    // A GitHub skill folder: the files it ships come too, read for the review first.
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for (path, url) in fetch::github_files(source)? {
        files.push((path, fetch::fetch_bytes(&url, fetch::MAX_FILE_BYTES)?));
    }
    let mut flagged: Vec<(String, usize, String)> =
        fetch::worth_a_look(&text).into_iter().map(|(n, line)| ("SKILL".to_string(), n, line)).collect();
    for (path, bytes) in &files {
        if let Ok(file_text) = std::str::from_utf8(bytes) {
            flagged.extend(fetch::worth_a_look(file_text).into_iter().map(|(n, line)| (path.clone(), n, line)));
        }
    }
    if !files.is_empty() {
        let names: Vec<&str> = files.iter().map(|(path, _)| path.as_str()).collect();
        println!(
            "{}",
            dim(&format!("It ships {} file{}: {}", files.len(), if files.len() == 1 { "" } else { "s" }, cut(&names.join(", "), 200)))
        );
    } else if skills::save::is_agent_skills(&text) && fetch::github_folder(source).is_none() {
        println!("{}", dim("Only this file is saved: any files a skill folder ships next to it are not downloaded (give the folder's GitHub page to get them)."));
    }
    if !flagged.is_empty() {
        println!("{}", paint(color, Style::Yellow, "Lines worth a look before you trust it:"));
        for (file, n, line) in flagged.iter().take(15) {
            let place = if file == "SKILL" { format!("{n:>4}") } else { format!("{file}:{n}") };
            println!("  {} {}", paint(color, Style::Yellow, &place), cut(line, 100));
        }
        if flagged.len() > 15 {
            println!("  {}", dim(&format!("… and {} more", flagged.len() - 15)));
        }
    }

    let target = short_path(&folder.to_string_lossy(), &home(), 60);
    if !yes {
        if !std::io::stdin().is_terminal() {
            eprintln!("rak: not saved — there is no terminal to ask (add -y to save a skill you have read)");
            return Ok(None);
        }
        print!("Save \"{name}\" to {target}? A skill is instructions the agent follows. [y/N] ");
        let _ = std::io::stdout().flush();
        let mut answer = String::new();
        let _ = std::io::stdin().lock().read_line(&mut answer);
        if !matches!(answer.trim(), "y" | "Y" | "yes") {
            println!("Not saved.");
            return Ok(None);
        }
    }
    let (skill, replaced) = save::write(folder, &text)?;
    if let Some(dir) = skill.resources.as_ref().filter(|_| !files.is_empty()) {
        for (path, bytes) in &files {
            let target = dir.join(path);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("cannot make {}: {e}", parent.display()))?;
            }
            std::fs::write(&target, bytes).map_err(|e| format!("cannot write {}: {e}", target.display()))?;
        }
    }
    let note = if replaced.is_some() { " (it replaces your earlier one)" } else { "" };
    println!("Saved {}{note}.", short_path(&skill.file.to_string_lossy(), &home(), 70));
    Ok(Some(skill))
}
