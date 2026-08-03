//! Terminal presentation: colours and the warning box. Colours self-blank on
//! NO_COLOR / non-tty.

use std::io::IsTerminal;

/// ANSI colour strings, or all-empty when colour is disabled.
pub struct Colors {
    pub red_box: String,  // BOLD + RED   — loud meteo warnings
    pub grey_box: String, // BOLD + DARK_GREY — quiet notices (drought)
    pub reset: String,
}

impl Colors {
    /// Enabled unless `NO_COLOR` is set or stdout is not a terminal.
    pub fn detect() -> Self {
        let on = std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal();
        if on {
            Colors {
                red_box: "\x1b[1m\x1b[0;31m".into(),
                grey_box: "\x1b[1m\x1b[1;30m".into(),
                reset: "\x1b[0m".into(),
            }
        } else {
            Colors::plain()
        }
    }

    /// All-empty colours, for non-coloured output (pipes, tests).
    pub fn plain() -> Self {
        Colors {
            red_box: String::new(),
            grey_box: String::new(),
            reset: String::new(),
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
}
