//! Light Markdown for the terminal: the model answers in Markdown whatever the
//! prompt says, so the chat shows it instead of the asterisks. Rendered line
//! by line while streaming, and only when stdout is a colour terminal — pipes,
//! files, --json and -q get the raw text.
//!
//! A table is the one thing that needs more than its own line: its rows are
//! held until the table ends, then drawn with aligned columns that fit the
//! terminal, long cells wrapping inside their column.

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
        // `$\rightarrow$`: a short formula of known symbols, as its characters.
        if c == '$' {
            if let Some(end) = find(i + 1, &['$']).filter(|&end| end > i + 1 && end - i <= 40) {
                let inner: String = chars[i + 1..end].iter().collect();
                if let Some(symbols) = math(&inner).filter(|_| !inner.contains('\n')) {
                    out.push_str(&symbols);
                    i = end + 1;
                    continue;
                }
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

/// Pure — unit-tested. A short LaTeX formula (what is between `$…$`) as
/// plain characters, when every command in it is a known symbol: `\rightarrow`
/// → `→`, `x \times y` → `x × y`, `90^\circ` → `90°`. None otherwise (prices
/// like "$5 and $10", formulas this can't draw), so it stays as written.
pub fn math(inner: &str) -> Option<String> {
    const SYMBOLS: &[(&str, &str)] = &[
        ("rightarrow", "→"),
        ("to", "→"),
        ("leftarrow", "←"),
        ("gets", "←"),
        ("leftrightarrow", "↔"),
        ("Rightarrow", "⇒"),
        ("implies", "⇒"),
        ("Leftarrow", "⇐"),
        ("Leftrightarrow", "⇔"),
        ("iff", "⇔"),
        ("uparrow", "↑"),
        ("downarrow", "↓"),
        ("mapsto", "↦"),
        ("times", "×"),
        ("cdot", "·"),
        ("div", "÷"),
        ("pm", "±"),
        ("le", "≤"),
        ("leq", "≤"),
        ("ge", "≥"),
        ("geq", "≥"),
        ("ne", "≠"),
        ("neq", "≠"),
        ("approx", "≈"),
        ("equiv", "≡"),
        ("sim", "∼"),
        ("infty", "∞"),
        ("in", "∈"),
        ("notin", "∉"),
        ("subset", "⊂"),
        ("cup", "∪"),
        ("cap", "∩"),
        ("forall", "∀"),
        ("exists", "∃"),
        ("neg", "¬"),
        ("land", "∧"),
        ("lor", "∨"),
        ("sum", "∑"),
        ("prod", "∏"),
        ("sqrt", "√"),
        ("partial", "∂"),
        ("nabla", "∇"),
        ("ldots", "…"),
        ("dots", "…"),
        ("cdots", "⋯"),
        ("checkmark", "✓"),
        ("circ", "∘"),
        ("degree", "°"),
        ("alpha", "α"),
        ("beta", "β"),
        ("gamma", "γ"),
        ("delta", "δ"),
        ("Delta", "Δ"),
        ("epsilon", "ε"),
        ("theta", "θ"),
        ("lambda", "λ"),
        ("mu", "μ"),
        ("pi", "π"),
        ("sigma", "σ"),
        ("Sigma", "Σ"),
        ("tau", "τ"),
        ("phi", "φ"),
        ("omega", "ω"),
        ("Omega", "Ω"),
    ];
    if !inner.contains('\\') {
        return None;
    }
    let inner = inner.replace("^{\\circ}", "°").replace("^\\circ", "°");
    let chars: Vec<char> = inner.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\\' => {
                let name: String = chars[i + 1..].iter().take_while(|c| c.is_ascii_alphabetic()).collect();
                if name.is_empty() {
                    // `\,` `\;` `\ ` are spaces; anything else is not ours to draw.
                    match chars.get(i + 1) {
                        Some(',' | ';' | ' ' | ':') => out.push(' '),
                        _ => return None,
                    }
                    i += 2;
                    continue;
                }
                out.push_str(SYMBOLS.iter().find(|(n, _)| *n == name)?.1);
                i += 1 + name.chars().count();
            }
            '{' | '}' => i += 1,
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    Some(out.trim().to_string())
}

/// A cell's alignment, from the separator row (`:--`, `:-:`, `--:`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// Pure — unit-tested. The cells of a `| a | b |` row, or None when the line
/// is not one. A `\|` stays in its cell.
pub fn table_cells(line: &str) -> Option<Vec<String>> {
    let trimmed = line.trim();
    if !trimmed.starts_with('|') || trimmed.len() < 2 {
        return None;
    }
    let inner = trimmed.strip_prefix('|')?;
    let inner = inner.strip_suffix('|').unwrap_or(inner);
    let mut cells = vec![String::new()];
    let mut chars = inner.chars().peekable();
    let mut in_code = false;
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                cells.last_mut().unwrap().push('|');
                chars.next();
            }
            '`' => {
                in_code = !in_code;
                cells.last_mut().unwrap().push(c);
            }
            '|' if !in_code => cells.push(String::new()),
            _ => cells.last_mut().unwrap().push(c),
        }
    }
    Some(cells.into_iter().map(|c| c.trim().to_string()).collect())
}

