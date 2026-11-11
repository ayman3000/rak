//! Skills: know-how written in Markdown that the agent loads when a task
//! needs it. Follows SwiftAgentKit's design: the system prompt carries a short
//! index (one line per skill: name and description) and the model loads a
//! skill's full instructions with `use_skill` when a task matches. A skill
//! costs a line until it is used.
//!
//! Two file formats are read:
//!
//! - SwiftAgentKit's (sak's, Naseem's): `name.md`, starting with `# Name`, an
//!   optional `Description:` line, then the instructions. Files the skill ships
//!   go in a folder next to it named like the file (`name/`).
//! - Agent Skills (Claude's): a folder holding `SKILL.md`, which starts with a
//!   `---` block giving `name:` and `description:`. The folder holds its files.
//!
//! A skill's files (references, templates, scripts) are never read for the
//! model: the instructions name them, and the model reads one when it needs it.

use crate::tools::{str_arg, Tool, ToolResult};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const MAX_DESCRIPTION_CHARS: usize = 200;

#[derive(Debug, Clone, PartialEq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub instructions: String,
    /// The folder of files the skill ships, if it has one.
    pub resources: Option<PathBuf>,
    /// The file it was read from; for a skill built into rak, its place in rak's repository.
    pub file: PathBuf,
    /// Shipped inside rak rather than read from a folder.
    pub builtin: bool,
}

impl Skill {
    /// The skill as `use_skill` hands it to the model.
    pub fn render(&self) -> String {
        let mut out = format!("[Skill \"{}\" loaded — follow these instructions for this task:]\n{}", self.name, self.instructions);
        if let Some(folder) = &self.resources {
            out.push_str(&format!(
                "\n\n[This skill ships files in {}. Paths the instructions mention are relative to it. Read one only \
                 when the instructions above point you at it.]",
                folder.display()
            ));
        }
        out
    }
}

/// Pure — unit-tested. A skill file in either format → (name, description,
/// instructions). None when it has no name or no instructions.
pub fn parse(markdown: &str) -> Option<(String, String, String)> {
    let text = markdown.trim_start_matches('\u{feff}');
    let (name, mut description, body) = if let Some(rest) = text.strip_prefix("---") {
        // Agent Skills: a front-matter block of `key: value` lines.
        let end = rest.find("\n---")?;
        let (header, body) = (&rest[..end], &rest[end + 4..]);
        let field = |key: &str| {
            header.lines().find_map(|line| {
                let (k, v) = line.split_once(':')?;
                (k.trim() == key).then(|| v.trim().trim_matches('"').trim_matches('\'').to_string())
            })
        };
        (field("name")?, field("description").unwrap_or_default(), body.to_string())
    } else {
        // SwiftAgentKit: `# Name`, header lines, then the body.
        let mut name = None;
        let mut description = String::new();
        let mut body = Vec::new();
        let mut in_body = false;
        for line in text.lines() {
            let trimmed = line.trim();
            if name.is_none() && !in_body {
                if let Some(title) = trimmed.strip_prefix("# ") {
                    name = Some(title.trim().to_string());
                    continue;
                }
            }
            if !in_body {
                let lower = trimmed.to_lowercase();
                if trimmed.is_empty() {
                    continue;
                } else if lower.starts_with("description:") {
                    description = trimmed["description:".len()..].trim().to_string();
                    continue;
                } else if lower.starts_with("triggers:") || lower.starts_with("tools:") || lower.starts_with("tier:") {
                    continue; // SwiftAgentKit metadata rak does not use
                }
                in_body = true;
            }
            body.push(line);
        }
        (name?, description, body.join("\n"))
    };
    let instructions = body.trim().to_string();
    if name.trim().is_empty() || instructions.is_empty() {
        return None;
    }
    // No description: the first line of the instructions stands in.
    if description.is_empty() {
        description = instructions.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").trim_start_matches('#').trim().to_string();
    }
    if description.chars().count() > MAX_DESCRIPTION_CHARS {
        description = description.chars().take(MAX_DESCRIPTION_CHARS).collect::<String>() + "…";
    }
    Some((name.trim().to_string(), description, instructions))
}

fn read(file: &Path, resources: Option<PathBuf>) -> Option<Skill> {
    let text = std::fs::read_to_string(file).ok()?;
    let (name, description, instructions) = parse(&text)?;
    Some(Skill { name, description, instructions, resources, file: file.to_path_buf(), builtin: false })
}

