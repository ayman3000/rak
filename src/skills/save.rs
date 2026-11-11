//! Writing skills: `save_skill`, the tool the agent uses to keep what it
//! learned (a procedure the user asked for, or the right way after a mistake),
//! and the file work `rak skills add|remove` shares with it.
//!
//! Learned from SwiftAgentKit's `learn_skill`: one Markdown file per skill,
//! `slug.md`, in the format its `FileAgentSkillStore` writes, so sak and
//! Naseem read rak's skills and the other way round. Unlike `learn_skill` it
//! asks first: a skill is instructions every later session follows, so the
//! person sees it whole before it is kept.

use super::{load_folder, parse, Shared, Skill};
use crate::text::one_line;
use crate::tools::{str_arg, Tool, ToolResult};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Pure — unit-tested. A file name from a skill name: "iOS app!" → "ios-app".
pub fn slug(name: &str) -> String {
    let mapped: String = name.to_lowercase().chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect();
    let joined = mapped.split('-').filter(|part| !part.is_empty()).collect::<Vec<_>>().join("-");
    if joined.is_empty() {
        "skill".to_string()
    } else {
        joined
    }
}

/// Pure — unit-tested. A skill in SwiftAgentKit's format.
pub fn to_markdown(name: &str, description: &str, instructions: &str) -> String {
    format!("# {}\nDescription: {}\n\n{}\n", one_line(name, 200), one_line(description, 400), instructions.trim())
}

/// Agent Skills text (a `SKILL.md`) starts with a `---` block; anything else
/// is SwiftAgentKit's format.
pub fn is_agent_skills(text: &str) -> bool {
    text.trim_start_matches('\u{feff}').starts_with("---")
}

/// Where a skill goes in `folder`, and the skill with that name already there.
/// A skill of that name replaces the one there when both are in the same
/// format; across formats it is refused, so nothing is silently left behind.
fn place(folder: &Path, name: &str, agent_skills: bool) -> Result<(PathBuf, Option<Skill>), String> {
    let existing = load_folder(folder).into_iter().find(|s| s.name.eq_ignore_ascii_case(name));
    if let Some(old) = existing {
        let old_is_agent_skills = old.file.file_name().is_some_and(|f| f == "SKILL.md");
        if old_is_agent_skills != agent_skills {
            return Err(format!(
                "a skill named \"{}\" is already at {}, in the other file format. Change that file instead, or remove it first \
                 (rak skills remove \"{}\")",
                old.name,
                old.file.display(),
                old.name
            ));
        }
        return Ok((old.file.clone(), Some(old)));
    }
    // A new skill: a file name nothing else uses.
    let base = slug(name);
    let path_for = |n: usize| {
        let stem = if n == 1 { base.clone() } else { format!("{base}-{n}") };
        if agent_skills {
            folder.join(stem).join("SKILL.md")
        } else {
            folder.join(format!("{stem}.md"))
        }
    };
    let n = (1..).find(|n| {
        let path = path_for(*n);
        // For SwiftAgentKit's format, a folder of that name holds another skill's files.
        !path.exists() && (agent_skills || !path.with_extension("").exists())
    });
    Ok((path_for(n.unwrap_or(1)), None))
}