/// Pure — unit-tested. The alignments of a separator row (`|---|:-:|`), or
/// None when the line is not one.
pub fn separator(line: &str) -> Option<Vec<Align>> {
    let cells = table_cells(line)?;
    cells
        .iter()
        .map(|cell| {
            let dashes = cell.trim_matches(':');
            if dashes.is_empty() || !dashes.chars().all(|c| c == '-') {
                return None;
            }
            Some(match (cell.starts_with(':'), cell.ends_with(':')) {
                (true, true) => Align::Center,
                (false, true) => Align::Right,
                _ => Align::Left,
            })
        })
        .collect()
}

/// Columns a styled string takes on screen: escape codes count for nothing.
fn visible_width(text: &str) -> usize {
    let mut plain = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // ESC [ … letter
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            plain.push(c);
        }
    }
    super::style::text_width(&plain)
}

/// A cell's text broken into lines of at most `width` columns, at spaces
/// where it can; each line styled on its own.
fn wrap_cell(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split(' ').filter(|w| !w.is_empty()) {
        let candidate = if current.is_empty() { word.to_string() } else { format!("{current} {word}") };
        if visible_width(&inline(&candidate)) <= width {
            current = candidate;
            continue;
        }
        if !current.is_empty() {
            lines.push(std::mem::take(&mut current));
        }
        // A word longer than the column is cut into pieces.
        let mut piece = String::new();
        for c in word.chars() {
            if super::style::text_width(&piece) + super::style::text_width(&c.to_string()) > width && !piece.is_empty() {
                lines.push(std::mem::take(&mut piece));
            }
            piece.push(c);
        }
        current = piece;
    }
    if !current.is_empty() || lines.is_empty() {
        lines.push(current);
    }
    lines.iter().map(|l| inline(l)).collect()
}

