//! Light Markdown for the terminal: the model answers in Markdown whatever the
//! prompt says, so the chat shows it instead of the asterisks. Rendered line
//! by line while streaming, and only when stdout is a colour terminal — pipes,
//! files, --json and -q get the raw text.

const BOLD: (&str, &str) = ("\x1b[1m", "\x1b[22m");
const ITALIC: (&str, &str) = ("\x1b[3m", "\x1b[23m");
const CODE: (&str, &str) = ("\x1b[36m", "\x1b[39m");
const DIM: (&str, &str) = ("\x1b[2m", "\x1b[22m");

/// Streaming state: inside a ``` block, lines are shown as code.
#[derive(Default)]
pub struct State {
    pub in_code: bool,
}

/// Pure — unit-tested. One source line → one styled line.
pub fn render(line: &str, state: &mut State) -> String {
    let trimmed = line.trim();
    if let Some(label) = trimmed.strip_prefix("```") {
        state.in_code = !state.in_code;
        let label = label.trim();
        let text = if !state.in_code {
            "└".to_string()
        } else if label.is_empty() {
            "┌ code".to_string()
        } else {
            format!("┌ {label}")
        };
        return format!("{}{text}{}", DIM.0, DIM.1);
    }
    if state.in_code {
        return format!("{}│ {}{}{line}{}", DIM.0, DIM.1, CODE.0, CODE.1);
    }

    // Headings: bold, without the hashes.
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) {
        let rest = &line[hashes..];
        if rest.starts_with(char::is_whitespace) {
            return format!("{}{}{}", BOLD.0, inline(rest.trim_start()), BOLD.1);
        }
    }
    // Bullets: a real bullet, same indent.
    let indent_len = line.len() - line.trim_start().len();
    let (indent, body) = line.split_at(indent_len);
    for marker in ['-', '*'] {
        if let Some(rest) = body.strip_prefix(marker) {
            if rest.starts_with(char::is_whitespace) && !rest.trim().is_empty() {
                return format!("{indent}• {}", inline(rest.trim_start()));
            }
        }
    }
    // Numbered lists and plain lines: inline styles only.
    inline(line)
}

/// `**bold**`, `` `code` `` and `*emphasis*` at word edges. Underscores are
/// left alone — snake_case is everywhere in code talk — and so are "2 * 3"
/// and "a*b".
pub fn inline(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let find = |from: usize, what: &[char]| -> Option<usize> {
        (from..chars.len().saturating_sub(what.len() - 1)).find(|&j| chars[j..j + what.len()] == *what)
    };
    while i < chars.len() {
        let c = chars[i];
        if c == '`' {
            if let Some(end) = find(i + 1, &['`']).filter(|&end| end > i + 1) {
                out.push_str(CODE.0);
                out.extend(&chars[i + 1..end]);
                out.push_str(CODE.1);
                i = end + 1;
                continue;
            }
        }
        if c == '*' && chars.get(i + 1) == Some(&'*') {
            if let Some(end) = find(i + 2, &['*', '*']).filter(|&end| end > i + 2 && !chars[i + 2..end].contains(&'*')) {
                let inner: String = chars[i + 2..end].iter().collect();
                out.push_str(BOLD.0);
                out.push_str(&inline(&inner));
                out.push_str(BOLD.1);
                i = end + 2;
                continue;
            }
        }
        if c == '*' && (i == 0 || chars[i - 1].is_whitespace()) && chars.get(i + 1).is_some_and(|n| !n.is_whitespace() && *n != '*') {
            let closes = |end: usize| chars.get(end + 1).is_none_or(|n| n.is_whitespace() || ".,;:!?)".contains(*n));
            if let Some(end) = find(i + 1, &['*']).filter(|&end| closes(end)) {
                let inner: String = chars[i + 1..end].iter().collect();
                out.push_str(ITALIC.0);
                out.push_str(&inline(&inner));
                out.push_str(ITALIC.1);
                i = end + 1;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

/// Feeds streamed pieces and gives back rendered text: whole lines as they
/// complete, the rest at `finish()`.
#[derive(Default)]
pub struct Renderer {
    buffer: String,
    state: State,
}

impl Renderer {
    pub fn push(&mut self, piece: &str) -> String {
        self.buffer.push_str(piece);
        let mut out = String::new();
        while let Some(newline) = self.buffer.find('\n') {
            let line: String = self.buffer.drain(..=newline).collect();
            out.push_str(&render(line.trim_end_matches(['\n', '\r']), &mut self.state));
            out.push('\n');
        }
        out
    }

    /// The line still being held, without a line break. Empty when there is none.
    pub fn finish(&mut self) -> String {
        if self.buffer.is_empty() {
            return String::new();
        }
        let rest = std::mem::take(&mut self.buffer);
        render(&rest, &mut self.state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(line: &str) -> String {
        render(line, &mut State::default())
    }

    #[test]
    fn headings_lose_their_hashes_and_bullets_become_real_ones() {
        assert_eq!(plain("## What I did"), "\x1b[1mWhat I did\x1b[22m");
        assert_eq!(plain("- one"), "• one");
        assert_eq!(plain("  * two"), "  • two");
        assert_eq!(plain("#hashtag"), "#hashtag");
        assert_eq!(plain("1. first"), "1. first");
        assert_eq!(plain("-5 degrees"), "-5 degrees");
    }

    #[test]
    fn bold_code_and_emphasis_are_styled() {
        assert_eq!(inline("run `cargo test` now"), "run \x1b[36mcargo test\x1b[39m now");
        assert_eq!(inline("**Fixed** it"), "\x1b[1mFixed\x1b[22m it");
        assert_eq!(inline("an *important* point."), "an \x1b[3mimportant\x1b[23m point.");
        // Code inside bold keeps the bold after it ends.
        assert_eq!(inline("**`calc.py`**:"), "\x1b[1m\x1b[36mcalc.py\x1b[39m\x1b[22m:");
    }

    #[test]
    fn arithmetic_and_snake_case_are_left_alone() {
        for text in ["2 * 3 * 4", "a*b*c", "my_var_name and other_name", "**unclosed", "`unclosed", "5 ** 2"] {
            assert_eq!(inline(text), text);
        }
        // Stars inside code are code, not emphasis.
        assert_eq!(inline("`a * b * c`"), "\x1b[36ma * b * c\x1b[39m");
    }

    #[test]
    fn code_blocks_are_framed_and_not_styled_inside() {
        let mut state = State::default();
        assert_eq!(render("```python", &mut state), "\x1b[2m┌ python\x1b[22m");
        assert_eq!(render("x = **kwargs  # not bold", &mut state), "\x1b[2m│ \x1b[22m\x1b[36mx = **kwargs  # not bold\x1b[39m");
        assert_eq!(render("```", &mut state), "\x1b[2m└\x1b[22m");
        assert_eq!(render("**after**", &mut state), "\x1b[1mafter\x1b[22m");
    }

    #[test]
    fn streamed_pieces_come_out_as_whole_lines() {
        let mut r = Renderer::default();
        assert_eq!(r.push("# Ti"), "");
        assert_eq!(r.push("tle\n- on"), "\x1b[1mTitle\x1b[22m\n");
        assert_eq!(r.push("e\r\nlast"), "• one\n");
        assert_eq!(r.finish(), "last");
        assert_eq!(r.finish(), "");
    }
}
