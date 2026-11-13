//! The chat's welcome panel and the status rule above each prompt.
//!
//! Everything here is pure: it takes what to show and a width, and returns
//! lines. main.rs gathers the facts and prints them; the tests check the
//! layout (every row exactly as wide as the box, at any width).

use super::style::{fitted, pad, rendered, span, text_width, width, Line, Span, Style};
use std::path::Path;

/// What the panel shows.
pub struct Info {
    pub version: String,
    pub model: String,
    /// Where the model is served from: "localhost:11434".
    pub server: String,
    /// Already shortened with ~.
    pub folder: String,
    pub branch: Option<String>,
    /// "asks before changes", "read-only", "auto mode"
    pub mode: String,
    /// Context window, tokens.
    pub window: usize,
    /// ("notes", "AGENTS.md")
    pub notes: Vec<(String, String)>,
    pub tool_groups: Vec<(String, Vec<String>)>,
    pub tool_count: usize,
    pub warnings: Vec<String>,
}

const R: [&str; 5] = ["██████ ", "██   ██", "██████ ", "██   ██", "██   ██"];
const A: [&str; 5] = [" █████ ", "██   ██", "███████", "██   ██", "██   ██"];
const K: [&str; 5] = ["██   ██", "██  ██ ", "█████  ", "██  ██ ", "██   ██"];
/// Top to bottom: cyan into blue into violet.
const LOGO_COLORS: [Style; 5] = [Style::C51, Style::C45, Style::C39, Style::C33, Style::C63];

pub fn logo() -> Vec<String> {
    (0..5).map(|i| format!("{} {} {}", R[i], A[i], K[i])).collect()
}

/// The panel, as lines exactly `width` wide (at least 44, at most 104).
pub fn panel(info: &Info, requested: usize, color: bool) -> Vec<String> {
    let total = requested.clamp(44, 104);
    let inner = total - 4; // "│ " … " │"
    let two_columns = inner >= 72;
    let left_width = if two_columns { 34 } else { inner };
    let right_width = if two_columns { inner - left_width - 3 } else { inner }; // " │ " between

    // Left: the wordmark and where you are.
    let mut left: Vec<Line> = logo().into_iter().zip(LOGO_COLORS).map(|(row, shade)| vec![span(row, &[shade, Style::Bold])]).collect();
    left.push(vec![span("a coding agent for your terminal", &[Style::Dim])]);
    left.push(Vec::new());
    let fact = |label: &str, value: &str, styles: &[Style]| -> Line { vec![span(pad(label, 8), &[Style::Dim]), span(value, styles)] };
    left.push(fact("model", &info.model, &[Style::C45]));
    left.push(fact("via", &info.server, &[]));
    left.push(fact("folder", &info.folder, &[]));
    if let Some(branch) = &info.branch {
        left.push(fact("branch", branch, &[]));
    }
    left.push(fact("mode", &info.mode, if info.mode.starts_with("auto") { &[Style::Yellow] } else { &[] }));
    left.push(fact("window", &format!("{} tokens", short(info.window)), &[]));
    for (label, value) in &info.notes {
        left.push(fact(label, value, &[]));
    }

    // Right: what the agent can use.
    let mut right: Vec<Line> = vec![vec![span("Tools", &[Style::Bold])]];
    let label_width = info.tool_groups.iter().map(|(label, _)| text_width(label)).max().unwrap_or(4) + 2;
    for (label, names) in &info.tool_groups {
        for (i, row) in wrap(names, right_width.saturating_sub(label_width), 2).into_iter().enumerate() {
            right.push(vec![span(pad(if i == 0 { label } else { "" }, label_width), &[Style::C37]), span(row, &[])]);
        }
    }
    for warning in &info.warnings {
        right.push(Vec::new());
        right.push(vec![span(format!("⚠ {warning}"), &[Style::Yellow])]);
    }

    let border = [Style::C37];
    let framed = |content: Line| -> String {
        let mut line = vec![span("│ ", &border)];
        line.extend(content);
        line.push(span(" │", &border));
        rendered(&line, color)
    };

    let mut lines = Vec::new();
    let title = format!(" rak {} ", info.version);
    let rule = "─".repeat((total - 3).saturating_sub(text_width(&title)));
    lines.push(rendered(&[span("╭─", &border), span(title, &[Style::Bold]), span(format!("{rule}╮"), &border)], color));

    if two_columns {
        for i in 0..left.len().max(right.len()) {
            let mut row = fitted(left.get(i).map(Vec::as_slice).unwrap_or(&[]), left_width);
            row.push(span(" │ ", &[Style::C37, Style::Dim]));
            row.extend(fitted(right.get(i).map(Vec::as_slice).unwrap_or(&[]), right_width));
            lines.push(framed(row));
        }
    } else {
        for l in &left {
            lines.push(framed(fitted(l, inner)));
        }
        lines.push(framed(fitted(&[], inner)));
        for r in &right {
            lines.push(framed(fitted(r, inner)));
        }
    }

    lines.push(framed(fitted(&[], inner)));
    let footer: Line = vec![
        span(format!("{} tool{}", info.tool_count, if info.tool_count == 1 { "" } else { "s" }), &[Style::C45]),
        span(
            if two_columns {
                " · /help for commands · Ctrl-C stops a request · Ctrl-D quits"
            } else {
                " · /help · Ctrl-C stops · Ctrl-D quits"
            },
            &[Style::Dim],
        ),
    ];
    lines.push(framed(fitted(&footer, inner)));
    lines.push(rendered(&[span(format!("╰{}╯", "─".repeat(total - 2)), &border)], color));
    lines
}