/// Writes a skill's text (either format) into `folder`, replacing a skill of
/// the same name there. Checks the file reads back as that skill.
/// Returns the skill as read back and the one it replaced.
pub fn write(folder: &Path, text: &str) -> Result<(Skill, Option<Skill>), String> {
    let Some((name, _, _)) = parse(text) else {
        return Err("this is not a skill: it needs a name (a \"# Name\" title, or a ---\\nname: ...\\n--- block) and instructions".into());
    };
    let agent_skills = is_agent_skills(text);
    let (path, replaced) = place(folder, &name, agent_skills)?;
    let parent = path.parent().unwrap_or(folder);
    std::fs::create_dir_all(parent).map_err(|e| format!("cannot make {}: {e}", parent.display()))?;
    std::fs::write(&path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    let resources = if agent_skills { Some(parent.to_path_buf()) } else { Some(path.with_extension("")).filter(|f| f.is_dir()) };
    let saved = std::fs::read_to_string(&path).ok().and_then(|t| parse(&t));
    match saved {
        Some((saved_name, description, instructions)) if saved_name == name => {
            Ok((Skill { name, description, instructions, resources, file: path, builtin: false }, replaced))
        }
        _ => Err(format!("wrote {} but it does not read back as the skill \"{name}\"", path.display())),
    }
}

/// Deletes the skill named `name` from `folder`, with the files it ships.
/// Returns what was removed.
pub fn remove(folder: &Path, name: &str) -> Result<PathBuf, String> {
    let Some(skill) = load_folder(folder).into_iter().find(|s| s.name.eq_ignore_ascii_case(name)) else {
        return Err(format!("no skill named \"{name}\" in {}", folder.display()));
    };
    let result = if skill.file.file_name().is_some_and(|f| f == "SKILL.md") {
        // Agent Skills: the folder is the skill.
        let dir = skill.file.parent().unwrap_or(folder).to_path_buf();
        std::fs::remove_dir_all(&dir).map(|_| dir)
    } else {
        // SwiftAgentKit: the file, and the folder of files next to it (a later
        // skill of the same name must not inherit them).
        std::fs::remove_file(&skill.file).map(|_| {
            if let Some(dir) = &skill.resources {
                let _ = std::fs::remove_dir_all(dir);
            }
            skill.file.clone()
        })
    };
    result.map_err(|e| format!("cannot remove {}: {e}", skill.file.display()))
}

/// Puts `skill` in the run's list, in place of any skill with its name.
pub fn share(skills: &Shared, skill: Skill) {
    let mut list = skills.write().unwrap();
    list.retain(|s| !s.name.eq_ignore_ascii_case(&skill.name));
    list.push(skill);
    list.sort_by_key(|s| s.name.to_lowercase());
}

/// Added to the system prompt with `save_skill`: the tool's description alone
/// did not make a model keep a lesson after the user corrected it.
pub const LEARNING: &str = "\n\nLearning: if the user corrected how you work, or something worked only after failed tries, \
     then before your final answer keep the lesson for next time: load the skill \"Write a skill\" and save it with \
     save_skill. Skip one-off slips. Don't wait to be asked; the user approves each save.";

/// Added to what the model is told when the user says no to a tool call: a
/// "no" is the clearest sign the agent was about to do something the wrong
/// way, and the user's next message often says the right one. A line in the
/// system prompt alone did not make a model keep the lesson.
pub const AFTER_REFUSAL: &str = " If the user then tells you the right way and it will matter again, save that as a \
     skill with save_skill before your final answer.";

/// The agent's tool for keeping a skill.
pub struct SaveSkill {
    skills: Shared,
    /// The project's skills folder (`.rak/skills`).
    project: PathBuf,
    /// The user's (`~/.config/rak/skills`), when there is a config folder.
    user: Option<PathBuf>,
}

impl SaveSkill {
    pub fn new(skills: Shared, project: PathBuf, user: Option<PathBuf>) -> Self {
        SaveSkill { skills, project, user }
    }
}

impl Tool for SaveSkill {
    fn name(&self) -> &str {
        "save_skill"
    }
    fn description(&self) -> &str {
        "Save a skill for this and later sessions (same name: replaces it). Load the skill \"Write a skill\" first."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "description": { "type": "string", "description": "One line: which task, when." },
                "instructions": { "type": "string", "description": "Markdown." },
                "scope": { "type": "string", "enum": ["user", "project"], "description": "user (default): every project." }
            },
            "required": ["name", "description", "instructions"]
        })
    }
    fn needs_approval(&self) -> bool {
        true
    }
    fn run(&self, args: &Value) -> ToolResult {
        let field = |key: &str| str_arg(args, key).map(str::trim).filter(|v| !v.is_empty());
        let (Some(name), Some(description), Some(instructions)) = (field("name"), field("description"), field("instructions")) else {
            return ToolResult::err("save_skill requires `name`, `description` and `instructions`.");
        };
        if name.chars().count() > 80 {
            return ToolResult::err("The name is too long: keep it to a few words, and put the rest in `description`.");
        }
        let folder = match field("scope") {
            Some("project") => self.project.clone(),
            _ => match &self.user {
                Some(user) => user.clone(),
                None => return ToolResult::err("There is no config folder for skills; save it with scope `project`."),
            },
        };
        let was_built_in = self.skills.read().unwrap().iter().any(|s| s.builtin && s.name.eq_ignore_ascii_case(name));
        match write(&folder, &to_markdown(name, description, instructions)) {
            Ok((skill, replaced)) => {
                let note = match (replaced, was_built_in) {
                    (Some(_), _) => " It replaces the earlier version.",
                    (None, true) => " It replaces the built-in skill of that name.",
                    _ => "",
                };
                let text = format!(
                    "Saved skill \"{}\" to {}.{note} use_skill loads it now; from the next session it is in the skills index.",
                    skill.name,
                    skill.file.display()
                );
                share(&self.skills, skill);
                ToolResult::ok(text)
            }
            Err(why) => ToolResult::err(format!("Not saved: {why}.")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::UseSkill;
    use crate::tools::testing::temp_dir;
    use std::sync::{Arc, RwLock};

    #[test]
    fn names_become_file_names() {
        assert_eq!(slug("iOS app!"), "ios-app");
        assert_eq!(slug("  Deploy -- to   staging "), "deploy-to-staging");
        assert_eq!(slug("???"), "skill");
        let text = to_markdown("Deploy", "Ship it\nsafely", "1. Test.\n2. Tag.\n");
        assert_eq!(text, "# Deploy\nDescription: Ship it safely\n\n1. Test.\n2. Tag.\n");
        let (name, description, body) = parse(&text).unwrap();
        assert_eq!((name.as_str(), description.as_str(), body.as_str()), ("Deploy", "Ship it safely", "1. Test.\n2. Tag."));
    }

    #[test]
    fn a_saved_skill_is_usable_at_once_and_replaces_one_of_its_name() {
        let dir = temp_dir("save-skill");
        let (project, user) = (dir.join("project"), dir.join("user"));
        let shared: Shared = Arc::new(RwLock::new(super::super::built_in("linux")));
        let save = SaveSkill::new(shared.clone(), project.clone(), Some(user.clone()));
        let use_skill = UseSkill::new(shared.clone());
        assert!(save.needs_approval());

        let args = json!({ "name": "Deploy", "description": "Ship to staging", "instructions": "1. Run the tests.\n2. Push." });
        let saved = save.run(&args);
        assert!(saved.ok && saved.text.contains("deploy.md") && !saved.text.contains("replaces"), "{}", saved.text);
        assert!(user.join("deploy.md").is_file() && !project.exists());
        assert!(use_skill.run(&json!({ "name": "deploy" })).text.contains("1. Run the tests."));

        // Same name, other case: the same file, replaced.
        let again = save.run(&json!({ "name": "deploy", "description": "Ship", "instructions": "Just push." }));
        assert!(again.ok && again.text.contains("replaces the earlier version"), "{}", again.text);
        assert_eq!(std::fs::read_dir(&user).unwrap().count(), 1);
        assert!(use_skill.run(&json!({ "name": "Deploy" })).text.contains("Just push."));
        assert_eq!(shared.read().unwrap().iter().filter(|s| s.name.eq_ignore_ascii_case("deploy")).count(), 1);

        // Project scope; a built-in replaced.
        let own = save.run(&json!({ "name": "Write a skill", "description": "Mine", "instructions": "My way.", "scope": "project" }));
        assert!(own.ok && own.text.contains("replaces the built-in"), "{}", own.text);
        assert!(project.join("write-a-skill.md").is_file());

        assert!(!save.run(&json!({ "name": "X", "instructions": "no description" })).ok);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn both_formats_are_written_kept_apart_and_removed_with_their_files() {
        let dir = temp_dir("write-skill");
        // A file already named like the new skill but holding another one: a new name is picked.
        std::fs::write(dir.join("deploy.md"), "# Ship\nDescription: other\nSteps.").unwrap();
        let (skill, replaced) = write(&dir, "# Deploy\nDescription: d\n\nSteps.").unwrap();
        assert!(replaced.is_none() && skill.file == dir.join("deploy-2.md"));

        let agent_skills = "---\nname: pdf-tools\ndescription: PDFs\nlicense: MIT\n---\nUse pypdf.\n";
        let (pdf, _) = write(&dir, agent_skills).unwrap();
        assert_eq!(pdf.file, dir.join("pdf-tools/SKILL.md"));
        assert_eq!(std::fs::read_to_string(&pdf.file).unwrap(), agent_skills, "kept word for word");
        std::fs::write(dir.join("pdf-tools/form.py"), "print()").unwrap();

        // The same name in the other format is refused rather than leaving two.
        let clash = write(&dir, "# pdf-tools\nDescription: x\nBody.").unwrap_err();
        assert!(clash.contains("other file format"), "{clash}");
        assert!(write(&dir, "<html>not a skill</html>").is_err());

        assert_eq!(remove(&dir, "PDF-TOOLS").unwrap(), dir.join("pdf-tools"));
        assert!(!dir.join("pdf-tools").exists());
        std::fs::create_dir_all(dir.join("deploy-2")).unwrap();
        remove(&dir, "deploy").unwrap();
        assert!(!dir.join("deploy-2.md").exists() && !dir.join("deploy-2").exists());
        assert!(remove(&dir, "deploy").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
