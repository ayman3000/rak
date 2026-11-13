//! A project's own notes, in a visible `rak/` folder inside the project. From
//! Naseem's `ProjectNotes` (its `naseem/` folder):
//!
//!   rak/project.md   what this is, who it is for, stack, how to build and test
//!   rak/roadmap.md   ## Done / ## In progress / ## Next / ## Known issues
//!   rak/journal.md   one dated line per request, newest last; past 30 the
//!                    oldest move to journal-archive.md, never deleted
//!   rak/rules.md     the project's instructions (with AGENTS.md, CLAUDE.md),
//!                    including an optional `## Gate` command
//!   rak/memory.md    notes kept with `remember` (see memory.rs)
//!
//! Every session starts with the instructions, project.md and the roadmap's
//! working sections already in its prompt, so it does not spend its first
//! steps reading them. After a request that changed something, one small
//! model call writes the journal line and names roadmap edits — move a card,
//! add a card — which are applied to the file as it is: nothing is deleted
//! (Naseem learned this when a model rewrote a roadmap and dropped 8 issues).
//! Plain files: read them, edit them, commit them.

use std::path::{Path, PathBuf};

pub const FOLDER: &str = "rak";
pub const PROJECT: &str = "project.md";
pub const ROADMAP: &str = "roadmap.md";
pub const JOURNAL: &str = "journal.md";
pub const ARCHIVE: &str = "journal-archive.md";
pub const RULES: &str = "rules.md";
/// Most of one notes file put in a prompt.
const READ_CAP: usize = 6_000;
/// Most of one instruction file put in a prompt.
const INSTRUCTION_CAP: usize = 8_000;
const JOURNAL_CAP: usize = 30;

pub fn folder(project: &Path) -> PathBuf {
    project.join(FOLDER)
}

/// The project's instruction files, in order: (shown name, path in the project).
const INSTRUCTION_FILES: [&str; 3] = ["rak/rules.md", "AGENTS.md", "CLAUDE.md"];

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Snapshot {
    pub project: Option<String>,
    pub roadmap: Option<String>,
    pub journal: Option<String>,
    /// (file name, text), each capped.
    pub instructions: Vec<(String, String)>,
}

impl Snapshot {
    /// The project has notes (project.md or roadmap.md).
    pub fn exists(&self) -> bool {
        self.project.is_some() || self.roadmap.is_some()
    }

    /// What the after-request call is shown: the notes, the journal's end.
    fn brief(&self) -> String {
        let mut parts = Vec::new();
        if let Some(p) = &self.project {
            parts.push(format!("## project.md\n{p}"));
        }
        if let Some(r) = &self.roadmap {
            parts.push(format!("## roadmap.md\n{r}"));
        }
        if let Some(j) = &self.journal {
            let lines: Vec<&str> = j.lines().filter(|l| !l.trim().is_empty()).collect();
            parts.push(format!("## journal.md (latest entries)\n{}", lines[lines.len().saturating_sub(8)..].join("\n")));
        }
        parts.join("\n\n")
    }

    /// The instructions as one block, each under its file's name.
    pub fn instructions_text(&self) -> Option<String> {
        (!self.instructions.is_empty())
            .then(|| self.instructions.iter().map(|(name, text)| format!("### {name}\n{text}")).collect::<Vec<_>>().join("\n\n"))
    }
}

/// Reads the notes and the instruction files of `project`.
pub fn read(project: &Path) -> Snapshot {
    let dir = folder(project);
    let load = |name: &str| {
        let text = std::fs::read_to_string(dir.join(name)).ok()?;
        let text = text.trim();
        (!text.is_empty()).then(|| text.chars().take(READ_CAP).collect::<String>())
    };
    let instructions = INSTRUCTION_FILES
        .iter()
        .filter_map(|rel| {
            let text = std::fs::read_to_string(project.join(rel)).ok()?;
            let text = text.trim();
            if text.is_empty() {
                return None;
            }
            let capped = if text.chars().count() > INSTRUCTION_CAP {
                format!("{}\n… (cut — the whole file is {rel})", text.chars().take(INSTRUCTION_CAP).collect::<String>())
            } else {
                text.to_string()
            };
            Some((rel.to_string(), capped))
        })
        .collect();
    Snapshot { project: load(PROJECT), roadmap: load(ROADMAP), journal: load(JOURNAL), instructions }
}

