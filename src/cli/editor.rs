//! The prompt's line editor: arrow keys, history saved between runs, pasted
//! text kept whole, and the "/" menu drawn under the line.
//!
//! The pure parts (reading a key from bytes, the line being edited, what to
//! draw) are separate from the part that touches the terminal, and tested.

use super::menu::{self, Command};
use super::style::{char_width, paint, Style};
use super::term;
use std::io::Write;
use std::path::PathBuf;

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Key {
    Char(char),
    Enter,
    Tab,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    WordLeft,
    WordRight,
    Esc,
    CtrlC,
    CtrlD,
    KillToStart,
    KillToEnd,
    KillWord,
    PasteStart,
    PasteEnd,
    /// Something this editor has no use for.
    Other,
    /// The keyboard is gone.
    Eof,
}

/// Pure — unit-tested. Reads one key. `next(None)` waits for a byte;
/// `next(Some(ms))` gives up after that long — an arrow key's bytes arrive
/// together, a person's next key doesn't come that fast.
pub fn read_key(next: &mut dyn FnMut(Option<i32>) -> Option<u8>) -> Key {
    let Some(first) = next(None) else { return Key::Eof };
    match first {
        b'\r' | b'\n' => Key::Enter,
        b'\t' => Key::Tab,
        127 | 8 => Key::Backspace,
        1 => Key::Home,
        2 => Key::Left,
        3 => Key::CtrlC,
        4 => Key::CtrlD,
        5 => Key::End,
        6 => Key::Right,
        11 => Key::KillToEnd,
        14 => Key::Down,
        16 => Key::Up,
        21 => Key::KillToStart,
        23 => Key::KillWord,
        27 => after_escape(next),
        0..=31 => Key::Other,
        _ => {
            let extra = match first {
                b if b & 0xE0 == 0xC0 => 1,
                b if b & 0xF0 == 0xE0 => 2,
                b if b & 0xF8 == 0xF0 => 3,
                _ => 0,
            };
            let mut bytes = vec![first];
            for _ in 0..extra {
                match next(Some(50)) {
                    Some(b) => bytes.push(b),
                    None => return Key::Other,
                }
            }
            match std::str::from_utf8(&bytes).ok().and_then(|s| s.chars().next()) {
                Some(c) => Key::Char(c),
                None => Key::Other,
            }
        }
    }
}

fn after_escape(next: &mut dyn FnMut(Option<i32>) -> Option<u8>) -> Key {
    let Some(second) = next(Some(40)) else { return Key::Esc };
    let letter = |b: u8| match b {
        b'A' => Key::Up,
        b'B' => Key::Down,
        b'C' => Key::Right,
        b'D' => Key::Left,
        b'H' => Key::Home,
        b'F' => Key::End,
        _ => Key::Other,
    };
    match second {
        b'[' => {
            // ESC [ parameters final-byte
            let mut parameters = String::new();
            let last = loop {
                match next(Some(40)) {
                    Some(b @ 0x40..=0x7E) => break b,
                    Some(b) if parameters.len() < 16 => parameters.push(b as char),
                    _ => return Key::Other,
                }
            };
            match (parameters.as_str(), last) {
                ("", b) => letter(b),
                ("3", b'~') => Key::Delete,
                ("1" | "7", b'~') => Key::Home,
                ("4" | "8", b'~') => Key::End,
                ("200", b'~') => Key::PasteStart,
                ("201", b'~') => Key::PasteEnd,
                // Ctrl or Alt with an arrow: by word.
                (p, b'C') if p.starts_with("1;") => Key::WordRight,
                (p, b'D') if p.starts_with("1;") => Key::WordLeft,
                _ => Key::Other,
            }
        }
        b'O' => next(Some(40)).map(letter).unwrap_or(Key::Other),
        b'b' => Key::WordLeft,
        b'f' => Key::WordRight,
        127 => Key::KillWord,
        _ => Key::Other,
    }
}

/// The line being edited.
#[derive(Default, Debug, PartialEq)]
pub struct Buffer {
    pub chars: Vec<char>,
    pub cursor: usize,
}