/// The skills in one folder, by name.
pub fn load_folder(folder: &Path) -> Vec<Skill> {
    let Ok(entries) = std::fs::read_dir(folder) else { return Vec::new() };
    let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    paths.sort();
    let mut out = Vec::new();
    for path in paths {
        if path.is_dir() {
            // Agent Skills: a folder with a SKILL.md.
            let file = path.join("SKILL.md");
            if file.is_file() {
                out.extend(read(&file, Some(path.clone())));
            }
        } else if path.extension().is_some_and(|e| e == "md") {
            // SwiftAgentKit: name.md, its files in name/.
            let folder = path.with_extension("");
            out.extend(read(&path, folder.is_dir().then_some(folder)));
        }
    }
    out
}

/// The skills from every folder, the first folder winning on a name. Sorted by
/// name, so the index stays the same from one call to the next (it is part of
/// the prompt, which servers cache).
pub fn load(folders: &[PathBuf]) -> Vec<Skill> {
    let mut out: Vec<Skill> = Vec::new();
    for folder in folders {
        for skill in load_folder(folder) {
            if !out.iter().any(|s| s.name.eq_ignore_ascii_case(&skill.name)) {
                out.push(skill);
            }
        }
    }
    out.sort_by_key(|s| s.name.to_lowercase());
    out
}

/// Skills shipped inside rak: (file in the repository's skills/ folder, its
/// text, the systems it applies to; empty means every system).
const BUILT_IN: &[(&str, &str, &[&str])] = &[("skills/ios-app.md", include_str!("../skills/ios-app.md"), &["macos"])];

/// The skills built into rak that apply on this system (`std::env::consts::OS`).
pub fn built_in(os: &str) -> Vec<Skill> {
    BUILT_IN
        .iter()
        .filter(|(_, _, systems)| systems.is_empty() || systems.contains(&os))
        .filter_map(|(file, text, _)| {
            let (name, description, instructions) = parse(text)?;
            Some(Skill { name, description, instructions, resources: None, file: PathBuf::from(file), builtin: true })
        })
        .collect()
}

/// The skills from the folders plus the built-in ones for this system. A skill
/// in a folder replaces a built-in one with the same name.
pub fn load_with_built_in(folders: &[PathBuf], os: &str) -> Vec<Skill> {
    let mut skills = load(folders);
    for skill in built_in(os) {
        if !skills.iter().any(|s| s.name.eq_ignore_ascii_case(&skill.name)) {
            skills.push(skill);
        }
    }
    skills.sort_by_key(|s| s.name.to_lowercase());
    skills
}

/// Pure — unit-tested. The part of the system prompt that lists the skills.
/// Empty when there are none.
pub fn index(skills: &[Skill]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let lines: Vec<String> = skills.iter().map(|s| format!("- {} — {}", s.name, s.description)).collect();
    format!(
        "\n\nSkills — reusable procedures. BEFORE starting a task that matches one, load it with `use_skill(name)` \
         and follow it:\n{}",
        lines.join("\n")
    )
}

/// Loads a skill's full instructions.
pub struct UseSkill {
    skills: Vec<Skill>,
}

impl UseSkill {
    pub fn new(skills: Vec<Skill>) -> Self {
        UseSkill { skills }
    }
}

