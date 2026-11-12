//! The menu under the prompt when a line starts with "/": the commands that
//! match what's typed so far, narrowing with every key. ↑ and ↓ choose, Tab
//! completes, Enter runs, Esc closes. This file is the pure part; the editor
//! draws it.

use super::style::{fitted, rendered, span, width, Line, Style};

#[derive(Clone, PartialEq, Debug)]
pub struct Command {
    pub name: String,      // "/model"
    pub arguments: String, // "[name]"
    pub about: String,
}

/// Help rows ("/model [name]", "…") → menu commands.
pub fn commands(help: &[(&str, &str)]) -> Vec<Command> {
    help.iter()
        .map(|(usage, about)| {
            let (name, arguments) = usage.split_once(' ').unwrap_or((usage, ""));
            Command { name: name.to_string(), arguments: arguments.to_string(), about: about.to_string() }
        })
        .collect()
}

/// The commands to offer for what's typed. Empty unless the line is a "/word"
/// still being typed (no space yet). Names that start with the text come
/// first, then names that merely contain it.
pub fn matches(typed: &str, commands: &[Command]) -> Vec<Command> {
    if !typed.starts_with('/') || typed.contains(char::is_whitespace) {
        return Vec::new();
    }
    let needle = typed.to_lowercase();
    let mut out: Vec<Command> = commands.iter().filter(|c| c.name.starts_with(&needle)).cloned().collect();
    if needle.len() > 1 {
        let inner = &needle[1..];
        out.extend(commands.iter().filter(|c| !c.name.starts_with(&needle) && c.name[1..].contains(inner)).cloned());
    }
    out
}

/// What Enter runs: the line as typed when its command exists (`known` has
/// every name and alias), otherwise the first match's name in its place —
/// "/mo kimi" runs "/model kimi".
pub fn resolve(line: &str, commands: &[Command], known: &[&str]) -> String {
    if !line.starts_with('/') {
        return line.to_string();
    }
    let (name, rest) = match line.split_once(' ') {
        Some((name, rest)) => (name, Some(rest)),
        None => (line, None),
    };
    let lower = name.to_lowercase();
    if known.contains(&lower.as_str()) {
        return line.to_string();
    }
    match (matches(&lower, commands).first(), rest) {
        (Some(first), Some(rest)) => format!("{} {rest}", first.name),
        (Some(first), None) => first.name.clone(),
        (None, _) => line.to_string(),
    }
}

/// The menu's rows for a terminal `cols` wide, the `selected` match marked.
/// At most `max_rows`; when there are more matches, the rows shown move to
/// keep the selected one in view and the last row says how many aren't shown.
pub fn rows(matches: &[Command], selected: usize, cols: usize, max_rows: usize, color: bool) -> Vec<String> {
    if matches.is_empty() || cols <= 20 {
        return Vec::new();
    }
    let overflow = matches.len() > max_rows;
    let size = if overflow { max_rows.saturating_sub(1).max(1) } else { matches.len() };
    let start = (selected + 1).saturating_sub(size).min(matches.len() - size);
    let shown = &matches[start..start + size];
    let label = |c: &Command| c.name.chars().count() + if c.arguments.is_empty() { 0 } else { c.arguments.chars().count() + 1 };
    let name_width = shown.iter().map(label).max().unwrap_or(0);
    let mut out: Vec<String> = shown
        .iter()
        .enumerate()
        .map(|(i, command)| {
            let marked = start + i == selected;
            let mut line: Line = vec![
                span(if marked { "❯ " } else { "  " }, &[Style::C45, Style::Bold]),
                span(command.name.as_str(), if marked { &[Style::C45, Style::Bold] } else { &[Style::C45] }),
            ];
            if !command.arguments.is_empty() {
                line.push(span(format!(" {}", command.arguments), &[Style::Dim]));
            }
            line.push(span(" ".repeat((2 + name_width).saturating_sub(width(&line) - 2)), &[]));
            line.push(span(command.about.as_str(), if marked { &[] } else { &[Style::Dim] }));
            // One column short of the edge, so a full row never wraps.
            if width(&line) > cols - 1 {
                line = fitted(&line, cols - 1);
            }
            rendered(&line, color)
        })
        .collect();
    if overflow {
        out.push(rendered(&[span(format!("  … {} more — ↑↓ or keep typing", matches.len() - size), &[Style::Dim])], color));
    }
    out
}