/// Pure — unit-tested. A table drawn with box lines, at most `columns` wide:
/// the header bold, columns as wide as their content allows, the widest
/// narrowed first when they don't fit, long cells wrapped.
pub fn table(header: &[String], aligns: &[Align], rows: &[Vec<String>], columns: usize) -> Vec<String> {
    let count = header.len().max(aligns.len()).max(rows.iter().map(Vec::len).max().unwrap_or(0)).max(1);
    let cell = |row: &[String], i: usize| row.get(i).cloned().unwrap_or_default();
    let all: Vec<Vec<String>> = std::iter::once(header.to_vec()).chain(rows.iter().cloned()).collect();
    let mut widths: Vec<usize> =
        (0..count).map(|i| all.iter().map(|row| visible_width(&inline(&cell(row, i)))).max().unwrap_or(0).max(1)).collect();
    // Borders take "│ " before each column, " │" after the last, " │ " between.
    let frame = 3 * count + 1;
    let room = columns.saturating_sub(frame).max(count * 3);
    while widths.iter().sum::<usize>() > room {
        let widest = (0..count).max_by_key(|&i| widths[i]).unwrap_or(0);
        if widths[widest] <= 3 {
            break;
        }
        widths[widest] -= 1;
    }
    let dim = |text: String| format!("{}{text}{}", DIM.0, DIM.1);
    let rule = |left: &str, middle: &str, right: &str| {
        dim(format!("{left}{}{right}", widths.iter().map(|w| "─".repeat(w + 2)).collect::<Vec<_>>().join(middle)))
    };
    let draw_row = |row: &[String], bold: bool| -> Vec<String> {
        let wrapped: Vec<Vec<String>> = (0..count).map(|i| wrap_cell(&cell(row, i), widths[i])).collect();
        let height = wrapped.iter().map(Vec::len).max().unwrap_or(1);
        (0..height)
            .map(|line| {
                let mut out = dim("│".to_string());
                for (i, lines) in wrapped.iter().enumerate() {
                    let text = lines.get(line).cloned().unwrap_or_default();
                    let gap = widths[i].saturating_sub(visible_width(&text));
                    let (before, after) = match aligns.get(i).copied().unwrap_or(Align::Left) {
                        Align::Left => (0, gap),
                        Align::Right => (gap, 0),
                        Align::Center => (gap / 2, gap - gap / 2),
                    };
                    let text = if bold && !text.is_empty() { format!("{}{text}{}", BOLD.0, BOLD.1) } else { text };
                    out.push_str(&format!(" {}{text}{} ", " ".repeat(before), " ".repeat(after)));
                    out.push_str(&dim("│".to_string()));
                }
                out
            })
            .collect()
    };
    let mut out = vec![rule("┌", "┬", "┐")];
    out.extend(draw_row(header, true));
    out.push(rule("├", "┼", "┤"));
    // When rows wrap over several lines, a rule between them shows where
    // each one ends.
    let drawn: Vec<Vec<String>> = rows.iter().map(|row| draw_row(row, false)).collect();
    let wrapped = drawn.iter().any(|lines| lines.len() > 1);
    for (i, lines) in drawn.into_iter().enumerate() {
        if wrapped && i > 0 {
            out.push(rule("├", "┼", "┤"));
        }
        out.extend(lines);
    }
    out.push(rule("└", "┴", "┘"));
    out
}

/// Pure — unit-tested. Columns the lines after the first start at, so a
/// wrapped bullet or numbered item continues under its own text.
pub fn hang(line: &str) -> usize {
    let indent = line.chars().take_while(|c| *c == ' ').count();
    let body = &line[indent..];
    let digits = body.chars().take_while(char::is_ascii_digit).count();
    if body.starts_with("- ") || body.starts_with("* ") {
        indent + 2
    } else if digits > 0 && body[digits..].starts_with(". ") {
        indent + digits + 2
    } else {
        indent
    }
}

/// Pure — unit-tested. A styled line broken at spaces so no word is cut at
/// the terminal's edge; lines after the first start `hang` columns in. A
/// word wider than the terminal is left whole. Escape codes take no room, and
/// a style open at a break carries on to the next line.
pub fn wrap(styled: &str, columns: usize, hang: usize) -> String {
    if columns < 20 || visible_width(styled) <= columns {
        return styled.to_string();
    }
    let hang = hang.min(columns / 2);
    let mut out = String::with_capacity(styled.len() + 16);
    // Columns used on the current line, and whether it holds a word yet.
    let (mut used, mut has_word) = (0, false);
    for (i, word) in styled.split(' ').enumerate() {
        let width = visible_width(word);
        if has_word && width > 0 && used + 1 + width > columns {
            out.push('\n');
            out.extend(std::iter::repeat_n(' ', hang));
            (used, has_word) = (hang, false);
        } else if i > 0 {
            out.push(' ');
            used += 1;
        }
        out.push_str(word);
        used += width;
        has_word |= width > 0;
    }
    out
}

/// Feeds streamed pieces and gives back rendered text: whole lines as they
/// complete, the rest at `finish()`.
#[derive(Default)]
pub struct Renderer {
    buffer: String,
    state: State,
    /// Lines that may be a table, held until it is clear (or it ends).
    held: Vec<String>,
    /// The terminal's width for tables; 0 asks the terminal.
    pub columns: usize,
}

impl Renderer {
    pub fn push(&mut self, piece: &str) -> String {
        self.buffer.push_str(piece);
        let mut out = String::new();
        while let Some(newline) = self.buffer.find('\n') {
            let line: String = self.buffer.drain(..=newline).collect();
            out.push_str(&self.line(line.trim_end_matches(['\n', '\r'])));
        }
        out
    }