/// Pure — unit-tested. The roadmap without Done, which only grows and is
/// rarely needed to start work.
pub fn working_sections(roadmap: &str, cap: usize) -> String {
    let mut out = Vec::new();
    let mut keep = false;
    for line in roadmap.lines() {
        if let Some(name) = line.strip_prefix("## ") {
            keep = name.trim().to_lowercase() != "done";
        }
        if keep {
            out.push(line);
        }
    }
    let text = out.join("\n").trim().to_string();
    if text.chars().count() > cap {
        format!("{}\n… (cut)", text.chars().take(cap).collect::<String>())
    } else {
        text
    }
}

/// Pure — unit-tested. The notes' part of the system prompt: loaded, plus how
/// to keep them current, or how to start them.
pub fn prompt_block(snapshot: &Snapshot) -> String {
    if !snapshot.exists() {
        return "\n\nThis project has no rak notes yet. If the person asks you to set up or explain the project, \
            create rak/project.md (what it is, who it is for, stack, how to build and test) and rak/roadmap.md with \
            sections ## Done, ## In progress, ## Next, ## Known issues; later sessions start from them."
            .to_string();
    }
    let mut block = "\n\nProject notes live in rak/: project.md, roadmap.md (Done / In progress / Next / Known issues) \
        and journal.md. They are loaded below — don't read them again unless you need a part not shown (Done is in \
        roadmap.md) or they changed. When you finish work that changes the project, update roadmap.md with edit_file \
        or apply_patch (move cards, add Next items) — never rewrite it whole."
        .to_string();
    if let Some(p) = &snapshot.project {
        block.push_str(&format!("\n\n## rak/project.md\n{p}"));
    }
    if let Some(r) = &snapshot.roadmap {
        block.push_str(&format!("\n\n## rak/roadmap.md — working sections\n{}", working_sections(r, 5_000)));
    }
    block
}

// MARK: - After a request: the journal line and roadmap edits

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Section {
    Done,
    InProgress,
    Next,
    KnownIssues,
}

impl Section {
    const ALL: [Section; 4] = [Section::Done, Section::InProgress, Section::Next, Section::KnownIssues];