/// The rule above each prompt: model, how full the context is, where you are.
///   ── kimi-k2.6:cloud · ctx 7.2K/262K ▰▱▱▱▱▱▱▱▱▱ 3% · rak (main) · asks before changes ────
#[allow(clippy::too_many_arguments)]
pub fn status_rule(
    model: &str,
    used: usize,
    window: usize,
    folder: &str,
    branch: Option<&str>,
    mode: &str,
    requested: usize,
    color: bool,
) -> String {
    let total = requested.max(20);
    let fraction = if window > 0 { (used as f64 / window as f64).min(1.0) } else { 0.0 };
    let cells = 10usize;
    let filled = if used > 0 { ((fraction * cells as f64).round() as usize).max(1) } else { 0 };
    let bar_style = if fraction >= 0.85 {
        Style::Red
    } else if fraction >= 0.6 {
        Style::Yellow
    } else {
        Style::C45
    };
    let sep = || span(" · ", &[Style::Dim]);
    let place = match branch {
        Some(branch) => format!("{folder} ({branch})"),
        None => folder.to_string(),
    };
    let mut line: Line = vec![
        span("── ", &[Style::C37]),
        span(model, &[Style::C45, Style::Bold]),
        sep(),
        span(format!("ctx {}/{} ", short(used), short(window)), &[Style::Dim]),
        span("▰".repeat(filled), &[bar_style]),
        span("▱".repeat(cells - filled), &[Style::Dim]),
        span(format!(" {}%", (fraction * 100.0).round() as usize), &[Style::Dim]),
        sep(),
        span(place, &[Style::Dim]),
        sep(),
        span(mode, if mode.starts_with("auto") { &[Style::Yellow] } else { &[Style::Dim] }),
        span(" ", &[]),
    ];
    if width(&line) >= total {
        line = fitted(&line, total);
    } else {
        let rest = total - width(&line);
        line.push(Span { text: "─".repeat(rest), styles: vec![Style::C37] });
    }
    rendered(&line, color)
}

/// 7234 → "7.2K", 262144 → "262K", 1_000_000 → "1M".
pub fn short(n: usize) -> String {
    let tenths_text = |tenths: f64, unit: &str| {
        if tenths == tenths.round() {
            format!("{}{unit}", tenths as u64)
        } else {
            format!("{tenths:.1}{unit}")
        }
    };
    match n {
        0..=999 => n.to_string(),
        1_000..=9_999 => tenths_text((n as f64 / 100.0).round() / 10.0, "K"),
        10_000..=999_999 => format!("{}K", (n as f64 / 1000.0).round() as u64),
        _ => tenths_text((n as f64 / 100_000.0).round() / 10.0, "M"),
    }
}

/// 1234567 → "1,234,567"
pub fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Names joined with ", " into rows no wider than `width`; past `max_lines`
/// the rest become "+N more".
pub fn wrap(names: &[String], width: usize, max_lines: usize) -> Vec<String> {
    if names.is_empty() || width == 0 {
        return Vec::new();
    }
    let mut rows: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut index = 0;
    while index < names.len() {
        let name = &names[index];
        let remaining = names.len() - index - 1;
        let is_last_row = rows.len() == max_lines - 1;
        // On the last row, leave room to say how many are left. Other rows
        // keep one column for the comma that says "continues".
        let reserve = if !is_last_row {
            1
        } else if remaining > 0 {
            format!(", +{remaining} more").len()
        } else {
            0
        };
        let candidate = if current.is_empty() { name.clone() } else { format!("{current}, {name}") };
        if text_width(&candidate) + reserve <= width {
            current = candidate;
            index += 1;
        } else if is_last_row {
            break;
        } else if current.is_empty() {
            // One name wider than the row: it takes the row, cut by the caller.
            rows.push(name.clone());
            index += 1;
        } else {
            rows.push(std::mem::take(&mut current));
        }
    }
    let left = names.len() - index;
    if left > 0 {
        current = if current.is_empty() { format!("+{left} more") } else { format!("{current}, +{left} more") };
    }
    if !current.is_empty() {
        rows.push(current);
    }
    // A row that continues on the next ends with a comma.
    let count = rows.len();
    for row in rows.iter_mut().take(count.saturating_sub(1)) {
        if text_width(row) < width {
            row.push(',');
        }
    }
    rows
}