    /// The line still being held, without a line break. Empty when there is none.
    pub fn finish(&mut self) -> String {
        let rest = std::mem::take(&mut self.buffer);
        // The last row of a table at the very end has no line break yet.
        if !rest.is_empty() && !self.held.is_empty() && !self.state.in_code && table_cells(&rest).is_some() {
            self.held.push(rest);
            return self.flush().trim_end_matches('\n').to_string();
        }
        let mut out = self.flush();
        if !rest.is_empty() {
            out.push_str(&self.shown(&rest));
        }
        out.trim_end_matches('\n').to_string()
    }

    /// One whole line: held while it may belong to a table, else rendered.
    fn line(&mut self, line: &str) -> String {
        if !self.state.in_code && table_cells(line).is_some() {
            // A table needs its separator as the second line.
            let is_table = self.held.len() >= 2 || (self.held.len() == 1 && separator(line).is_some());
            if self.held.is_empty() || is_table {
                self.held.push(line.to_string());
                return String::new();
            }
        }
        let mut out = self.flush();
        out.push_str(&self.shown(line));
        out.push('\n');
        out
    }

    /// One line rendered and, unless it is code, wrapped at words to the
    /// terminal's width.
    fn shown(&mut self, line: &str) -> String {
        let code = self.state.in_code || line.trim_start().starts_with("```");
        let rendered = render(line, &mut self.state);
        if code {
            return rendered;
        }
        let columns = if self.columns > 0 { self.columns } else { super::term::size().0 };
        wrap(&rendered, columns.saturating_sub(1), hang(line))
    }

