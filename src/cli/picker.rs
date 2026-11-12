//! A list to pick from with the arrow keys: ↑/↓ move, Enter picks, Esc
//! cancels, and typing narrows the list. Used for /provider and /model. The
//! parts that decide what is shown are pure and tested; `pick` draws them.

use super::editor::{next_byte, read_key, Key};
use super::style::{cut, paint, text_width, Style};
use super::term;
use std::io::Write;

/// One row: the name picked, and a note shown after it.
pub struct Choice {
    pub name: String,
    pub note: String,
    /// Shown dim: there, but not usable as things stand (a provider with no key).
    pub dim: bool,
}

#[derive(Debug, PartialEq)]
pub enum Picked {
    /// The row at this index of the choices.
    Item(usize),
    /// A name typed that matches no row (only when `allow_typed`).
    Typed(String),
    Cancelled,
}

/// Pure — unit-tested. The rows whose name or note contains the filter,
/// ignoring case; all of them for an empty filter.
pub fn filtered(choices: &[Choice], filter: &str) -> Vec<usize> {
    let filter = filter.to_lowercase();
    (0..choices.len())
        .filter(|&i| {
            filter.is_empty() || choices[i].name.to_lowercase().contains(&filter) || choices[i].note.to_lowercase().contains(&filter)
        })
        .collect()
}

/// Pure — unit-tested. Where the visible window starts so `selected` is in it.
pub fn scrolled(selected: usize, offset: usize, height: usize) -> usize {
    if selected < offset {
        selected
    } else if selected >= offset + height {
        selected + 1 - height
    } else {
        offset
    }
}

/// Pure — unit-tested. The lines to draw: a hint, then the visible rows.
#[allow(clippy::too_many_arguments)]
pub fn lines(
    choices: &[Choice],
    shown: &[usize],
    selected: usize,
    offset: usize,
    height: usize,
    current: Option<usize>,
    filter: &str,
    allow_typed: bool,
    columns: usize,
    color: bool,
) -> Vec<String> {
    let dim = |text: &str| paint(color, Style::Dim, text);
    let mut hint = "↑/↓ move · Enter picks · Esc cancels · type to narrow".to_string();
    if !filter.is_empty() {
        hint = format!("↑/↓ move · Enter picks · Esc cancels · showing \"{filter}\" ({} of {})", shown.len(), choices.len());
    }
    let mut out = vec![dim(&cut(&hint, columns.saturating_sub(1)))];
    if shown.is_empty() {
        let none =
            if allow_typed { format!("nothing matches — Enter uses \"{filter}\" as typed") } else { "nothing matches".to_string() };
        out.push(dim(&format!("  {none}")));
        return out;
    }
    let name_width = shown.iter().map(|&i| text_width(&choices[i].name)).max().unwrap_or(0).min(32);
    for (position, &i) in shown.iter().enumerate().skip(offset).take(height) {
        let choice = &choices[i];
        let mark = if current == Some(i) { " ← current" } else { "" };
        let name = format!("{}{}", choice.name, " ".repeat(name_width.saturating_sub(text_width(&choice.name))));
        let row = cut(&format!("{name}  {}{mark}", choice.note), columns.saturating_sub(4));
        out.push(if position == selected {
            format!("{} {}", paint(color, Style::Cyan, "❯"), paint(color, Style::Bold, &row))
        } else if choice.dim {
            format!("  {}", dim(&row))
        } else {
            format!("  {row}")
        });
    }
    let below = shown.len().saturating_sub(offset + height);
    if offset > 0 || below > 0 {
        out.push(dim(&format!("  ({} more above, {below} more below)", offset)));
    }
    out
}

