//! How much of a file one `read_file` call returns.
//!
//! The model picks `limit`, and left alone it picks small slices. A small
//! slice is not cheap: every tool call re-sends the whole conversation. So
//! `limit` is a hint. A slice is never smaller than `FLOOR`, and when the rest
//! of the file fits in one ordinary read, the rest is sent.

pub const FLOOR: usize = 4_000;
pub const DEFAULT_LIMIT: usize = 40_000;

#[derive(Debug, PartialEq)]
pub struct ReadSlice {
    pub start: usize,
    pub end: usize,
    pub widened_note: Option<String>,
    pub truncated_note: Option<String>,
}

impl ReadSlice {
    pub fn new(total_chars: usize, offset: Option<usize>, limit: Option<usize>) -> Self {
        let start = offset.unwrap_or(0).min(total_chars);
        let requested = limit.unwrap_or(DEFAULT_LIMIT);
        let remaining = total_chars - start;
        let effective = if remaining <= DEFAULT_LIMIT { remaining } else { requested.max(FLOOR) };
        let end = (start + effective).min(total_chars);
        let served = end - start;

        let widened_note = (served > requested).then(|| widen_reason(served, requested, remaining));
        let truncated_note =
            (end < total_chars).then(|| format!("… [truncated at {end}/{total_chars} chars — call again with offset {end}]"));
        ReadSlice { start, end, widened_note, truncated_note }
    }

    /// The slice, followed by whichever notes apply.
    pub fn annotate(&self, text: &str) -> String {
        let mut out = text.to_string();
        if let Some(n) = &self.truncated_note {
            out.push('\n');
            out.push_str(n);
        }
        if let Some(n) = &self.widened_note {
            out.push('\n');
            out.push_str(n);
        }
        out
    }
}

/// Names the number of calls the requested pace implies: that is the cost the
/// model is not counting.
fn widen_reason(served: usize, requested: usize, remaining: usize) -> String {
    let mut note = format!("[Returned {served} characters, not the {requested} requested.");
    if requested > 0 {
        let calls = remaining.div_ceil(requested);
        if calls > 2 {
            note.push_str(&format!(
                " At {requested} characters a call this file would take {calls} calls, and every call \
                 re-sends the whole conversation — reading more at once is far cheaper."
            ));
        }
    }
    note.push_str(" Ask for a larger `limit` next time.]");
    note
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_file_comes_back_whole_whatever_was_asked() {
        let s = ReadSlice::new(9_000, None, Some(200));
        assert_eq!((s.start, s.end), (0, 9_000));
        assert!(s.widened_note.as_deref().unwrap().contains("would take 45 calls"));
        assert!(s.truncated_note.is_none());
    }

    #[test]
    fn large_file_is_paged_but_never_below_the_floor() {
        let s = ReadSlice::new(500_000, Some(1_000), Some(50));
        assert_eq!((s.start, s.end), (1_000, 1_000 + FLOOR));
        assert!(s.truncated_note.as_deref().unwrap().contains("offset 5000"));
    }

    #[test]
    fn a_big_enough_request_is_served_as_asked() {
        let s = ReadSlice::new(500_000, None, Some(60_000));
        assert_eq!(s.end, 60_000);
        assert!(s.widened_note.is_none());
    }

    #[test]
    fn offset_past_the_end_is_empty() {
        let s = ReadSlice::new(100, Some(900), None);
        assert_eq!((s.start, s.end), (100, 100));
    }
}