impl Buffer {
    pub fn text(&self) -> String {
        self.chars.iter().collect()
    }
    pub fn set(&mut self, text: &str) {
        self.chars = text.chars().collect();
        self.cursor = self.chars.len();
    }
    pub fn at_end(&self) -> bool {
        self.cursor == self.chars.len()
    }
    pub fn insert(&mut self, c: char) {
        self.chars.insert(self.cursor, c);
        self.cursor += 1;
    }
    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.chars.remove(self.cursor);
        }
    }
    pub fn delete(&mut self) {
        if self.cursor < self.chars.len() {
            self.chars.remove(self.cursor);
        }
    }
    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }
    pub fn right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.chars.len());
    }
    fn word_start(&self) -> usize {
        let mut i = self.cursor;
        while i > 0 && self.chars[i - 1].is_whitespace() {
            i -= 1;
        }
        while i > 0 && !self.chars[i - 1].is_whitespace() {
            i -= 1;
        }
        i
    }
    pub fn word_left(&mut self) {
        self.cursor = self.word_start();
    }
    pub fn word_right(&mut self) {
        let mut i = self.cursor;
        while i < self.chars.len() && self.chars[i].is_whitespace() {
            i += 1;
        }
        while i < self.chars.len() && !self.chars[i].is_whitespace() {
            i += 1;
        }
        self.cursor = i;
    }
    pub fn kill_word(&mut self) {
        let start = self.word_start();
        self.chars.drain(start..self.cursor);
        self.cursor = start;
    }
    pub fn kill_to_start(&mut self) {
        self.chars.drain(..self.cursor);
        self.cursor = 0;
    }
    pub fn kill_to_end(&mut self) {
        self.chars.truncate(self.cursor);
    }
}

/// How a character of the line looks on screen: a pasted line break is shown
/// as ⏎, so the line stays one line while it is edited.
fn shown(c: char) -> char {
    match c {
        '\n' => '⏎',
        '\t' => ' ',
        c if c.is_control() => ' ',
        c => c,
    }
}

/// Pure — unit-tested. The bytes that redraw the prompt line, the menu rows
/// under it, and put the cursor where it belongs. `previous_row` is the
/// screen row the cursor was left on by the last draw, counted from the
/// prompt's first row. Returns the bytes and the row the cursor is on now.
///
/// Everything from the prompt down is cleared and written again; moves are
/// relative, so it works wherever on the screen the prompt is, and when the
/// screen scrolls to make room.
pub fn frame(prompt: &str, prompt_width: usize, buffer: &Buffer, bold: bool, menu_rows: &[String], cols: usize, previous_row: usize) -> (String, usize) {
    let cols = cols.max(2);
    let mut out = String::new();
    if previous_row > 0 {
        out.push_str(&format!("\x1b[{previous_row}A"));
    }
    out.push_str("\r\x1b[J");
    out.push_str(prompt);
    let text: String = buffer.chars.iter().map(|c| shown(*c)).collect();
    out.push_str(&if bold { format!("\x1b[1m{text}\x1b[0m") } else { text });

    let columns = |chars: &[char]| -> usize { chars.iter().map(|c| char_width(shown(*c))).sum() };
    let total = prompt_width + columns(&buffer.chars);
    // A line that ends exactly at the edge leaves the cursor waiting there;
    // move it to the next row so the rows below are counted the same way.
    if total > 0 && total.is_multiple_of(cols) {
        out.push_str("\r\n");
    }
    let mut end_row = total / cols;
    for row in menu_rows {
        out.push_str("\r\n");
        out.push_str(row);
        end_row += 1;
    }
    let at = prompt_width + columns(&buffer.chars[..buffer.cursor]);
    let (row, column) = (at / cols, at % cols);
    if end_row > row {
        out.push_str(&format!("\x1b[{}A", end_row - row));
    }
    out.push('\r');
    if column > 0 {
        out.push_str(&format!("\x1b[{column}C"));
    }
    (out, row)
}

/// The history file's form: one entry per line, line breaks written as \n.
pub fn encode_entry(entry: &str) -> String {
    entry.replace('\\', "\\\\").replace('\n', "\\n")
}

pub fn decode_entry(line: &str) -> String {
    let mut out = String::new();
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some(other) => out.push(other),
            None => out.push(c),
        }
    }
    out
}

const HISTORY_KEPT: usize = 1000;

pub struct Editor {
    history: Vec<String>,
    history_file: Option<PathBuf>,
    commands: Vec<Command>,
    color: bool,
}

impl Editor {
    pub fn new(history_file: Option<PathBuf>, commands: Vec<Command>, color: bool) -> Editor {
        let history = history_file
            .as_ref()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .map(|text| text.lines().filter(|l| !l.is_empty()).map(decode_entry).collect())
            .unwrap_or_default();
        Editor { history, history_file, commands, color }
    }

