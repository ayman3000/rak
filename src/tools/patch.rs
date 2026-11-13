//! apply_patch: edits a file with a unified diff (git / `diff -u`), several
//! hunks in one approved step. Ported from SwiftAgentKit's `UnifiedDiff` and
//! `PatchFileTool`.
//!
//! Hunks are placed by their lines, not their numbers: the `@@` line numbers
//! are hints, so a diff whose numbers drifted still applies. When exact
//! matching fails, trailing whitespace is ignored, then up to two context
//! lines at a hunk's edges may differ (as GNU patch's fuzz) — never a `-` or
//! `+` line. All or nothing: when any hunk fails, nothing is written and the
//! error shows the file's real lines near the spot.

use super::{str_arg, verify, Ctx, Tool, ToolResult};
use serde_json::{json, Value};
use std::sync::Arc;

/// One line of a hunk.
#[derive(Debug, Clone, PartialEq)]
pub enum Line {
    Context(String),
    Remove(String),
    Add(String),
}

/// One `@@ … @@` hunk.
#[derive(Debug, Clone, PartialEq)]
pub struct Hunk {
    /// The old file's first line, 1-based, from the header: a hint only.
    pub old_start: usize,
    pub lines: Vec<Line>,
}

impl Hunk {
    /// The lines the file must have (context and removed), in order.
    fn before(lines: &[Line]) -> Vec<&str> {
        lines
            .iter()
            .filter_map(|l| match l {
                Line::Context(s) | Line::Remove(s) => Some(s.as_str()),
                Line::Add(_) => None,
            })
            .collect()
    }

    fn after(&self) -> Vec<String> {
        self.lines
            .iter()
            .filter_map(|l| match l {
                Line::Context(s) | Line::Add(s) => Some(s.clone()),
                Line::Remove(_) => None,
            })
            .collect()
    }
}

/// Why a diff did not apply.
#[derive(Debug, PartialEq)]
pub enum ApplyError {
    /// Hunk `index` (0-based) was found nowhere; `nearby` is the file's
    /// numbered lines around its hint, `diagnosis` the line that broke it.
    NotFound { index: usize, preview: String, nearby: String, diagnosis: Option<String> },
    /// An insertion-only hunk whose line number is past the end.
    CannotAnchor { index: usize },
}

/// Pure — unit-tested. The hunks of a unified diff; file headers are
/// ignored (the tool's `path` names the file). None when there are none.
pub fn parse(patch: &str) -> Option<Vec<Hunk>> {
    let mut hunks = Vec::new();
    let mut current: Option<Hunk> = None;
    let mut raw: Vec<&str> = patch.split('\n').collect();
    while raw.last() == Some(&"") {
        raw.pop();
    }
    for line in raw {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.starts_with("@@") {
            if let Some(h) = current.take().filter(|h| !h.lines.is_empty()) {
                hunks.push(h);
            }
            current = Some(Hunk { old_start: old_start(line).unwrap_or(1), lines: Vec::new() });
            continue;
        }
        if line.starts_with("diff --git") || line.starts_with("index ") || line.starts_with("--- ") || line.starts_with("+++ ") {
            continue;
        }
        let Some(hunk) = current.as_mut() else { continue };
        if line.starts_with('\\') {
            continue; // "\ No newline at end of file"
        }
        let mut chars = line.chars();
        match chars.next() {
            None => hunk.lines.push(Line::Context(String::new())), // a bare blank context line
            Some('+') => hunk.lines.push(Line::Add(chars.as_str().to_string())),
            Some('-') => hunk.lines.push(Line::Remove(chars.as_str().to_string())),
            Some(' ') => hunk.lines.push(Line::Context(chars.as_str().to_string())),
            Some(_) => {}
        }
    }
    if let Some(h) = current.filter(|h| !h.lines.is_empty()) {
        hunks.push(h);
    }
    (!hunks.is_empty()).then_some(hunks)
}