/// Groups tool names the way the panel shows them. MCP tools go under their
/// server, as a count: one server alone can bring dozens.
pub fn group(tools: &[&str], mcp_servers: &[(String, usize)]) -> Vec<(String, Vec<String>)> {
    let known: [(&str, &[&str]); 5] = [
        ("files", &["list_dir", "search_files", "code_map", "read_file", "view_image", "edit_file", "apply_patch", "write_file"]),
        ("shell", &["run_shell"]),
        ("plan", &["todo", "delegate"]),
        ("web", &["web_search", "fetch_url"]),
        ("skills", &["use_skill", "save_skill"]),
    ];
    let mut rest: Vec<&str> = tools.to_vec();
    let mut groups = Vec::new();
    for (label, names) in known {
        let present: Vec<String> = names.iter().filter(|n| rest.contains(n)).map(|n| n.to_string()).collect();
        if !present.is_empty() {
            rest.retain(|n| !present.iter().any(|p| p == n));
            groups.push((label.to_string(), present));
        }
    }
    if !rest.is_empty() {
        rest.sort();
        groups.push(("more".to_string(), rest.into_iter().map(str::to_string).collect()));
    }
    if !mcp_servers.is_empty() {
        let servers =
            mcp_servers.iter().map(|(name, count)| format!("{name} ({count} tool{})", if *count == 1 { "" } else { "s" })).collect();
        groups.push(("mcp".to_string(), servers));
    }
    groups
}

/// "/Users/me/projects/x" → "~/projects/x"; a long path keeps its ends.
pub fn short_path(path: &str, home: &str, max: usize) -> String {
    let p = if !home.is_empty() && path == home {
        "~".to_string()
    } else if !home.is_empty() && path.starts_with(&format!("{home}/")) {
        format!("~{}", &path[home.len()..])
    } else {
        path.to_string()
    };
    if p.chars().count() <= max {
        return p;
    }
    let parts: Vec<&str> = p.split('/').filter(|s| !s.is_empty()).collect();
    if parts.len() <= 2 {
        return p;
    }
    format!("{}/…/{}", if p.starts_with('~') { "~" } else { "" }, parts[parts.len() - 2..].join("/"))
}

/// The branch from `.git/HEAD`, without running git. None outside a repo or
/// on a detached HEAD.
pub fn git_branch(folder: &Path) -> Option<String> {
    let head = std::fs::read_to_string(folder.join(".git/HEAD")).ok()?;
    head.trim().strip_prefix("ref: refs/heads/").map(str::to_string)
}