    fn title(self) -> &'static str {
        match self {
            Section::Done => "Done",
            Section::InProgress => "In progress",
            Section::Next => "Next",
            Section::KnownIssues => "Known issues",
        }
    }

    fn named(name: &str) -> Option<Section> {
        let name = name.trim().to_lowercase();
        Self::ALL.into_iter().find(|s| s.title().to_lowercase() == name)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Edit {
    Move { card: String, to: Section },
    Add { to: Section, text: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Update {
    pub edits: Vec<Edit>,
    pub journal_line: String,
}

/// What a journal line records, from a fixed set so lines mean the same in
/// every project: `- 2026-10-20 06:30 [fix] — asked → outcome`.
const KINDS: [(&str, &str); 6] = [
    ("build", "new features, files or content"),
    ("fix", "repairing bugs or breakage"),
    ("review", "checking, auditing or testing, without new work"),
    ("ship", "committing, merging, pushing, releasing"),
    ("question", "advice, explanations or answers — nothing changed"),
    ("setup", "configuration, tooling, housekeeping"),
];

/// Pure — unit-tested. The prompt for the small after-request call.
pub fn update_prompt(snapshot: &Snapshot, request: &str, reply: &str, now: &str) -> String {
    let brief = snapshot.brief();
    let kinds: Vec<String> = KINDS.iter().map(|(k, m)| format!("{k} ({m})")).collect();
    format!(
        "You keep a software project's notes current. Below are the current notes, then what the person asked in the \
         last request and what the agent replied.\n\n{}\n\n## Last request\n{}\n\n## Final reply\n{}\n\n\
         Reply with EXACTLY two fenced blocks and nothing else. You do not rewrite the roadmap; you list edits and rak \
         applies them. Nothing can be deleted.\n\
         First block, tagged ```edits — one edit per line, or the single word NONE:\n  \
         MOVE \"<a card's text, exactly as it appears in the roadmap>\" TO <Done|In progress|Next|Known issues>\n  \
         ADD <Done|In progress|Next|Known issues>: <new card text>\n\
         Cards may carry `started:` and `done:` lines; rak writes those itself — never put them in an edit.\n\
         Move a card to Done only when the final reply says that work was finished (and checked, if it says how). If \
         the request stopped early — a turn limit, an error, a question to the person — move nothing to Done. Add \
         only what this request clearly revealed. Never invent details; if unsure, write NONE.\n\
         Second block, tagged ```journal: ONE line \"- {now} [<kind>] — <what was asked> → <what changed or was \
         decided>\", under 200 characters. If the request stopped early, say so plainly.\n\
         <kind> is exactly one of: {}. Pick by what the request did, not its topic.",
        if brief.is_empty() { "(no notes yet)".to_string() } else { brief },
        request.chars().take(1_500).collect::<String>(),
        reply.chars().take(3_000).collect::<String>(),
        kinds.join(", ")
    )
}

/// The text inside ```tag … ```.
fn fenced<'a>(text: &'a str, tag: &str) -> Option<&'a str> {
    let open = text.find(&format!("```{tag}"))? + 3 + tag.len();
    let rest = &text[open..];
    Some(&rest[..rest.find("```")?])
}

/// Pure — unit-tested. The call's reply as an update; None when the journal
/// line is missing — a bad reply must not touch good notes.
pub fn parse_update(reply: &str) -> Option<Update> {
    let edits = fenced(reply, "edits")?;
    let journal = fenced(reply, "journal")?;
    let line = journal.lines().map(str::trim).find(|l| l.starts_with('-'))?;
    Some(Update {
        edits: edits.lines().filter_map(parse_edit).collect(),
        journal_line: normalized_kind(&line.chars().take(220).collect::<String>()),
    })
}

fn parse_edit(raw: &str) -> Option<Edit> {
    let line = raw.trim();
    let upper = line.to_uppercase();
    if upper.starts_with("MOVE ") {
        let to = upper.rfind(" TO ")?;
        let card = line[5..to].trim().trim_matches(['"', '\'', '“', '”', ' ']).to_string();
        let section = Section::named(&line[to + 4..])?;
        return (!card.is_empty()).then_some(Edit::Move { card, to: section });
    }
    if upper.starts_with("ADD ") {
        let colon = line.find(':')?;
        let section = Section::named(&line[4..colon])?;
        let text = line[colon + 1..].trim().to_string();
        return (!text.is_empty()).then_some(Edit::Add { to: section, text });
    }
    None
}

/// A known kind is kept (lowercased); an unknown one is removed.
fn normalized_kind(line: &str) -> String {
    let (Some(open), Some(close)) = (line.find('['), line.find(']')) else { return line.to_string() };
    if open > 20 || close < open {
        return line.to_string();
    }
    let word = line[open + 1..close].trim().to_lowercase();
    if KINDS.iter().any(|(k, _)| *k == word) {
        format!("{}[{word}]{}", &line[..open], &line[close + 1..])
    } else {
        format!("{}{}", line[..open].trim_end(), &line[close + 1..])
    }
}

const PLACEHOLDERS: [&str; 6] = ["- none", "- (nothing)", "(nothing)", "- nothing", "- n/a", "- —"];

fn card_text(line: &str) -> Option<&str> {
    let t = line.trim();
    if let Some(rest) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
        return Some(rest);
    }
    let digits = t.chars().take_while(char::is_ascii_digit).count();
    (digits > 0 && (t[digits..].starts_with(". ") || t[digits..].starts_with(") "))).then(|| &t[digits + 2..])
}

/// A card's text for matching: case, Markdown marks and quotes ignored (a
/// model naming `add(a, b)` may drop the backticks).
fn norm(s: &str) -> String {
    let plain: String = s.to_lowercase().chars().filter(|c| !matches!(c, '`' | '*' | '"' | '“' | '”' | '\'')).collect();
    plain.split_whitespace().collect::<Vec<_>>().join(" ").trim_matches(['.', ' ', ':']).to_string()
}

/// How many edits were actually applied.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Applied {
    pub moved: usize,
    pub added: usize,
}

fn is_heading(line: &str) -> bool {
    line.starts_with("## ")
}

fn indent(line: &str) -> usize {
    line.chars().take_while(|c| *c == ' ' || *c == '\t').count()
}

fn field_key(line: &str) -> Option<String> {
    if indent(line) == 0 {
        return None;
    }
    let t = line.trim().strip_prefix("- ")?;
    Some(t[..t.find(':')?].trim().to_lowercase())
}