    /// What is held, drawn as a table when it is one, else line by line.
    fn flush(&mut self) -> String {
        let held = std::mem::take(&mut self.held);
        let mut out = String::new();
        let aligns = held.get(1).and_then(|l| separator(l));
        match (held.first().and_then(|l| table_cells(l)), aligns) {
            (Some(header), Some(aligns)) if held.len() >= 2 => {
                let rows: Vec<Vec<String>> = held[2..].iter().filter_map(|l| table_cells(l)).collect();
                let columns = if self.columns > 0 { self.columns } else { super::term::size().0 };
                for line in table(&header, &aligns, &rows, columns.saturating_sub(1)) {
                    out.push_str(&line);
                    out.push('\n');
                }
            }
            _ => {
                for line in &held {
                    out.push_str(&self.shown(line));
                    out.push('\n');
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(line: &str) -> String {
        render(line, &mut State::default())
    }

    #[test]
    fn short_latex_formulas_of_known_symbols_become_characters() {
        assert_eq!(inline(r"Start learning $\rightarrow$"), "Start learning →");
        assert_eq!(inline(r"size $n \times m$, angle $90^\circ$, $\alpha \le \beta$"), "size n × m, angle 90°, α ≤ β");
        assert_eq!(math(r"\Rightarrow").as_deref(), Some("⇒"));
        assert_eq!(math(r"a \, b").as_deref(), Some("a   b"));
        // Prices, unknown commands and code stay as written.
        assert_eq!(inline("costs $5 and $10"), "costs $5 and $10");
        assert_eq!(inline(r"$\frac{1}{2}$"), r"$\frac{1}{2}$");
        assert_eq!(math("x + y"), None, "no command: not a formula this draws");
        assert!(inline(r"`$\rightarrow$`").contains(r"$\rightarrow$"), "code is left alone");
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
    fn long_lines_break_at_words_and_list_items_continue_under_their_text() {
        let text = "The first thing you wrote here was \"does the app need any enhancements?\", and I can't see earlier sessions.";
        let wrapped = wrap(text, 40, 0);
        assert!(wrapped.lines().all(|l| l.chars().count() <= 40), "{wrapped}");
        assert_eq!(wrapped.replace('\n', " "), text, "nothing lost, no word cut");
        assert!(wrapped.lines().all(|l| !l.starts_with(' ') && !l.ends_with(' ')), "{wrapped:?}");
        assert_eq!(wrap("short line", 40, 0), "short line");
        // A bullet continues under its text; a numbered item too.
        assert_eq!((hang("- item"), hang("  * item"), hang("12. item"), hang("    plain"), hang("-dash")), (2, 4, 4, 4, 0));
        let mut r = Renderer { columns: 31, ..Renderer::default() };
        let out = strip(&r.push("- one two three four five six seven eight nine ten\n"));
        assert_eq!(out, "• one two three four five six\n  seven eight nine ten\n");
        // Styles take no room, and a word wider than the terminal stays whole.
        let out =
            strip(&r.push("**bold words** that go on and on past the edge https://example.com/a/very/long/address/that/does/not/fit\n"));
        assert_eq!(out, "bold words that go on and on\npast the edge\nhttps://example.com/a/very/long/address/that/does/not/fit\n");
        // Code is never re-wrapped.
        let code = format!("```\n{}\n```\n", "x ".repeat(40).trim_end());
        assert!(strip(&r.push(&code)).lines().nth(1).unwrap().len() > 60);
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

    #[test]
    fn table_rows_and_separators_are_read() {
        assert_eq!(table_cells(r"| a | `x|y` | c \| d |"), Some(vec!["a".into(), "`x|y`".into(), "c | d".into()]));
        assert_eq!(table_cells("not | a row"), None);
        assert_eq!(separator("|:---|:-:|--:|"), Some(vec![Align::Left, Align::Center, Align::Right]));
        assert_eq!(separator("| a | b |"), None);
    }

    fn strip(text: &str) -> String {
        let mut out = String::new();
        let mut chars = text.chars();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    #[test]
    fn a_streamed_table_is_drawn_with_aligned_columns() {
        let mut r = Renderer { columns: 80, ..Default::default() };
        let mut out = r.push("Before.\n| Name | Count |\n|---|--:|\n| apples | 3 |\n");
        assert_eq!(strip(&out), "Before.\n", "the table is held until it ends");
        out.push_str(&r.push("| `kiwi` | 12 |\nAfter.\n"));
        let lines: Vec<String> = strip(&out).lines().map(str::to_string).collect();
        assert_eq!(lines[1], "┌────────┬───────┐");
        assert_eq!(lines[2], "│ Name   │ Count │");
        assert_eq!(lines[4], "│ apples │     3 │");
        assert_eq!(lines[5], "│ kiwi   │    12 │", "code marks don't count toward the width; numbers right-aligned");
        assert_eq!(lines[6], "└────────┴───────┘");
        assert_eq!(lines[7], "After.");
    }

    #[test]
    fn a_table_at_the_end_comes_out_at_finish_and_a_lone_pipe_line_is_left_alone() {
        let mut r = Renderer { columns: 80, ..Default::default() };
        let mut out = r.push("| a | b |\n|---|---|\n| 1 | 2 |");
        out.push_str(&r.finish());
        assert!(strip(&out).ends_with("└───┴───┘"), "{out}");
        let mut r = Renderer { columns: 80, ..Default::default() };
        let mut out = r.push("| just a line\nnext\n");
        out.push_str(&r.finish());
        assert_eq!(strip(&out), "| just a line\nnext\n");
    }

    #[test]
    fn a_wide_table_fits_the_terminal_by_wrapping_its_widest_column() {
        let header = vec!["Tool".to_string(), "What it does".to_string()];
        let rows = vec![vec![
            "read_file".to_string(),
            "Reads a file, up to forty thousand characters in one call, or a range of lines.".to_string(),
        ]];
        let lines: Vec<String> = table(&header, &[Align::Left, Align::Left], &rows, 40).iter().map(|l| strip(l)).collect();
        assert!(lines.iter().all(|l| crate::cli::style::text_width(l) <= 40), "{lines:#?}");
        assert!(lines.len() > 5, "the long cell wraps over several lines: {lines:#?}");
        assert!(lines.iter().any(|l| l.starts_with("│ read_file │ Reads a file")), "{lines:#?}");
        let two = vec![rows[0].clone(), vec!["list_dir".into(), "Lists a folder.".into()]];
        let lines: Vec<String> = table(&header, &[Align::Left, Align::Left], &two, 40).iter().map(|l| strip(l)).collect();
        assert_eq!(lines.iter().filter(|l| l.starts_with('├')).count(), 2, "wrapped rows get a rule between them: {lines:#?}");
    }
}