/// Lets the person pick one of `choices`, starting on `start`. Without a
/// terminal to draw on, falls back to a numbered list and a typed answer.
pub fn pick(choices: &[Choice], start: Option<usize>, allow_typed: bool, color: bool) -> Picked {
    let Some(raw) = term::Raw::enter() else { return pick_plain(choices, allow_typed) };
    let mut out = std::io::stdout();
    let mut filter = String::new();
    let mut selected = start.unwrap_or(0);
    let mut offset = 0;
    let mut drawn = 0;
    let result = loop {
        let shown = filtered(choices, &filter);
        selected = selected.min(shown.len().saturating_sub(1));
        let (columns, rows) = term::size();
        let height = rows.saturating_sub(5).clamp(3, 15);
        offset = scrolled(selected, offset, height);
        let frame = lines(choices, &shown, selected, offset, height, start, &filter, allow_typed, columns, color);
        // Back to where the last frame began, then draw over it.
        let mut bytes = if drawn > 0 { format!("\r\x1b[{drawn}A\x1b[J") } else { "\r\x1b[J".to_string() };
        bytes.push_str(&frame.join("\r\n"));
        bytes.push_str("\r\n");
        drawn = frame.len();
        let _ = out.write_all(bytes.as_bytes());
        let _ = out.flush();

        match read_key(&mut next_byte) {
            Key::Up => selected = selected.checked_sub(1).unwrap_or(shown.len().saturating_sub(1)),
            Key::Down => selected = if selected + 1 < shown.len() { selected + 1 } else { 0 },
            Key::Home => selected = 0,
            Key::End => selected = shown.len().saturating_sub(1),
            Key::Enter => match shown.get(selected) {
                Some(&i) => break Picked::Item(i),
                None if allow_typed && !filter.trim().is_empty() => break Picked::Typed(filter.trim().to_string()),
                None => {}
            },
            Key::Esc | Key::CtrlC | Key::CtrlD | Key::Eof => break Picked::Cancelled,
            Key::Backspace => {
                filter.pop();
                selected = 0;
            }
            Key::Char(c) => {
                filter.push(c);
                selected = 0;
            }
            _ => {}
        }
    };
    // The list goes; what was picked is said by whoever asked.
    let _ = out.write_all(format!("\r\x1b[{drawn}A\x1b[J").as_bytes());
    let _ = out.flush();
    drop(raw);
    result
}

/// No terminal to draw on (Windows, or input from a pipe): a numbered list.
fn pick_plain(choices: &[Choice], allow_typed: bool) -> Picked {
    for (i, choice) in choices.iter().enumerate() {
        println!("  {:>3}. {}  {}", i + 1, choice.name, choice.note);
    }
    print!("Pick a number{} (empty: cancel): ", if allow_typed { " or type a name" } else { "" });
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).unwrap_or(0) == 0 {
        return Picked::Cancelled;
    }
    let answer = line.trim();
    match answer.parse::<usize>() {
        _ if answer.is_empty() => Picked::Cancelled,
        Ok(n) if (1..=choices.len()).contains(&n) => Picked::Item(n - 1),
        _ => match choices.iter().position(|c| c.name == answer) {
            Some(i) => Picked::Item(i),
            None if allow_typed => Picked::Typed(answer.to_string()),
            None => Picked::Cancelled,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choices(names: &[&str]) -> Vec<Choice> {
        names.iter().map(|n| Choice { name: n.to_string(), note: String::new(), dim: false }).collect()
    }

    #[test]
    fn typing_narrows_the_list_by_name_or_note() {
        let mut list = choices(&["glm-5.2", "glm-5.3-flash", "kimi-k2.6", "GPT-OSS:120b"]);
        list[2].note = "Moonshot".into();
        assert_eq!(filtered(&list, ""), [0, 1, 2, 3]);
        assert_eq!(filtered(&list, "glm"), [0, 1]);
        assert_eq!(filtered(&list, "gpt"), [3], "case is ignored");
        assert_eq!(filtered(&list, "moon"), [2], "the note counts");
        assert!(filtered(&list, "zzz").is_empty());
    }

    #[test]
    fn the_window_follows_the_selection() {
        assert_eq!(scrolled(0, 0, 5), 0);
        assert_eq!(scrolled(4, 0, 5), 0);
        assert_eq!(scrolled(5, 0, 5), 1, "one past the bottom scrolls one row");
        assert_eq!(scrolled(2, 3, 5), 2, "above the top scrolls up to it");
    }

    #[test]
    fn the_frame_marks_the_selection_and_the_current_choice() {
        let list = choices(&["ollama", "anthropic", "zai"]);
        let shown = filtered(&list, "");
        let frame = lines(&list, &shown, 1, 0, 10, Some(2), "", false, 80, false);
        assert!(frame[0].contains("Esc cancels"));
        assert!(frame[2].starts_with("❯ anthropic"));
        assert!(frame[3].contains("zai") && frame[3].contains("← current"));
        assert_eq!(frame.len(), 4, "no scroll note when everything fits");

        let many = choices(&["a", "b", "c", "d", "e", "f"]);
        let frame = lines(&many, &filtered(&many, ""), 3, 2, 3, None, "", false, 80, false);
        assert_eq!(frame.last().unwrap(), "  (2 more above, 1 more below)");

        let frame = lines(&list, &[], 0, 0, 10, None, "claude-x", true, 80, false);
        assert!(frame[1].contains("Enter uses \"claude-x\""));
    }
}
