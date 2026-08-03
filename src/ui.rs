//! Terminal presentation: colours and the warning box. Colours self-blank on
//! NO_COLOR / non-tty.

use std::io::IsTerminal;

/// ANSI colour strings, or all-empty when colour is disabled. Warning boxes are
/// coloured by severity level; notices (drought) use grey.
pub struct Colors {
    pub yellow: String, // BOLD + yellow — level 1
    pub orange: String, // BOLD + orange — level 2
    pub red: String,    // BOLD + red    — level 3
    pub grey: String,   // BOLD + grey   — notices
    pub reset: String,
}

impl Colors {
    /// Enabled unless `NO_COLOR` is set or stdout is not a terminal.
    pub fn detect() -> Self {
        let on = std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal();
        if on {
            Colors {
                yellow: "\x1b[1m\x1b[0;33m".into(),
                orange: "\x1b[1m\x1b[38;5;208m".into(),
                red: "\x1b[1m\x1b[0;31m".into(),
                grey: "\x1b[1m\x1b[1;30m".into(),
                reset: "\x1b[0m".into(),
            }
        } else {
            Colors::plain()
        }
    }

    /// All-empty colours, for non-coloured output (pipes, tests).
    pub fn plain() -> Self {
        Colors {
            yellow: String::new(),
            orange: String::new(),
            red: String::new(),
            grey: String::new(),
            reset: String::new(),
        }
    }

    /// The box colour for a warning severity level (3=red, 2=orange, 1=yellow).
    pub fn level(&self, level: i64) -> &str {
        match level {
            3 => &self.red,
            2 => &self.orange,
            1 => &self.yellow,
            _ => &self.red,
        }
    }
}

/// Draw a rectangle around `body`, with `caption` set into the top edge. Bars are in
/// `col`, content left default (reset after each bar). Auto-sizes to the widest line
/// (counted in chars) and returns the box as lines.
pub fn warn_box(col: &str, rst: &str, caption: &str, body: &[String]) -> Vec<String> {
    let title = format!("─ {caption} ");
    let title_len = title.chars().count();
    let mut w = 16usize;
    for l in body {
        let ln = l.chars().count() + 2;
        if ln > w {
            w = ln;
        }
    }
    let mut out = Vec::with_capacity(body.len() + 2);
    out.push(format!(
        "{col}┌{title}{}┐{rst}",
        "─".repeat(w.saturating_sub(title_len))
    ));
    for l in body {
        let pad = w.saturating_sub(2).saturating_sub(l.chars().count());
        out.push(format!("{col}│{rst} {l}{} {col}│{rst}", " ".repeat(pad)));
    }
    out.push(format!("{col}└{}┘{rst}", "─".repeat(w)));
    out
}

/// Word-wrap `text` to at most `width` characters per line, breaking on whitespace.
/// A word longer than `width` gets its own (over-long) line. Paragraphs (separated by a
/// blank line, `\n\n`) are preserved: each is wrapped independently and separated by one
/// empty line. Empty text -> no lines.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split("\n\n") {
        if paragraph.split_whitespace().next().is_none() {
            continue; // skip empty paragraphs
        }
        if !lines.is_empty() {
            lines.push(String::new()); // blank line between paragraphs
        }
        let mut cur = String::new();
        for word in paragraph.split_whitespace() {
            if cur.is_empty() {
                cur.push_str(word);
            } else if cur.chars().count() + 1 + word.chars().count() <= width {
                cur.push(' ');
                cur.push_str(word);
            } else {
                lines.push(std::mem::take(&mut cur));
                cur.push_str(word);
            }
        }
        if !cur.is_empty() {
            lines.push(cur);
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body() -> Vec<String> {
        vec!["hello".to_string(), "a longer line here".to_string()]
    }

    #[test]
    fn draws_four_lines_for_two_line_body() {
        let lines = warn_box("", "", "NOTICE", &body());
        assert_eq!(lines.len(), 4);
    }

    #[test]
    fn caption_set_into_top_edge() {
        let lines = warn_box("", "", "NOTICE", &body());
        assert!(lines[0].starts_with("┌─ NOTICE "));
        assert!(lines[0].ends_with('┐'));
    }

    #[test]
    fn body_lines_enclosed_and_preserved() {
        let lines = warn_box("", "", "NOTICE", &body());
        assert!(lines[1].starts_with("│ hello"));
        assert!(lines[2].contains("a longer line here"));
        assert!(lines[3].starts_with('└') && lines[3].ends_with('┘'));
    }

    #[test]
    fn all_rows_share_one_char_width() {
        let lines = warn_box("", "", "NOTICE", &body());
        let w = lines[0].chars().count();
        for l in &lines {
            assert_eq!(l.chars().count(), w);
        }
    }

    #[test]
    fn wrap_breaks_on_spaces_within_width() {
        assert!(wrap("", 10).is_empty());
        assert_eq!(wrap("one two three", 7), vec!["one two", "three"]);
        // a word longer than the width gets its own (over-long) line
        assert_eq!(wrap("aaaaaaaaaa bb", 5), vec!["aaaaaaaaaa", "bb"]);
        // wide chars counted by char, not byte
        assert_eq!(wrap("30°C do 33°C", 6), vec!["30°C", "do", "33°C"]);
    }

    #[test]
    fn wrap_preserves_paragraph_breaks() {
        // a blank line between paragraphs becomes one empty line, each wrapped alone
        assert_eq!(wrap("a b\n\nc d", 10), vec!["a b", "", "c d"]);
        assert_eq!(
            wrap("one two three\n\ndone", 7),
            vec!["one two", "three", "", "done"]
        );
    }
}