/// "http://localhost:11434/v1" → "localhost:11434".
pub fn server_name(base_url: &str) -> String {
    let rest = base_url.split_once("://").map(|(_, rest)| rest).unwrap_or(base_url);
    rest.split('/').next().unwrap_or(rest).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> Info {
        Info {
            version: "0.1.0".into(),
            model: "kimi-k2.6:cloud".into(),
            server: "localhost:11434".into(),
            folder: "~/projects/rak".into(),
            branch: Some("main".into()),
            mode: "asks before changes".into(),
            window: 262_144,
            notes: vec![("notes".into(), "AGENTS.md".into())],
            tool_groups: group(&["list_dir", "search_files", "read_file", "write_file", "edit_file", "run_shell"], &[]),
            tool_count: 6,
            warnings: vec![],
        }
    }

    #[test]
    fn every_panel_row_is_exactly_as_wide_as_the_box() {
        for requested in [20, 44, 60, 79, 80, 100, 104, 200] {
            let lines = panel(&info(), requested, false);
            let expected = requested.clamp(44, 104);
            for line in &lines {
                assert_eq!(text_width(line), expected, "at {requested}: {line}");
            }
            assert!(lines[0].starts_with("╭─ rak 0.1.0 "));
            assert!(lines.last().unwrap().starts_with('╰'));
        }
    }

    #[test]
    fn the_panel_shows_the_facts_and_the_tools() {
        let text = panel(&info(), 100, false).join("\n");
        for needle in
            ["kimi-k2.6:cloud", "localhost:11434", "~/projects/rak", "main", "262K tokens", "AGENTS.md", "run_shell", "6 tools", "██████"]
        {
            assert!(text.contains(needle), "missing {needle}");
        }
        // Narrow: one column, tools under the facts.
        let narrow = panel(&info(), 50, false);
        let tools_at = narrow.iter().position(|l| l.contains("Tools")).unwrap();
        let model_at = narrow.iter().position(|l| l.contains("kimi")).unwrap();
        assert!(tools_at > model_at);
    }

    #[test]
    fn the_logo_rows_are_all_the_same_width() {
        assert!(logo().iter().all(|row| text_width(row) == 23));
    }

    #[test]
    fn the_status_rule_fills_the_width_and_shows_how_full_the_context_is() {
        for cols in [20, 50, 80, 140] {
            let rule = status_rule("kimi-k2.6:cloud", 7_234, 262_144, "rak", Some("main"), "asks before changes", cols, false);
            assert_eq!(text_width(&rule), cols);
        }
        let rule = status_rule("m", 7_234, 262_144, "rak", Some("main"), "auto mode", 120, false);
        assert!(rule.contains("ctx 7.2K/262K ▰▱▱▱▱▱▱▱▱▱ 3%"));
        assert!(rule.contains("rak (main) · auto mode"));
        assert!(status_rule("m", 0, 128_000, "rak", None, "read-only", 120, false).contains("▱▱▱▱▱▱▱▱▱▱ 0% · rak · read-only"));
        assert!(status_rule("m", 500_000, 128_000, "rak", None, "x", 120, false).contains("▰▰▰▰▰▰▰▰▰▰ 100%"));
    }

    #[test]
    fn numbers_are_shortened() {
        let cases = [
            (0, "0"),
            (999, "999"),
            (1_000, "1K"),
            (7_234, "7.2K"),
            (9_960, "10K"),
            (128_000, "128K"),
            (262_144, "262K"),
            (1_000_000, "1M"),
            (1_250_000, "1.3M"),
        ];
        for (n, text) in cases {
            assert_eq!(short(n), text);
        }
    }

    #[test]
    fn names_wrap_and_the_overflow_is_counted() {
        let names: Vec<String> =
            ["list_dir", "search_files", "read_file", "edit_file", "write_file"].iter().map(|s| s.to_string()).collect();
        assert_eq!(wrap(&names, 80, 2), ["list_dir, search_files, read_file, edit_file, write_file"]);
        assert_eq!(wrap(&names, 34, 2), ["list_dir, search_files, read_file,", "edit_file, write_file"]);
        let rows = wrap(&names, 22, 2);
        assert_eq!(rows.len(), 2);
        assert!(rows[1].ends_with("more") && rows.iter().all(|r| text_width(r) <= 22), "{rows:?}");
        assert!(wrap(&[], 80, 2).is_empty());
    }

    #[test]
    fn tools_are_grouped_and_unknown_ones_kept() {
        let groups = group(&["run_shell", "read_file", "zap", "list_dir"], &[("docs".to_string(), 1), ("browser".to_string(), 26)]);
        assert_eq!(groups[0], ("files".to_string(), vec!["list_dir".to_string(), "read_file".to_string()]));
        assert_eq!(groups[1].0, "shell");
        assert_eq!(groups[2], ("more".to_string(), vec!["zap".to_string()]));
        assert_eq!(groups[3], ("mcp".to_string(), vec!["docs (1 tool)".to_string(), "browser (26 tools)".to_string()]));
    }

    #[test]
    fn paths_and_servers_are_shortened() {
        assert_eq!(short_path("/Users/me", "/Users/me", 40), "~");
        assert_eq!(short_path("/Users/me/projects/x", "/Users/me", 40), "~/projects/x");
        assert_eq!(short_path("/Users/me/a/very/long/path/to/the/project", "/Users/me", 20), "~/…/the/project");
        assert_eq!(short_path("/opt/x", "/Users/me", 40), "/opt/x");
        assert_eq!(server_name("http://localhost:11434/v1"), "localhost:11434");
        assert_eq!(server_name("https://openrouter.ai/api/v1"), "openrouter.ai");
    }

    #[test]
    fn numbers_are_grouped_in_threes() {
        for (n, text) in [(0, "0"), (999, "999"), (1_000, "1,000"), (12_345, "12,345"), (1_234_567, "1,234,567")] {
            assert_eq!(grouped(n), text);
        }
    }
}