/// The selection after an arrow key: down is +1, up is −1, wrapping.
pub fn moved(selected: usize, step: isize, count: usize) -> usize {
    if count == 0 {
        return 0;
    }
    (selected as isize + step).rem_euclid(count as isize) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::commands::{HELP, KNOWN};
    use crate::cli::style::text_width;

    fn all() -> Vec<Command> {
        commands(HELP)
    }

    fn names(list: &[Command]) -> Vec<&str> {
        list.iter().map(|c| c.name.as_str()).collect()
    }

    #[test]
    fn help_rows_become_commands() {
        let c = &all()[0];
        assert_eq!((c.name.as_str(), c.arguments.as_str()), ("/model", "[name]"));
        assert_eq!(all().iter().find(|c| c.name == "/tools").unwrap().arguments, "");
    }

    #[test]
    fn a_slash_offers_everything_and_typing_narrows_it() {
        assert_eq!(matches("/", &all()).len(), HELP.len());
        assert_eq!(names(&matches("/mo", &all())), ["/model", "/models"]);
        assert_eq!(names(&matches("/MO", &all())), ["/model", "/models"]);
        // Names that start with the text come before names that contain it.
        assert_eq!(
            names(&matches("/e", &all())),
            ["/exit", "/model", "/models", "/provider", "/resume", "/notes", "/yes", "/usage", "/clear", "/help"]
        );
    }

    #[test]
    fn no_menu_for_ordinary_text_or_once_arguments_begin() {
        assert!(matches("hello", &all()).is_empty());
        assert!(matches("/model kimi", &all()).is_empty());
        assert!(matches("/zzz", &all()).is_empty());
        assert!(matches("", &all()).is_empty());
    }

    #[test]
    fn enter_runs_the_first_match_but_leaves_known_names_alone() {
        let c = all();
        assert_eq!(resolve("/mo", &c, KNOWN), "/model");
        assert_eq!(resolve("/mo kimi", &c, KNOWN), "/model kimi");
        assert_eq!(resolve("/m", &c, KNOWN), "/m"); // an alias
        assert_eq!(resolve("/q", &c, KNOWN), "/q");
        assert_eq!(resolve("/zzz", &c, KNOWN), "/zzz");
        assert_eq!(resolve("fix /mo", &c, KNOWN), "fix /mo");
    }

    #[test]
    fn rows_mark_the_selection_and_never_reach_the_edge() {
        let m = matches("/", &all());
        for cols in [30, 60, 100] {
            let r = rows(&m, 1, cols, 16, false);
            assert_eq!(r.len(), m.len());
            assert!(r[1].starts_with("❯ /models"));
            assert!(r[0].starts_with("  /model"));
            assert!(r.iter().all(|row| text_width(row) < cols), "a row reached the edge at {cols}");
        }
        assert!(rows(&m, 0, 10, 16, false).is_empty());
        assert!(rows(&[], 0, 80, 16, false).is_empty());
    }

    #[test]
    fn a_long_list_scrolls_to_keep_the_selection_in_view() {
        let m = matches("/", &all());
        let r = rows(&m, 0, 80, 4, false);
        assert_eq!(r.len(), 4);
        assert!(r[3].contains(&format!("{} more", m.len() - 3)));
        let last = m.len() - 1;
        let r = rows(&m, last, 80, 4, false);
        assert!(r[2].starts_with(&format!("❯ {}", m[last].name)));
    }

    #[test]
    fn arrows_wrap_around() {
        assert_eq!(moved(0, -1, 5), 4);
        assert_eq!(moved(4, 1, 5), 0);
        assert_eq!(moved(2, 1, 5), 3);
        assert_eq!(moved(0, 1, 0), 0);
    }
}