impl Tool for UseSkill {
    fn name(&self) -> &str {
        "use_skill"
    }
    fn description(&self) -> &str {
        "Load a skill's full instructions by name. The skills index in your system prompt lists every available skill \
         with a one-line description — when a task matches one, call this BEFORE starting the task and follow the \
         loaded instructions."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "name": { "type": "string", "description": "Exact skill name from the skills index." } },
            "required": ["name"]
        })
    }
    fn run(&self, args: &Value) -> ToolResult {
        let Some(wanted) = str_arg(args, "name").map(str::trim).filter(|n| !n.is_empty()) else {
            return ToolResult::err("use_skill requires `name`.");
        };
        match self.skills.iter().find(|s| s.name.eq_ignore_ascii_case(wanted)) {
            Some(skill) => ToolResult::ok(skill.render()),
            // An unknown name gets the list, so the model can correct itself in one step.
            None => {
                let names: Vec<&str> = self.skills.iter().map(|s| s.name.as_str()).collect();
                ToolResult::err(format!("No skill named \"{wanted}\". Available skills: {}.", names.join(", ")))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::testing::temp_dir;

    #[test]
    fn swiftagentkit_files_are_read() {
        let (name, description, body) =
            parse("# iOS app\n\nDescription: Build and run an iOS app in the simulator.\nTriggers: ios, xcode\n\n## Steps\n1. Build.\n")
                .unwrap();
        assert_eq!((name.as_str(), description.as_str()), ("iOS app", "Build and run an iOS app in the simulator."));
        assert_eq!(body, "## Steps\n1. Build.");
    }

    #[test]
    fn agent_skills_files_are_read() {
        let (name, description, body) =
            parse("---\nname: pdf-tools\ndescription: \"Fill PDF forms\"\nlicense: MIT\n---\n\n# PDF\nUse pypdf.\n").unwrap();
        assert_eq!((name.as_str(), description.as_str(), body.as_str()), ("pdf-tools", "Fill PDF forms", "# PDF\nUse pypdf."));
    }

    #[test]
    fn a_missing_description_comes_from_the_first_line_and_long_ones_are_cut() {
        let (_, description, _) = parse("# Notes\n\n## Keep notes short\nMore.").unwrap();
        assert_eq!(description, "Keep notes short");
        let (_, long, _) = parse(&format!("# X\nDescription: {}\nBody", "d".repeat(500))).unwrap();
        assert_eq!(long.chars().count(), MAX_DESCRIPTION_CHARS + 1);
        assert!(parse("No title here").is_none());
        assert!(parse("# Title only\n").is_none());
        assert!(parse("---\ndescription: no name\n---\nbody").is_none());
    }

    #[test]
    fn built_in_skills_parse_and_apply_only_where_they_should() {
        for (file, text, _) in BUILT_IN {
            assert!(parse(text).is_some(), "{file} does not parse");
        }
        let mac = built_in("macos");
        assert!(mac.iter().any(|s| s.name == "iOS app" && s.builtin && s.resources.is_none()));
        assert!(built_in("linux").iter().all(|s| s.name != "iOS app"), "no iOS skill off a Mac");
    }

    #[test]
    fn a_skill_of_your_own_replaces_the_built_in_one() {
        let dir = temp_dir("skills-override");
        std::fs::write(dir.join("ios.md"), "# iOS app\nDescription: my own way\nSteps.").unwrap();
        let skills = load_with_built_in(std::slice::from_ref(&dir), "macos");
        let ios: Vec<&Skill> = skills.iter().filter(|s| s.name == "iOS app").collect();
        assert_eq!(ios.len(), 1);
        assert!(!ios[0].builtin && ios[0].description == "my own way");
        assert!(load_with_built_in(&[], "macos").iter().any(|s| s.builtin));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn folders_are_read_in_both_formats_and_the_first_folder_wins() {
        let dir = temp_dir("skills");
        let (project, user) = (dir.join("project"), dir.join("user"));
        std::fs::create_dir_all(project.join("deploy")).unwrap();
        std::fs::create_dir_all(user.join("pdf-tools")).unwrap();
        std::fs::write(project.join("deploy.md"), "# Deploy\nDescription: project version\nSteps.").unwrap();
        std::fs::write(user.join("deploy.md"), "# deploy\nDescription: user version\nSteps.").unwrap();
        std::fs::write(user.join("pdf-tools/SKILL.md"), "---\nname: pdf-tools\ndescription: PDFs\n---\nUse pypdf.").unwrap();
        std::fs::write(user.join("notes.txt"), "not a skill").unwrap();
        std::fs::write(user.join("broken.md"), "no title").unwrap();

        let skills = load(&[project.clone(), user.clone(), dir.join("missing")]);
        let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Deploy", "pdf-tools"]);
        assert_eq!(skills[0].description, "project version");
        assert_eq!(skills[0].resources.as_deref(), Some(project.join("deploy").as_path()));
        assert_eq!(skills[1].resources.as_deref(), Some(user.join("pdf-tools").as_path()));

        let text = index(&skills);
        assert!(text.contains("- Deploy — project version\n- pdf-tools — PDFs"));
        assert!(index(&[]).is_empty());

        let tool = UseSkill::new(skills);
        assert!(!tool.needs_approval());
        let loaded = tool.run(&json!({ "name": "deploy" }));
        assert!(loaded.ok && loaded.text.contains("Steps.") && loaded.text.contains("ships files in"), "{}", loaded.text);
        let unknown = tool.run(&json!({ "name": "nope" }));
        assert!(!unknown.ok && unknown.text.contains("Available skills: Deploy, pdf-tools."));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