/// A card block after it lands in `section`: dated when work starts or ends.
fn stamped(mut block: Vec<String>, section: Section, day: &str) -> Vec<String> {
    match section {
        Section::InProgress => {
            block.retain(|l| field_key(l).as_deref() != Some("done"));
            if !block.iter().any(|l| field_key(l).as_deref() == Some("started")) {
                block.push(format!("  - started: {day}"));
            }
        }
        Section::Done => {
            block.retain(|l| field_key(l).as_deref() != Some("done"));
            block.push(format!("  - done: {day}"));
        }
        _ => {}
    }
    block
}

/// Pure — unit-tested. The roadmap with the edits applied: only moves and
/// additions; a card that can't be found (or matches twice) is skipped; an
/// emptied section gets "- None".
pub fn apply_edits(edits: &[Edit], roadmap: Option<&str>, day: &str) -> (String, Applied) {
    let mut applied = Applied::default();
    let mut lines: Vec<String> = roadmap.unwrap_or("# Roadmap\n").split('\n').map(String::from).collect();
    fn range(lines: &[String], s: Section) -> Option<(usize, usize)> {
        let h = lines.iter().position(|l| is_heading(l) && Section::named(&l[3..]) == Some(s))?;
        let end = lines[h + 1..].iter().position(|l| is_heading(l)).map(|e| h + 1 + e).unwrap_or(lines.len());
        Some((h + 1, end))
    }
    fn insert(lines: &mut Vec<String>, block: Vec<String>, s: Section) {
        if range(lines, s).is_none() {
            while lines.last().is_some_and(|l| l.is_empty()) {
                lines.pop();
            }
            lines.push(String::new());
            lines.push(format!("## {}", s.title()));
        }
        let (start, end) = range(lines, s).unwrap();
        if let Some(p) = (start..end).find(|&i| PLACEHOLDERS.contains(&lines[i].trim().to_lowercase().as_str())) {
            lines.remove(p);
        }
        let (start, end) = range(lines, s).unwrap();
        let mut at = end;
        while at > start && lines[at - 1].trim().is_empty() {
            at -= 1;
        }
        lines.splice(at..at, block);
    }
    for edit in edits {
        match edit {
            Edit::Add { to, text } => {
                let exists = lines.iter().any(|l| card_text(l).map(norm) == Some(norm(text)));
                if !exists {
                    insert(&mut lines, stamped(vec![format!("- {text}")], *to, day), *to);
                    applied.added += 1;
                }
            }
            Edit::Move { card, to } => {
                let want = norm(card);
                let cards: Vec<(usize, String)> = (0..lines.len())
                    .filter(|&i| indent(&lines[i]) == 0)
                    .filter_map(|i| card_text(&lines[i]).map(|t| (i, norm(t))))
                    .collect();
                // The same text; else (20 characters or more) the start of a
                // card, or one holding the other. Never a guess between two.
                let exact: Vec<usize> = cards.iter().filter(|(_, t)| *t == want).map(|(i, _)| *i).collect();
                let close: Vec<usize> = cards
                    .iter()
                    .filter(|(_, t)| {
                        want.chars().count() >= 20
                            && (t.starts_with(&want) || t.contains(&want) || (t.chars().count() >= 20 && want.contains(t.as_str())))
                    })
                    .map(|(i, _)| *i)
                    .collect();
                let hits = if exact.is_empty() { close } else { exact };
                let [i] = hits[..] else { continue };
                let Some(from) = Section::ALL.into_iter().find(|s| range(&lines, *s).is_some_and(|(a, b)| (a..b).contains(&i))) else {
                    continue;
                };
                if from == *to {
                    continue;
                }
                let mut end = i + 1;
                while end < lines.len() && !is_heading(&lines[end]) && indent(&lines[end]) > 0 {
                    end += 1;
                }
                let mut block: Vec<String> = lines.drain(i..end).collect();
                if let Some(text) = card_text(&block[0]) {
                    block[0] = format!("- {text}");
                }
                if let Some((a, b)) = range(&lines, from) {
                    if !(a..b).any(|k| card_text(&lines[k]).is_some()) {
                        let mut at = a;
                        while at < b && lines[at].trim().is_empty() {
                            at += 1;
                        }
                        // Only blank lines left: right under the heading.
                        if at == b {
                            at = a;
                        }
                        lines.insert(at, "- None".to_string());
                    }
                }
                insert(&mut lines, stamped(block, *to, day), *to);
                applied.moved += 1;
            }
        }
    }
    (lines.join("\n"), applied)
}

