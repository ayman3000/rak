//! Colours, and text measured in terminal columns.

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Style {
    Dim,
    Bold,
    Cyan,
    Yellow,
    Red,
    Green,
    // 256-colour shades for the welcome panel and status rule: the logo's
    // cyan → blue → violet, and a muted teal for borders and labels.
    C51,
    C45,
    C39,
    C33,
    C63,
    C37,
}

impl Style {
    pub fn code(self) -> &'static str {
        match self {
            Style::Dim => "2",
            Style::Bold => "1",
            Style::Cyan => "36",
            Style::Yellow => "33",
            Style::Red => "31",
            Style::Green => "32",
            Style::C51 => "38;5;51",
            Style::C45 => "38;5;45",
            Style::C39 => "38;5;39",
            Style::C33 => "38;5;33",
            Style::C63 => "38;5;63",
            Style::C37 => "38;5;37",
        }
    }
}

/// `text` in `style` when `color` is on; as it is otherwise.
pub fn paint(color: bool, style: Style, text: &str) -> String {
    if color && !text.is_empty() {
        format!("\x1b[{}m{text}\x1b[0m", style.code())
    } else {
        text.to_string()
    }
}

/// A piece of text and how it's styled. Width is measured on the text alone.
#[derive(Clone, PartialEq, Debug)]
pub struct Span {
    pub text: String,
    pub styles: Vec<Style>,
}

pub fn span(text: impl Into<String>, styles: &[Style]) -> Span {
    Span { text: text.into(), styles: styles.to_vec() }
}

pub type Line = Vec<Span>;

pub fn width(line: &[Span]) -> usize {
    line.iter().map(|s| text_width(&s.text)).sum()
}

/// Exactly `width` columns: cut with an ellipsis, or padded with spaces.
pub fn fitted(line: &[Span], width: usize) -> Line {
    let mut out: Line = Vec::new();
    let mut left = width;
    for s in line {
        if left == 0 {
            break;
        }
        let w = text_width(&s.text);
        if w <= left {
            out.push(s.clone());
            left -= w;
        } else {
            // No room for the whole span: keep what fits, end with "…".
            let mut kept = String::new();
            let mut used = 0;
            for c in s.text.chars() {
                if used + char_width(c) > left - 1 {
                    break;
                }
                used += char_width(c);
                kept.push(c);
            }
            kept.push('…');
            left -= used + 1;
            out.push(Span { text: kept, styles: s.styles.clone() });
            if left > 0 {
                out.push(span(" ".repeat(left), &[]));
            }
            left = 0;
        }
    }
    if left > 0 {
        out.push(span(" ".repeat(left), &[]));
    }
    out
}

pub fn rendered(line: &[Span], color: bool) -> String {
    line.iter()
        .map(|s| {
            if !color || s.styles.is_empty() || s.text.is_empty() {
                return s.text.clone();
            }
            let codes: Vec<&str> = s.styles.iter().map(|st| st.code()).collect();
            format!("\x1b[{}m{}\x1b[0m", codes.join(";"), s.text)
        })
        .collect()
}

/// Columns a character takes: 0 for combining marks, 2 for wide East Asian
/// characters and emoji, 1 for the rest. A short table, not all of Unicode.
pub fn char_width(c: char) -> usize {
    let u = c as u32;
    let zero = matches!(u, 0 | 0x0300..=0x036F | 0x0610..=0x061A | 0x064B..=0x065F | 0x0670 | 0x06D6..=0x06DC
        | 0x200B..=0x200F | 0xFE00..=0xFE0F);
    let wide = matches!(u, 0x1100..=0x115F | 0x2E80..=0xA4CF | 0xAC00..=0xD7A3 | 0xF900..=0xFAFF | 0xFE30..=0xFE4F
        | 0xFF00..=0xFF60 | 0xFFE0..=0xFFE6 | 0x1F300..=0x1FAFF | 0x20000..=0x3FFFD);
    if zero {
        0
    } else if wide {
        2
    } else {
        1
    }
}

pub fn text_width(text: &str) -> usize {
    text.chars().map(char_width).sum()
}

/// `text` followed by spaces up to `width` columns.
pub fn pad(text: &str, width: usize) -> String {
    format!("{text}{}", " ".repeat(width.saturating_sub(text_width(text))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fitting_pads_short_lines_and_cuts_long_ones_to_the_exact_width() {
        let line = vec![span("model ", &[Style::Dim]), span("kimi-k2.6:cloud", &[Style::C45])];
        assert_eq!(rendered(&fitted(&line, 30), false), format!("model kimi-k2.6:cloud{}", " ".repeat(9)));
        assert_eq!(rendered(&fitted(&line, 12), false), "model kimi-…");
        for w in 1..40 {
            assert_eq!(width(&fitted(&line, w)), w);
        }
    }

    #[test]
    fn colour_is_only_added_when_asked_for() {
        let line = vec![span("a", &[Style::Bold, Style::C45]), span("b", &[])];
        assert_eq!(rendered(&line, false), "ab");
        assert_eq!(rendered(&line, true), "\x1b[1;38;5;45ma\x1b[0mb");
    }

    #[test]
    fn widths_count_columns_not_characters() {
        assert_eq!(text_width("abc"), 3);
        assert_eq!(text_width("日本"), 4);
        assert_eq!(text_width("e\u{301}"), 1);
        assert_eq!(text_width("❯ ⏎ │"), 5);
    }
}
