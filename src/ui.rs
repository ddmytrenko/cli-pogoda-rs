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

/// The terminal's width in columns, or `None` when stdout is not a terminal — piped or
/// redirected output is left unwrapped, since there is no window to fit. Queried afresh
/// on every run, so a resized window is honoured the next time the command is run.
pub fn term_width() -> Option<usize> {
    terminal_size::terminal_size().map(|(terminal_size::Width(w), _)| w as usize)
}

/// Shorten `s` to at most `width` chars, marking the cut with an ellipsis. Only used for
/// box captions, which are labels; body text is wrapped (never cut) instead.
fn ellipsize(s: &str, width: usize) -> String {
    if s.chars().count() <= width {
        return s.to_string();
    }
    match width {
        0 => String::new(),
        w => s.chars().take(w - 1).chain(['…']).collect(),
    }
}

/// Draw a rectangle around `body`, with `caption` set into the top edge. Bars are in
/// `col`, content left default (reset after each bar). Renders at `width` inner columns
/// (box total = `width` + 2). A caption too long for the edge is ellipsized; body lines
/// are never cut — the box grows instead, so callers that must respect a window width
/// (see `wrap`) pass body already wrapped to `width - 2`. Widths are counted in chars.
pub fn warn_box(col: &str, rst: &str, caption: &str, body: &[String], width: usize) -> Vec<String> {
    let mut w = width;
    for l in body {
        let ln = l.chars().count() + 2;
        if ln > w {
            w = ln;
        }
    }
    // "─ caption " — the caption gets whatever the edge leaves it.
    let title = format!("─ {} ", ellipsize(caption, w.saturating_sub(3)));
    let title_len = title.chars().count();
    let w = w.max(title_len);
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

/// Word-wrap `text` to at most `width` characters per line, breaking on whitespace. A
/// word too long to ever fit (a narrow window, a long Polish compound) is hard-broken
/// across lines rather than overflowing — no line comes back wider than `width`.
/// Paragraphs (separated by a blank line, `\n\n`) are preserved: each is wrapped
/// independently and separated by one empty line. Empty text -> no lines.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
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
            // The word just placed may itself be wider than the line: split off full
            // lines until what's left fits.
            while cur.chars().count() > width {
                lines.push(cur.chars().take(width).collect());
                cur = cur.chars().skip(width).collect();
            }
        }
        if !cur.is_empty() {
            lines.push(cur);
        }
    }
    lines
}

/// Word-wrap one display line to `width`, keeping its leading indent on the continuation
/// lines so the block stays visually aligned. `None` (output is not a terminal) leaves
/// the line untouched.
pub fn wrap_line(line: &str, width: Option<usize>) -> Vec<String> {
    let Some(width) = width.filter(|w| line.chars().count() > *w) else {
        return vec![line.to_string()];
    };
    let indent = line.chars().take_while(|c| *c == ' ').count();
    wrap(line.trim_start(), width.saturating_sub(indent))
        .into_iter()
        .map(|l| format!("{}{l}", " ".repeat(indent)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body() -> Vec<String> {
        vec!["hello".to_string(), "a longer line here".to_string()]
    }

    #[test]
    fn draws_four_lines_for_two_line_body() {
        let lines = warn_box("", "", "NOTICE", &body(), 0);
        assert_eq!(lines.len(), 4);
    }

    #[test]
    fn caption_set_into_top_edge() {
        let lines = warn_box("", "", "NOTICE", &body(), 0);
        assert!(lines[0].starts_with("┌─ NOTICE "));
        assert!(lines[0].ends_with('┐'));
    }

    #[test]
    fn body_lines_enclosed_and_preserved() {
        let lines = warn_box("", "", "NOTICE", &body(), 0);
        assert!(lines[1].starts_with("│ hello"));
        assert!(lines[2].contains("a longer line here"));
        assert!(lines[3].starts_with('└') && lines[3].ends_with('┘'));
    }

    #[test]
    fn all_rows_share_one_char_width() {
        let lines = warn_box("", "", "NOTICE", &body(), 0);
        let w = lines[0].chars().count();
        for l in &lines {
            assert_eq!(l.chars().count(), w);
        }
    }

    #[test]
    fn width_forces_a_uniform_wider_box() {
        // content is short, but width 60 makes every row 60 inner + 2 border = 62.
        let lines = warn_box("", "", "NOTICE", &body(), 60);
        for l in &lines {
            assert_eq!(l.chars().count(), 62);
        }
    }

    #[test]
    fn box_stays_within_the_width_at_any_width() {
        // Pre-wrapped body (as callers pass it) + a caption too long for the edge: the
        // box still fits, however narrow the window is.
        for w in 4..=40usize {
            let body = wrap("Silny wiatr — od 22:00 do jutra 03:00", w.saturating_sub(2));
            let lines = warn_box("", "", "OSTRZEŻENIE! (stopień 1, 70%)", &body, w);
            for l in &lines {
                assert_eq!(l.chars().count(), w + 2, "at width {w}: {l:?}");
            }
        }
    }

    #[test]
    fn caption_too_long_for_the_edge_is_ellipsized() {
        let lines = warn_box("", "", "OSTRZEŻENIE! (stopień 1, 70%)", &[], 12);
        assert_eq!(lines[0], "┌─ OSTRZEŻE… ┐");
        assert_eq!(lines[0].chars().count(), 14); // 12 inner + 2 borders
    }

    #[test]
    fn wrap_breaks_on_spaces_within_width() {
        assert!(wrap("", 10).is_empty());
        assert_eq!(wrap("one two three", 7), vec!["one two", "three"]);
        // wide chars counted by char, not byte
        assert_eq!(wrap("30°C do 33°C", 6), vec!["30°C", "do", "33°C"]);
    }

    #[test]
    fn wrap_hard_breaks_a_word_too_long_to_fit() {
        // no line may exceed the width, even when a single word can't fit on one
        assert_eq!(wrap("aaaaaaaaaa bb", 5), vec!["aaaaa", "aaaaa", "bb"]);
        assert_eq!(
            wrap("ab południowo-zachodniego", 10),
            vec!["ab", "południowo", "-zachodnie", "go"]
        );
    }

    #[test]
    fn wrap_line_keeps_the_indent_and_passes_through_untouched_when_it_fits() {
        assert_eq!(
            wrap_line("   Wiatr: 3 m/s", Some(40)),
            vec!["   Wiatr: 3 m/s"]
        );
        // not a terminal -> no wrapping at all
        assert_eq!(
            wrap_line("   a very long forecast line indeed", None),
            vec!["   a very long forecast line indeed"]
        );
        // continuation lines line up under the first
        assert_eq!(
            wrap_line("   Wiatr z zachodu: 3 m/s", Some(16)),
            vec!["   Wiatr z", "   zachodu: 3", "   m/s"]
        );
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