/// Writes the roadmap edits and the journal line; past the cap the oldest
/// journal lines move to journal-archive.md.
pub fn apply(update: &Update, project: &Path, day: &str) -> std::io::Result<Applied> {
    let dir = folder(project);
    std::fs::create_dir_all(&dir)?;
    let roadmap_file = dir.join(ROADMAP);
    let old = std::fs::read_to_string(&roadmap_file).ok();
    let mut applied = Applied::default();
    if !update.edits.is_empty() {
        let new;
        (new, applied) = apply_edits(&update.edits, old.as_deref(), day);
        if Some(&new) != old.as_ref() {
            std::fs::write(&roadmap_file, new)?;
        }
    }
    let journal_file = dir.join(JOURNAL);
    let text = std::fs::read_to_string(&journal_file).unwrap_or_else(|_| "# Journal\n".to_string());
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines.push(update.journal_line.clone());
    let entries: Vec<usize> = (0..lines.len()).filter(|&i| lines[i].starts_with("- ")).collect();
    if entries.len() > JOURNAL_CAP {
        let moving = &entries[..entries.len() - JOURNAL_CAP];
        let archive_file = dir.join(ARCHIVE);
        let mut archive = std::fs::read_to_string(&archive_file).unwrap_or_else(|_| "# Journal archive\n\n".to_string());
        if !archive.ends_with('\n') {
            archive.push('\n');
        }
        for &i in moving {
            archive.push_str(&lines[i]);
            archive.push('\n');
        }
        std::fs::write(&archive_file, archive)?;
        for &i in moving.iter().rev() {
            lines.remove(i);
        }
    }
    std::fs::write(&journal_file, lines.join("\n") + "\n")?;
    Ok(applied)
}

/// Pure — unit-tested. What an update did, for one line on screen: only
/// what was applied, and what was proposed but not found.
pub fn summary(update: &Update, applied: Applied) -> String {
    let mut parts = vec!["journal".to_string()];
    if applied.moved + applied.added > 0 {
        parts.push(format!("roadmap ({} moved, {} added)", applied.moved, applied.added));
    }
    let skipped = update.edits.len() - applied.moved - applied.added;
    let skipped = if skipped > 0 {
        format!("; {skipped} roadmap edit{} not applied (card not found or already there)", if skipped == 1 { "" } else { "s" })
    } else {
        String::new()
    };
    format!("notes updated: {}{skipped}", parts.join(", "))
}

// MARK: - Gate

/// A project's hand-over check: a `## Gate` section with a fenced command in
/// an instruction file. A request that edited files must run every part of
/// it after its last edit before it may call the work done (Naseem added it
/// after "all green" was claimed while the linter never ran).
#[derive(Debug, Clone, PartialEq)]
pub struct Gate {
    pub command: String,
    pub parts: Vec<String>,
    ran: Vec<bool>,
    edited: bool,
}

impl Gate {
    /// Pure — unit-tested. The first `## Gate` found in the instructions.
    pub fn from_instructions(instructions: &[(String, String)]) -> Option<Gate> {
        for (_, text) in instructions {
            let lines: Vec<&str> = text.lines().collect();
            let Some(h) = lines.iter().position(|l| l.trim().eq_ignore_ascii_case("## gate")) else { continue };
            let (mut body, mut in_fence) = (Vec::new(), false);
            for line in &lines[h + 1..] {
                let t = line.trim();
                if !in_fence && t.starts_with("## ") {
                    break;
                }
                if t.starts_with("```") {
                    if in_fence {
                        break;
                    }
                    in_fence = true;
                    continue;
                }
                if in_fence && !t.is_empty() && !t.starts_with('#') {
                    body.push(t.to_string());
                }
            }
            let command = body.join(" && ");
            let parts: Vec<String> =
                command.split("&&").flat_map(|p| p.split(';')).map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect();
            if !parts.is_empty() {
                let ran = vec![false; parts.len()];
                return Some(Gate { command, parts, ran, edited: false });
            }
        }
        None
    }

    /// A new request: nothing edited yet.
    pub fn reset(&mut self) {
        self.edited = false;
        self.ran.iter_mut().for_each(|r| *r = false);
    }