    fn remember(&mut self, line: &str) {
        if line.trim().is_empty() || self.history.last().map(String::as_str) == Some(line) {
            return;
        }
        self.history.push(line.to_string());
        if self.history.len() > HISTORY_KEPT {
            let extra = self.history.len() - HISTORY_KEPT;
            self.history.drain(..extra);
        }
        if let Some(file) = &self.history_file {
            if let Some(folder) = file.parent() {
                let _ = std::fs::create_dir_all(folder);
            }
            let text: String = self.history.iter().map(|e| encode_entry(e) + "\n").collect();
            let _ = std::fs::write(file, text);
        }
    }

    /// One line from the person. None at end of input (Ctrl-D on an empty
    /// line). `with_menu` shows the "/" menu; `plain_prompt` is the prompt's
    /// text, `styled_prompt` the same with colour.
    pub fn read_line(&mut self, plain_prompt: &str, style: Style, with_menu: bool) -> Option<String> {
        let prompt = paint(self.color, style, plain_prompt);
        let Some(raw) = term::Raw::enter() else {
            return self.read_plain(&prompt);
        };
        let prompt_width = plain_prompt.chars().map(char_width).sum();
        let mut out = std::io::stdout();
        let _ = out.write_all(b"\x1b[?2004h"); // pasted text arrives marked as a paste

        let mut buffer = Buffer::default();
        let mut row = 0;
        let mut selected = 0;
        let mut menu_for = String::new();
        // The line Esc closed the menu for; typing anything else brings it back.
        let mut dismissed: Option<String> = None;
        // ↑/↓ went into history: the line is a recalled one and the arrows
        // must keep going through history, so no menu until a key is typed.
        let mut browsing: Option<usize> = None;
        let mut draft = String::new();
        let mut pasting = false;

        let result = loop {
            let text = buffer.text();
            if text != menu_for {
                selected = 0;
                menu_for = text.clone();
            }
            if dismissed.as_deref() != Some(text.as_str()) {
                dismissed = None;
            }
            let matches = if with_menu && dismissed.is_none() && browsing.is_none() { menu::matches(&text, &self.commands) } else { Vec::new() };
            selected = selected.min(matches.len().saturating_sub(1));

            // While more input is already waiting (a paste), don't draw every step.
            let (cols, rows) = term::size();
            let waiting = pasting && peek();
            if !waiting {
                let menu_rows = menu::rows(&matches, selected, cols, rows.saturating_sub(4).clamp(3, 16), self.color);
                let (bytes, now) = frame(&prompt, prompt_width, &buffer, false, &menu_rows, cols, row);
                row = now;
                let _ = out.write_all(bytes.as_bytes());
                let _ = out.flush();
            }

            let key = read_key(&mut next_byte);
            if pasting {
                match key {
                    Key::PasteEnd => pasting = false,
                    Key::Enter => buffer.insert('\n'),
                    Key::Tab => (0..4).for_each(|_| buffer.insert(' ')),
                    Key::Char(c) => buffer.insert(c),
                    Key::Eof => break None,
                    _ => {}
                }
                continue;
            }
            let typing = !matches!(key, Key::Up | Key::Down | Key::Enter | Key::Other);
            match key {
                Key::Up if !matches.is_empty() => selected = menu::moved(selected, -1, matches.len()),
                Key::Down if !matches.is_empty() => selected = menu::moved(selected, 1, matches.len()),
                Key::Esc if !matches.is_empty() => dismissed = Some(text.clone()),
                Key::Up => {
                    let to = match browsing {
                        None if self.history.is_empty() => None,
                        None => {
                            draft = text.clone();
                            Some(self.history.len() - 1)
                        }
                        Some(i) => Some(i.saturating_sub(1)),
                    };
                    if let Some(i) = to {
                        browsing = Some(i);
                        buffer.set(&self.history[i]);
                    }
                }
                Key::Down => match browsing {
                    Some(i) if i + 1 < self.history.len() => {
                        browsing = Some(i + 1);
                        buffer.set(&self.history[i + 1]);
                    }
                    Some(_) => {
                        browsing = None;
                        buffer.set(&draft);
                        // Back on the person's own line: no menu jumps up at them.
                        dismissed = Some(draft.clone());
                    }
                    None => {}
                },
                Key::Tab => {
                    // The "/word" becomes the selected command (and a space,
                    // when the command takes arguments).
                    if let Some(command) = matches.get(selected).filter(|_| buffer.at_end()) {
                        let space = if command.arguments.is_empty() { "" } else { " " };
                        buffer.set(&format!("{}{space}", command.name));
                    }
                }
                Key::Enter => {
                    if let Some(command) = matches.get(selected).filter(|_| buffer.at_end()) {
                        buffer.set(&command.name);
                    }
                    break Some(buffer.text());
                }
                Key::CtrlD if buffer.chars.is_empty() => break None,
                Key::CtrlD | Key::Delete => buffer.delete(),
                Key::CtrlC => {
                    if buffer.chars.is_empty() {
                        let (bytes, _) = frame(&prompt, prompt_width, &buffer, false, &[], cols, row);
                        let hint = paint(self.color, Style::Dim, "(Ctrl-D or /exit to quit)");
                        let _ = out.write_all(format!("{bytes}\r\n{hint}\r\n").as_bytes());
                        row = 0;
                    } else {
                        buffer.set("");
                    }
                }
                Key::Char(c) => buffer.insert(c),
                Key::Backspace => buffer.backspace(),
                Key::Left => buffer.left(),
                Key::Right => buffer.right(),
                Key::Home => buffer.cursor = 0,
                Key::End => buffer.cursor = buffer.chars.len(),
                Key::WordLeft => buffer.word_left(),
                Key::WordRight => buffer.word_right(),
                Key::KillWord => buffer.kill_word(),
                Key::KillToStart => buffer.kill_to_start(),
                Key::KillToEnd => buffer.kill_to_end(),
                Key::PasteStart => pasting = true,
                Key::Eof => break None,
                Key::Esc | Key::PasteEnd | Key::Other => {}
            }
            if typing && browsing.is_some() && !matches!(key, Key::Left | Key::Right | Key::Home | Key::End | Key::WordLeft | Key::WordRight) {
                browsing = None; // an edited line is the person's own again
                dismissed = Some(buffer.text());
            }
        };

        // Leave the line as entered, in bold so it stands apart from the
        // answer under it, with the menu gone and the cursor below it.
        buffer.cursor = buffer.chars.len();
        let (cols, _) = term::size();
        let (bytes, _) = frame(&prompt, prompt_width, &buffer, self.color && result.is_some(), &[], cols, row);
        let _ = out.write_all(format!("{bytes}\r\n\x1b[?2004l").as_bytes());
        let _ = out.flush();
        drop(raw);

        if let Some(line) = &result {
            self.remember(line);
        }
        result
    }