/// Pure — unit-tested. The files a diff changes, from its `+++` headers
/// (`a/`/`b/` dropped, /dev/null skipped), in order and distinct.
pub fn targets(patch: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in patch.lines() {
        let Some(name) = line.strip_prefix("+++ ") else { continue };
        let name = name.split('\t').next().unwrap_or("").trim();
        if name.is_empty() || name == "/dev/null" {
            continue;
        }
        let name = name.strip_prefix("a/").or_else(|| name.strip_prefix("b/")).unwrap_or(name).to_string();
        if !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

/// "@@ -12,7 +12,8 @@" → 12.
fn old_start(header: &str) -> Option<usize> {
    let rest = &header[header.find('-')? + 1..];
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

fn rstrip(s: &str) -> &str {
    s.trim_end_matches([' ', '\t'])
}

/// Pure — unit-tested. The source with every hunk applied, or why not.
/// Context lines keep the file's own text, so a whitespace-tolerant match
/// never rewrites whitespace it did not touch.
pub fn apply(hunks: &[Hunk], source: &str) -> Result<String, ApplyError> {
    let trailing_newline = source.ends_with('\n');
    let mut lines: Vec<String> = source.split('\n').map(str::to_string).collect();
    if trailing_newline {
        lines.pop();
    }
    let mut offset: isize = 0;
    for (index, hunk) in hunks.iter().enumerate() {
        let hint = (hunk.old_start as isize - 1 + offset).max(0) as usize;
        if Hunk::before(&hunk.lines).is_empty() {
            let after = hunk.after();
            if hint > lines.len() {
                return Err(ApplyError::CannotAnchor { index });
            }
            offset += after.len() as isize;
            lines.splice(hint..hint, after);
            continue;
        }
        let Some((start, core)) = resolve(&hunk.lines, &lines, hint) else {
            let before = Hunk::before(&hunk.lines);
            return Err(ApplyError::NotFound {
                index,
                preview: before.iter().take(3).copied().collect::<Vec<_>>().join(" ⏎ ").chars().take(120).collect(),
                nearby: nearby(&lines, hint),
                diagnosis: diagnose(&hunk.lines, &lines, hint),
            });
        };
        let mut replacement = Vec::new();
        let (mut at, mut consumed) = (start, 0);
        for line in core {
            match line {
                Line::Context(_) => {
                    replacement.push(lines[at].clone());
                    at += 1;
                    consumed += 1;
                }
                Line::Remove(_) => {
                    at += 1;
                    consumed += 1;
                }
                Line::Add(s) => replacement.push(s.clone()),
            }
        }
        offset += replacement.len() as isize - consumed as isize;
        lines.splice(start..start + consumed, replacement);
    }
    let mut result = lines.join("\n");
    if trailing_newline {
        result.push('\n');
    }
    Ok(result)
}

/// Where a hunk applies: exact, then trailing whitespace ignored, then fuzz 1
/// and 2 (edge context lines dropped). The start and the lines that matched.
fn resolve<'a>(hunk: &'a [Line], lines: &[String], hint: usize) -> Option<(usize, &'a [Line])> {
    let before = Hunk::before(hunk);
    if let Some(at) = locate(&before, lines, hint, false).or_else(|| locate(&before, lines, hint, true)) {
        return Some((at, hunk));
    }
    let is_context = |l: &&Line| matches!(l, Line::Context(_));
    let lead = hunk.iter().take_while(is_context).count();
    let trail = hunk.iter().rev().take_while(is_context).count();
    for fuzz in 1..=2 {
        for l in 0..=fuzz.min(lead) {
            for t in 0..=fuzz.min(trail) {
                if l.max(t) != fuzz || l + t >= hunk.len() {
                    continue;
                }
                let core = &hunk[l..hunk.len() - t];
                let before = Hunk::before(core);
                if before.is_empty() {
                    continue;
                }
                if let Some(at) = locate(&before, lines, hint + l, false).or_else(|| locate(&before, lines, hint + l, true)) {
                    return Some((at, core));
                }
            }
        }
    }
    None
}

/// The start of `block` in `lines` nearest `hint`.
fn locate(block: &[&str], lines: &[String], hint: usize, tolerant: bool) -> Option<usize> {
    if block.is_empty() || block.len() > lines.len() {
        return None;
    }
    let same = |a: &str, b: &str| if tolerant { rstrip(a) == rstrip(b) } else { a == b };
    (0..=lines.len() - block.len())
        .filter(|&start| block.iter().enumerate().all(|(j, b)| same(&lines[start + j], b)))
        .min_by_key(|&start| start.abs_diff(hint))
}

/// The file's numbered lines around `hint`.
fn nearby(lines: &[String], hint: usize) -> String {
    if lines.is_empty() {
        return "(the file is empty)".to_string();
    }
    let center = hint.min(lines.len() - 1);
    let (lo, hi) = (center.saturating_sub(8), (center + 8).min(lines.len() - 1));
    (lo..=hi).map(|n| format!("{} | {}", n + 1, lines[n])).collect::<Vec<_>>().join("\n")
}

/// Which hunk line broke the match, and what the file has there. Also names
/// the usual trap: a file line that itself starts with `-` or `+` (a Markdown
/// bullet) sent without its own marker.
fn diagnose(hunk: &[Line], lines: &[String], hint: usize) -> Option<String> {
    let before = Hunk::before(hunk);
    if before.is_empty() {
        return None;
    }
    let (mut matched, mut start) = (0, 0);
    for k in (1..before.len()).rev() {
        if let Some(at) = locate(&before[..k], lines, hint, true) {
            (matched, start) = (k, at);
            break;
        }
    }
    let mut seen = 0;
    let (mut hunk_line, mut is_remove) = (0, false);
    for (n, line) in hunk.iter().enumerate() {
        if let Line::Context(_) | Line::Remove(_) = line {
            if seen == matched {
                (hunk_line, is_remove) = (n + 1, matches!(line, Line::Remove(_)));
                break;
            }
            seen += 1;
        }
    }
    let text = before[matched];
    let short: String = text.chars().take(100).collect();
    let mut message =
        format!("Hunk line {hunk_line} was read as {} \"{short}\", which isn't in the file", if is_remove { "REMOVE" } else { "CONTEXT" });
    if matched > 0 && start + matched < lines.len() {
        let there: String = lines[start + matched].chars().take(100).collect();
        message.push_str(&format!("; at that spot (file line {}) the file has \"{there}\"", start + matched + 1));
    }
    message.push('.');
    if let Some(marker) = ["-", "+"].into_iter().find(|m| lines.iter().any(|l| rstrip(l) == rstrip(&format!("{m}{text}")))) {
        let short: String = text.chars().take(80).collect();
        message.push_str(&format!(
            " The file has the line \"{marker}{short}\": a line that itself starts with \"{marker}\" still needs its own marker \
             — remove it with \"-{marker}…\", keep it with \" {marker}…\". For edits like this, edit_file is simpler."
        ));
    }
    Some(message)
}

/// What a diff adds and removes: (hunks, +lines, -lines).
pub fn counts(hunks: &[Hunk]) -> (usize, usize, usize) {
    let lines = hunks.iter().flat_map(|h| &h.lines);
    let added = lines.clone().filter(|l| matches!(l, Line::Add(_))).count();
    let removed = lines.filter(|l| matches!(l, Line::Remove(_))).count();
    (hunks.len(), added, removed)
}

pub struct ApplyPatch(pub Arc<Ctx>);

impl Tool for ApplyPatch {
    fn name(&self) -> &'static str {
        "apply_patch"
    }
    fn description(&self) -> &'static str {
        "Edit an existing file by applying a unified diff (git / `diff -u` format). One file per call: `path` is \
         that file; to change two files, call it twice. Provide the smallest diff that makes the change — one or \
         more `@@` hunks with a few lines of surrounding context; `-` lines are removed, `+` lines added. Line \
         numbers in `@@` headers may be approximate (matched by context, tolerant of whitespace drift). Use it for \
         changes in several places of one file; for one small change, and for Markdown lists, `edit_file` is \
         simpler. Every hunk line starts with its marker, even when the file's own line starts with `-` or `+`: \
         remove the bullet `- item` with `-- item`, keep it as context with ` - item`. If a hunk fails, the error \
         shows the file's current lines near the spot — regenerate the diff from those; don't rewrite the whole \
         file. Requires approval."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File to patch (a leading ~ is expanded)." },
                "patch": { "type": "string", "description": "A unified diff (git / diff -u). Text hunks only; no binary or rename." }
            },
            "required": ["path", "patch"]
        })
    }
    fn needs_approval(&self) -> bool {
        true
    }
    fn run(&self, args: &Value) -> ToolResult {
        let Some(raw) = str_arg(args, "path").filter(|s| !s.is_empty()) else {
            return ToolResult::err("apply_patch requires a `path`.");
        };
        let Some(patch) = str_arg(args, "patch").filter(|s| !s.is_empty()) else {
            return ToolResult::err("apply_patch requires a `patch` (a unified diff).");
        };
        let path = self.0.resolve(raw);
        if let Some(reason) = self.0.block_reason(&path) {
            return ToolResult::err(format!("Refusing to patch {raw}: {reason}."));
        }
        let files = targets(patch);
        if files.len() > 1 {
            return ToolResult::err(format!(
                "This patch changes {} files ({}); apply_patch takes one file per call. Nothing was changed. \
                 Send one apply_patch per file, each with that file's `path`.",
                files.len(),
                files.join(", ")
            ));
        }
        if path.is_dir() {
            let named = files.first().map(|f| format!(" (this diff names {f})")).unwrap_or_default();
            return ToolResult::err(format!("{raw} is a folder. `path` must be the file the diff changes{named}. Nothing was changed."));
        }
        let Ok(bytes) = std::fs::read(&path) else {
            return ToolResult::err(format!("Cannot read file to patch: {raw}. Use write_file to create a new file."));
        };
        let Ok(source) = String::from_utf8(bytes) else {
            return ToolResult::err(format!("Not a UTF-8 text file: {raw}"));
        };
        let Some(hunks) = parse(patch) else {
            return ToolResult::err("The `patch` isn't a valid unified diff (no @@ hunks found). Nothing was changed.");
        };
        let patched = match apply(&hunks, &source) {
            Ok(text) => text,
            Err(ApplyError::NotFound { index, preview, nearby, diagnosis }) => {
                let why =
                    diagnosis.map(|d| format!(" {d}")).unwrap_or_else(|| format!(" The surrounding lines weren't found: \"{preview}\"."));
                return ToolResult::err(format!(
                    "Hunk {} didn't match {raw}.{why} Nothing was changed. The file's CURRENT content near that spot is:\n\n\
                     {nearby}\n\nRegenerate the diff against these actual lines (do not rewrite the whole file).",
                    index + 1
                ));
            }
            Err(ApplyError::CannotAnchor { index }) => {
                return ToolResult::err(format!(
                    "Hunk {} is an insertion whose line number is past the end of {raw}. Add a line of context so it \
                     can be anchored. Nothing was changed.",
                    index + 1
                ));
            }
        };
        if let Some(rejection) = verify::check(&path, &patched) {
            return ToolResult::err(rejection.message(raw));
        }
        if let Err(e) = std::fs::write(&path, patched) {
            return ToolResult::err(format!("Write failed: {e}"));
        }
        self.0.note_seen(&path);
        let (n, added, removed) = counts(&hunks);
        ToolResult::ok(format!("Applied {n} hunk{} (+{added}/-{removed}) to {raw}.", if n == 1 { "" } else { "s" }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::testing::temp_dir;

    fn setup(label: &str) -> (std::path::PathBuf, Arc<Ctx>) {
        let dir = temp_dir(label);
        let ctx = Ctx::new(&dir, false);
        (dir, ctx)
    }

    fn patched(patch: &str, source: &str) -> Result<String, ApplyError> {
        apply(&parse(patch).expect("hunks"), source)
    }

    #[test]
    fn hunks_apply_by_their_lines_even_when_the_numbers_drifted() {
        let source = "a\nb\nc\nd\ne\nf\ng\n";
        // Numbers say line 40; the lines are at 3.
        let patch = "--- a/x\n+++ b/x\n@@ -40,3 +40,3 @@\n b\n-c\n+C\n d\n";
        assert_eq!(patched(patch, source).unwrap(), "a\nb\nC\nd\ne\nf\ng\n");
        // Two hunks; the second's place shifts by the first's added line.
        let two = "@@ -1,2 +1,3 @@\n a\n+a2\n b\n@@ -6,2 +7,2 @@\n f\n-g\n+G\n";
        assert_eq!(patched(two, source).unwrap(), "a\na2\nb\nc\nd\ne\nf\nG\n");
        // Insertion only, anchored by number.
        assert_eq!(patched("@@ -0,0 +1 @@\n+top\n", "x\n").unwrap(), "top\nx\n");
        // No trailing newline kept as it was.
        assert_eq!(patched("@@ -1 +1 @@\n-x\n+y\n", "x").unwrap(), "y");
    }

    #[test]
    fn whitespace_and_edge_context_may_drift_but_changes_may_not() {
        // Trailing spaces in the file; the file's context text is kept.
        let source = "fn a() {  \n    old();\n}\n";
        assert_eq!(patched("@@ -1,3 +1,3 @@\n fn a() {\n-    old();\n+    new();\n }\n", source).unwrap(), "fn a() {  \n    new();\n}\n");
        // An edge context line that is wrong: fuzz drops it.
        let source = "one\ntwo\nthree\nfour\n";
        assert_eq!(patched("@@ -1,3 +1,3 @@\n WRONG\n two\n-three\n+THREE\n", source).unwrap(), "one\ntwo\nTHREE\nfour\n");
        // A removed line that is not there: never fuzzed away.
        assert!(matches!(patched("@@ -1,2 +1,2 @@\n one\n-nope\n+x\n", source), Err(ApplyError::NotFound { index: 0, .. })));
    }

    #[test]
    fn a_failed_hunk_says_what_the_file_has_and_names_the_bullet_trap() {
        let source = "# Plan\n- item one\n- item two\n";
        // The bullet sent without its own marker: "-" read as "remove".
        let Err(ApplyError::NotFound { nearby, diagnosis, .. }) = patched("@@ -1,3 +1,3 @@\n # Plan\n- item one\n+- item 1\n", source)
        else {
            panic!("should not apply")
        };
        let diagnosis = diagnosis.unwrap();
        assert!(diagnosis.contains("REMOVE \" item one\"") && diagnosis.contains("its own marker"), "{diagnosis}");
        assert!(nearby.starts_with("1 | # Plan"), "{nearby}");
        // Sent right, it applies.
        assert_eq!(
            patched("@@ -1,3 +1,3 @@\n # Plan\n-- item one\n+- item 1\n - item two\n", source).unwrap(),
            "# Plan\n- item 1\n- item two\n"
        );
    }

    #[test]
    fn headers_name_the_files_and_junk_is_not_a_diff() {
        assert_eq!(targets("--- a/src/x.rs\n+++ b/src/x.rs\n@@ -1 +1 @@\n-a\n+b\n+++ b/y.rs\t2026\n"), ["src/x.rs", "y.rs"]);
        assert!(targets("+++ /dev/null\n").is_empty());
        assert_eq!(parse("just text\n-not a hunk\n"), None);
        assert_eq!(parse("@@ -3,1 +3,1 @@\r\n-a\r\n+b\r\n").unwrap()[0].lines, [Line::Remove("a".into()), Line::Add("b".into())]);
    }

    #[test]
    fn the_tool_writes_only_a_whole_good_result() {
        let (dir, ctx) = setup("patch");
        std::fs::write(dir.join("a.txt"), "x\ny\nz\n").unwrap();
        let tool = ApplyPatch(ctx);
        let ok = tool.run(&json!({ "path": "a.txt", "patch": "@@ -2 +2 @@\n-y\n+Y\n" }));
        assert!(ok.ok && ok.text == "Applied 1 hunk (+1/-1) to a.txt.", "{}", ok.text);
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "x\nY\nz\n");
        // First hunk fine, second not: nothing written.
        let bad = tool.run(&json!({ "path": "a.txt", "patch": "@@ -1 +1 @@\n-x\n+X\n@@ -3 +3 @@\n-nope\n+N\n" }));
        assert!(!bad.ok && bad.text.contains("Hunk 2 didn't match") && bad.text.contains("3 | z"), "{}", bad.text);
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "x\nY\nz\n");
        let two = tool.run(&json!({ "path": "a.txt", "patch": "+++ b/a.txt\n@@ -1 +1 @@\n-x\n+X\n+++ b/b.txt\n@@ -1 +1 @@\n-q\n+Q\n" }));
        assert!(!two.ok && two.text.contains("one file per call"), "{}", two.text);
        let missing = tool.run(&json!({ "path": "new.txt", "patch": "@@ -0,0 +1 @@\n+hi\n" }));
        assert!(!missing.ok && missing.text.contains("write_file"), "{}", missing.text);
    }

    #[test]
    fn a_patch_that_breaks_the_syntax_is_not_written() {
        let (dir, ctx) = setup("patch-json");
        std::fs::write(dir.join("p.json"), "{\n  \"a\": 1\n}\n").unwrap();
        let r = ApplyPatch(ctx).run(&json!({ "path": "p.json", "patch": "@@ -2 +2 @@\n-  \"a\": 1\n+  \"a\": 1,,\n" }));
        assert!(!r.ok, "{}", r.text);
        assert_eq!(std::fs::read_to_string(dir.join("p.json")).unwrap(), "{\n  \"a\": 1\n}\n");
    }
}