    pub fn note_edit(&mut self) {
        self.edited = true;
        self.ran.iter_mut().for_each(|r| *r = false);
    }

    pub fn note_shell(&mut self, command: &str) {
        let squeeze = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
        let command = squeeze(command);
        for (part, ran) in self.parts.iter().zip(self.ran.iter_mut()) {
            if command.contains(&squeeze(part)) {
                *ran = true;
            }
        }
    }

    /// The parts not run since the last edit; empty when satisfied.
    pub fn missing(&self) -> Vec<&str> {
        if !self.edited {
            return Vec::new();
        }
        self.parts.iter().zip(&self.ran).filter(|(_, ran)| !**ran).map(|(p, _)| p.as_str()).collect()
    }

    pub fn note(&self) -> String {
        let missing: Vec<String> = self.missing().iter().map(|p| format!("`{p}`")).collect();
        format!(
            "[rak] This project's rules (## Gate) say to run `{}` before handing over, and {} hasn't run since your last \
             edit. Run it now and report its real output — pass or fail.",
            self.command,
            missing.join(", ")
        )
    }
}

/// Now as "YYYY-MM-DD HH:MM" in local time (UTC where that is unknown).
pub fn now_local() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    #[cfg(unix)]
    {
        let t = secs as libc::time_t;
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        if !unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
            return format!("{:04}-{:02}-{:02} {:02}:{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min);
        }
    }
    let iso = crate::routines::iso(secs);
    format!("{} {}", &iso[..10], &iso[11..16])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::testing::temp_dir;

    const ROADMAP_TEXT: &str = "# Roadmap\n\n## Done\n- Login page\n\n## In progress\n- None\n\n## Next\n- Export to CSV\n  - done-when: a file opens in Excel\n- Dark mode\n\n## Known issues\n- Slow start on big folders";

    #[test]
    fn the_prompt_has_the_notes_without_done_or_says_how_to_start() {
        assert!(prompt_block(&Snapshot::default()).contains("create rak/project.md"));
        let snapshot = Snapshot { project: Some("A todo app.".into()), roadmap: Some(ROADMAP_TEXT.into()), ..Default::default() };
        let block = prompt_block(&snapshot);
        assert!(
            block.contains("## rak/project.md\nA todo app.") && block.contains("- Export to CSV") && !block.contains("Login page"),
            "{block}"
        );
    }

    #[test]
    fn edits_move_and_add_cards_and_never_delete() {
        let edits = [
            Edit::Move { card: "Export to CSV".into(), to: Section::Done },
            Edit::Move { card: "dark MODE.".into(), to: Section::InProgress },
            Edit::Add { to: Section::KnownIssues, text: "Crash on empty file".into() },
            Edit::Add { to: Section::Next, text: "Dark mode".into() },
            Edit::Move { card: "No such card".into(), to: Section::Done },
        ];
        let (out, applied) = apply_edits(&edits, Some(ROADMAP_TEXT), "2026-11-13");
        assert_eq!(applied, Applied { moved: 2, added: 1 }, "the missing card and the duplicate are not counted");
        assert_eq!(
            out,
            "# Roadmap\n\n## Done\n- Login page\n- Export to CSV\n  - done-when: a file opens in Excel\n  - done: 2026-11-13\n\n\
             ## In progress\n- Dark mode\n  - started: 2026-11-13\n\n## Next\n- None\n\n## Known issues\n- Slow start on big folders\n- Crash on empty file"
        );
        let (fresh, _) = apply_edits(&[Edit::Add { to: Section::Next, text: "First".into() }], None, "d");
        assert_eq!(fresh, "# Roadmap\n\n## Next\n- First");
    }

    #[test]
    fn a_card_is_found_without_its_backticks_but_never_guessed() {
        let roadmap =
            "## Known issues\n- `add(a, b)` in `calc.py` subtracts instead of adding: it returns `a - b` instead of `a + b`.\n- Slow start";
        let moved = [Edit::Move { card: "add(a, b) in calc.py subtracts instead of adding".into(), to: Section::Done }];
        let (out, applied) = apply_edits(&moved, Some(roadmap), "d");
        assert_eq!(applied.moved, 1, "{out}");
        assert!(out.contains("## Done\n- `add(a, b)` in `calc.py`"), "the card keeps its own text: {out}");
        let two = "## Next\n- Add unit tests for the parser\n- Add unit tests for the parser and lexer";
        let (_, applied) = apply_edits(&[Edit::Move { card: "unit tests for the parser".into(), to: Section::Done }], Some(two), "d");
        assert_eq!(applied.moved, 0, "two cards hold it: skipped");
    }

    #[test]
    fn a_reply_becomes_an_update_only_with_its_journal_line() {
        let reply = "```edits\nMOVE \"Export to CSV\" TO Done\nADD Known issues: Crash on empty file\nnonsense\n```\n\
                     ```journal\n- 2026-11-13 10:00 [FIX] — fix export → done, tests pass\n```";
        let update = parse_update(reply).unwrap();
        assert_eq!(update.edits.len(), 2);
        assert_eq!(update.journal_line, "- 2026-11-13 10:00 [fix] — fix export → done, tests pass");
        assert_eq!(summary(&update, Applied { moved: 1, added: 1 }), "notes updated: journal, roadmap (1 moved, 1 added)");
        assert_eq!(
            summary(&update, Applied { moved: 0, added: 1 }),
            "notes updated: journal, roadmap (0 moved, 1 added); 1 roadmap edit not applied (card not found or already there)"
        );
        assert!(parse_update("```edits\nNONE\n```").is_none(), "no journal: nothing touched");
        let odd = parse_update("```edits\nNONE\n```\n```journal\n- 2026-11-13 10:00 [party] — x → y\n```").unwrap();
        assert_eq!((odd.edits.len(), odd.journal_line.as_str()), (0, "- 2026-11-13 10:00 — x → y"));
    }

    #[test]
    fn the_journal_keeps_30_and_archives_the_rest() {
        let dir = temp_dir("notes");
        for i in 0..32 {
            let line = format!("- 2026-11-13 10:{i:02} [build] — step {i} → ok");
            apply(&Update { edits: vec![], journal_line: line }, &dir, "2026-11-13").unwrap();
        }
        let journal = std::fs::read_to_string(dir.join("rak/journal.md")).unwrap();
        let archive = std::fs::read_to_string(dir.join("rak/journal-archive.md")).unwrap();
        assert_eq!(journal.lines().filter(|l| l.starts_with("- ")).count(), 30);
        assert!(
            journal.starts_with("# Journal\n- 2026-11-13 10:02") && archive.contains("step 0") && archive.contains("step 1"),
            "{journal}"
        );
        assert!(!dir.join("rak/roadmap.md").exists(), "no edits, no roadmap written");
    }

    #[test]
    fn the_gate_must_run_after_the_last_edit() {
        let rules = "# Rules\nUse tabs.\n\n## Gate\n```\ncargo test\ncargo clippy -- -D warnings\n```\n## Other\n";
        let mut gate = Gate::from_instructions(&[("rak/rules.md".into(), rules.into())]).unwrap();
        assert_eq!(gate.parts, ["cargo test", "cargo clippy -- -D warnings"]);
        assert!(gate.missing().is_empty(), "nothing edited");
        gate.note_edit();
        gate.note_shell("cd x && cargo  test --quiet");
        assert_eq!(gate.missing(), ["cargo clippy -- -D warnings"]);
        assert!(gate.note().contains("`cargo clippy -- -D warnings` hasn't run"));
        gate.note_shell("cargo clippy -- -D warnings");
        assert!(gate.missing().is_empty());
        gate.note_edit();
        assert_eq!(gate.missing().len(), 2, "an edit makes both due again");
        assert!(Gate::from_instructions(&[("AGENTS.md".into(), "no gate here".into())]).is_none());
    }

    #[test]
    fn notes_and_instructions_are_read_from_the_project() {
        let dir = temp_dir("notes-read");
        std::fs::create_dir_all(dir.join("rak")).unwrap();
        std::fs::write(dir.join("rak/project.md"), "  A shop.  \n").unwrap();
        std::fs::write(dir.join("AGENTS.md"), "Run make.").unwrap();
        std::fs::write(dir.join("rak/rules.md"), "Be brief.").unwrap();
        let snapshot = read(&dir);
        assert_eq!(snapshot.project.as_deref(), Some("A shop."));
        assert_eq!(snapshot.instructions_text().unwrap(), "### rak/rules.md\nBe brief.\n\n### AGENTS.md\nRun make.");
        assert!(now_local().len() == 16);
    }
}