    /// Without a terminal to drive (or on a system this editor doesn't know
    /// yet): the prompt, then a line read the ordinary way.
    fn read_plain(&mut self, prompt: &str) -> Option<String> {
        print!("{prompt}");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        match std::io::stdin().read_line(&mut line) {
            Ok(n) if n > 0 => Some(line.trim_end_matches(['\n', '\r']).to_string()),
            _ => None,
        }
    }
}

fn next_byte(wait_ms: Option<i32>) -> Option<u8> {
    PEEKED.with(|p| p.take()).or_else(|| term::read_byte(wait_ms))
}

thread_local! {
    /// A byte read while checking whether more input was waiting.
    static PEEKED: std::cell::Cell<Option<u8>> = const { std::cell::Cell::new(None) };
}

/// Whether another key is already waiting.
fn peek() -> bool {
    PEEKED.with(|p| {
        if p.get().is_none() {
            p.set(term::read_byte(Some(0)));
        }
        p.get().is_some()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Keys from a fixed list of bytes; a timed read past the end gives None.
    fn keys(bytes: &[u8]) -> Vec<Key> {
        let mut rest = bytes.to_vec();
        let mut next = move |_: Option<i32>| if rest.is_empty() { None } else { Some(rest.remove(0)) };
        let mut out = Vec::new();
        loop {
            match read_key(&mut next) {
                Key::Eof => return out,
                key => out.push(key),
            }
        }
    }

    #[test]
    fn letters_and_control_keys_are_read() {
        assert_eq!(keys(b"a/\r"), [Key::Char('a'), Key::Char('/'), Key::Enter]);
        assert_eq!(keys("é日🙂".as_bytes()), [Key::Char('é'), Key::Char('日'), Key::Char('🙂')]);
        assert_eq!(keys(&[127, 9, 3, 4, 21, 23, 11, 1, 5]), [
            Key::Backspace, Key::Tab, Key::CtrlC, Key::CtrlD, Key::KillToStart, Key::KillWord, Key::KillToEnd, Key::Home, Key::End
        ]);
    }

    #[test]
    fn escape_sequences_become_keys_and_a_lone_escape_stays_one() {
        assert_eq!(keys(b"\x1b[A\x1b[B\x1b[C\x1b[D"), [Key::Up, Key::Down, Key::Right, Key::Left]);
        assert_eq!(keys(b"\x1bOA\x1b[H\x1b[F\x1b[3~"), [Key::Up, Key::Home, Key::End, Key::Delete]);
        assert_eq!(keys(b"\x1b[1;5C\x1b[1;3D\x1bb\x1bf"), [Key::WordRight, Key::WordLeft, Key::WordLeft, Key::WordRight]);
        assert_eq!(keys(b"\x1b[200~x\x1b[201~"), [Key::PasteStart, Key::Char('x'), Key::PasteEnd]);
        assert_eq!(keys(b"\x1b"), [Key::Esc]);
        // A sequence this editor doesn't use is swallowed whole, not typed.
        assert_eq!(keys(b"\x1b[15~a"), [Key::Other, Key::Char('a')]);
    }

    #[test]
    fn the_line_is_edited_at_the_cursor() {
        let mut b = Buffer::default();
        b.set("fix the test");
        b.word_left();
        b.insert('X');
        assert_eq!(b.text(), "fix the Xtest");
        b.backspace();
        b.kill_word();
        assert_eq!((b.text().as_str(), b.cursor), ("fix test", 4));
        b.kill_to_end();
        assert_eq!(b.text(), "fix ");
        b.left();
        b.left();
        b.delete();
        assert_eq!(b.text(), "fi ");
        b.cursor = 0;
        b.word_right();
        assert_eq!(b.cursor, 2);
        b.kill_to_start();
        assert_eq!(b.text(), " ");
        b.backspace(); // at the start: nothing to remove
        assert_eq!(b.text(), " ");
    }

    #[test]
    fn a_short_line_is_drawn_with_the_cursor_after_it() {
        let mut b = Buffer::default();
        b.set("hi");
        let (bytes, row) = frame("> ", 2, &b, false, &[], 80, 0);
        assert_eq!(bytes, "\r\x1b[J> hi\r\x1b[4C");
        assert_eq!(row, 0);
        b.cursor = 0;
        assert_eq!(frame("> ", 2, &b, false, &[], 80, 0).0, "\r\x1b[J> hi\r\x1b[2C");
    }

    #[test]
    fn the_menu_goes_under_the_line_and_the_cursor_comes_back_up() {
        let mut b = Buffer::default();
        b.set("/m");
        let rows = vec!["one".to_string(), "two".to_string()];
        let (bytes, row) = frame("> ", 2, &b, false, &rows, 80, 0);
        assert_eq!(bytes, "\r\x1b[J> /m\r\none\r\ntwo\x1b[2A\r\x1b[4C");
        assert_eq!(row, 0);
    }

    #[test]
    fn a_wrapped_line_counts_its_rows() {
        let mut b = Buffer::default();
        b.set(&"x".repeat(25)); // with the prompt: 27 columns on a 10-column screen
        let (bytes, row) = frame("> ", 2, &b, false, &[], 10, 0);
        assert_eq!(row, 2);
        assert!(bytes.ends_with("\r\x1b[7C"));
        // The next draw starts by going back up those rows.
        assert!(frame("> ", 2, &b, false, &[], 10, row).0.starts_with("\x1b[2A\r\x1b[J"));
        // Cursor at the start of the text: up two rows from the end.
        b.cursor = 0;
        assert!(frame("> ", 2, &b, false, &[], 10, 0).0.ends_with("\x1b[2A\r\x1b[2C"));
        // Exactly full rows: the cursor sits at the start of the next one.
        b.set(&"x".repeat(18));
        let (bytes, row) = frame("> ", 2, &b, false, &[], 10, 0);
        assert_eq!(row, 2);
        assert!(bytes.ends_with("x\r\n\r"));
    }

    #[test]
    fn pasted_line_breaks_show_as_one_line_and_wide_characters_take_two_columns() {
        let mut b = Buffer::default();
        b.set("a\nb");
        assert!(frame("> ", 2, &b, false, &[], 80, 0).0.contains("> a⏎b\r\x1b[5C"));
        b.set("日本");
        assert!(frame("> ", 2, &b, false, &[], 80, 0).0.ends_with("\r\x1b[6C"));
    }

    #[test]
    fn history_entries_survive_the_file() {
        for entry in ["plain", "two\nlines", "a \\n that is not a line break", "back\\slash\n"] {
            let encoded = encode_entry(entry);
            assert!(!encoded.contains('\n'));
            assert_eq!(decode_entry(&encoded), entry);
        }
    }
}
