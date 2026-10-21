//! How the shell's own parts look: the header, transcript lines, the echo of
//! a sent line, the command menu, the input box, the status bar and the
//! footer. Each is a method of [`Skin`] with a default; override one to
//! change that part, and call the free function of the same name to build
//! on the default instead of replacing it:
//!
//! ```
//! use abalone::skin::{self, Skin};
//! use abalone::{Row, Style};
//!
//! /// The default footer, with a clock-like tag on the end.
//! struct Mine;
//! impl Skin for Mine {
//!     fn footer(&self, text: &str, w: usize) -> Row {
//!         let mut row = skin::footer(text, w);
//!         row.push((Style::default().bold(), "  · beta".into()));
//!         row
//!     }
//! }
//! ```

use crate::editor::Suggestion;
use crate::out::{Kind, Line};
use crate::status::{Notice, status_row};
use crate::style::{Row, Style, char_width, highlight, pad, text_width, wrap};
use crate::theme::theme;

/// The input line as the shell sees it.
pub struct Input<'a> {
    pub prompt: &'a str,
    pub line: &'a str,
    /// Char index of the cursor.
    pub cursor: usize,
    /// The rest of the highlighted command, after the cursor.
    pub ghost: Option<String>,
    /// Shown while the line is empty.
    pub placeholder: &'a str,
    /// The line starts with a command the shell knows.
    pub known_command: bool,
}

pub trait Skin {
    /// The header at the top of the transcript: the logo with `lines` beside it.
    fn header(&self, logo: Vec<Row>, lines: &[Row], w: usize) -> Vec<Row> {
        header(logo, lines, w)
    }

    /// One line of output, wrapped to `w`.
    fn line(&self, kind: Kind, text: &str, w: usize) -> Vec<Row> {
        line(kind, text, w)
    }

    /// A line of styled pieces, wrapped to `w`.
    fn pieces(&self, pieces: &Line, w: usize) -> Vec<Row> {
        pieces_rows(pieces, w)
    }

    /// A line the user sent.
    fn echo(&self, prompt: &str, text: &str, w: usize) -> Vec<Row> {
        echo(prompt, text, w)
    }

    /// The command menu above the input box; at most `max` suggestions show.
    fn menu(&self, items: &[Suggestion], sel: usize, typed: &str, max: usize) -> Vec<Row> {
        menu(items, sel, typed, max)
    }

    /// The input line, and the column the cursor goes in.
    fn input(&self, input: &Input, w: usize) -> (Row, usize) {
        self::input(input, w)
    }

    /// The rules above and below the input box.
    fn rule(&self, w: usize) -> Row {
        rule(w)
    }

    /// The status bar: `left` (a running command or the scroll), then the notices.
    fn status(&self, left: Row, notices: &[Notice], w: usize) -> Row {
        status_row(left, notices, w)
    }

    /// The line under the input box.
    fn footer(&self, text: &str, w: usize) -> Row {
        footer(text, w)
    }
}

/// The look Abalone ships with.
#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultSkin;

impl Skin for DefaultSkin {}

pub fn header(logo: Vec<Row>, lines: &[Row], _w: usize) -> Vec<Row> {
    let mut rows: Vec<Row> = vec![vec![]];
    let logo_w = logo.iter().map(crate::style::row_width).max().unwrap_or(0);
    let n = logo.len().max(lines.len());
    // Text beside a taller logo is centred on it.
    let offset = logo.len().saturating_sub(lines.len()) / 2;
    for i in 0..n {
        let mut row: Row = vec![(Style::default(), " ".into())];
        if logo_w > 0 {
            let piece = logo
                .get(i)
                .cloned()
                .unwrap_or_else(|| vec![(Style::default(), " ".repeat(logo_w))]);
            row.extend(piece);
            row.push((Style::default(), "   ".into()));
        }
        if let Some(text) = i.checked_sub(offset).and_then(|j| lines.get(j)) {
            row.extend(text.iter().cloned());
        }
        rows.push(row);
    }
    rows.push(vec![]);
    rows
}

/// Width for transcript text: one column of margin each side.
fn body_width(w: usize) -> usize {
    w.saturating_sub(3).max(10)
}

pub fn line(kind: Kind, text: &str, w: usize) -> Vec<Row> {
    wrap(text, body_width(w))
        .into_iter()
        .map(|piece| vec![(kind.style(), format!(" {piece}"))])
        .collect()
}

/// Only the last piece wraps, under itself, so a bullet's text lines up past the bullet.
pub fn pieces_rows(pieces: &Line, w: usize) -> Vec<Row> {
    let text_w = body_width(w);
    let mut rows = Vec::new();
    let mut row: Row = vec![(Style::default(), " ".into())];
    let Some(((kind, last), lead)) = pieces.split_last() else {
        return vec![row];
    };
    row.extend(lead.iter().map(|(kind, text)| (kind.style(), text.clone())));
    let lead_w: usize = lead.iter().map(|(_, t)| text_width(t)).sum();
    if lead_w > text_w / 2 {
        // Too little room beside the lead: the last piece wraps on rows of its own.
        rows.push(row);
        for piece in wrap(last, text_w) {
            rows.push(vec![(Style::default(), " ".into()), (kind.style(), piece)]);
        }
        return rows;
    }
    for (i, piece) in wrap(last, text_w - lead_w).into_iter().enumerate() {
        if i > 0 {
            rows.push(std::mem::replace(
                &mut row,
                vec![(Style::default(), " ".repeat(1 + lead_w))],
            ));
        }
        row.push((kind.style(), piece));
    }
    rows.push(row);
    rows
}

pub fn echo(prompt: &str, text: &str, w: usize) -> Vec<Row> {
    let t = theme();
    let prompt_w = text_width(prompt);
    // A space and the prompt before the text, and the last column left free.
    let text_w = w.saturating_sub(2 + prompt_w).max(1);
    let bg = Style::default().bg(t.echo_bg);
    let indent = " ".repeat(prompt_w);
    let mut rows = vec![vec![]];
    for (i, piece) in wrap(text, text_w).into_iter().enumerate() {
        let lead = if i == 0 { prompt } else { &indent };
        let pad = text_w.saturating_sub(text_width(&piece));
        rows.push(vec![
            (
                Style {
                    fg: Some(t.muted),
                    ..bg
                },
                format!(" {lead}"),
            ),
            (
                Style {
                    fg: Some(t.bright),
                    ..bg
                },
                format!("{piece}{}", " ".repeat(pad)),
            ),
        ]);
    }
    rows.push(vec![]);
    rows
}

pub fn menu(items: &[Suggestion], sel: usize, typed: &str, max: usize) -> Vec<Row> {
    let t = theme();
    let max = max.max(1);
    let query = typed.trim_start_matches('/').to_lowercase();
    let n = items.len();
    let first = sel.saturating_sub(max - 1).min(n.saturating_sub(max));
    let label_w = items
        .iter()
        .map(|i| text_width(&i.label))
        .max()
        .unwrap_or(0)
        .clamp(16, 32);
    let (muted, mark) = (Style::fg(t.muted), Style::fg(t.bright).bold());
    let mut rows = Vec::new();
    for (i, item) in items.iter().enumerate().skip(first).take(max) {
        let label = pad(&item.label, label_w);
        let mut row: Row = Vec::new();
        if i == sel {
            let accent = Style::fg(t.accent);
            row.push((accent, "  ❯ ".into()));
            row.push((accent, label));
            row.push((accent, format!("  {}", item.description)));
        } else {
            row.push((Style::default(), "    ".into()));
            row.extend(highlight(&label, &query, false, muted, mark));
            row.push((muted, "  ".into()));
            row.extend(highlight(&item.description, &query, true, muted, mark));
        }
        rows.push(row);
    }
    if n > max {
        rows.push(vec![(muted, format!("    {}/{n} · ↑↓ for more", sel + 1))]);
    }
    rows
}

/// The input line scrolled sideways to keep the cursor in view; a known
/// command's name in the accent colour.
pub fn input(input: &Input, w: usize) -> (Row, usize) {
    let t = theme();
    let prompt_w = text_width(input.prompt);
    let room = w.saturating_sub(prompt_w + 1);
    let line: Vec<char> = input.line.chars().collect();
    let cursor = input.cursor.min(line.len());
    // The first char shown: as far left as keeps the cursor in `room` columns.
    let mut start = cursor;
    let mut before = 0;
    while start > 0 && before + char_width(line[start - 1]) <= room {
        start -= 1;
        before += char_width(line[start]);
    }
    let mut end = start;
    let mut used = 0;
    while end < line.len() && used + char_width(line[end]) <= room {
        used += char_width(line[end]);
        end += 1;
    }
    let shown: String = line[start..end].iter().collect();
    let mut row: Row = vec![(Style::fg(t.muted), input.prompt.into())];
    let name_end = shown.find(char::is_whitespace).unwrap_or(shown.len());
    if start == 0 && input.known_command {
        row.push((Style::fg(t.accent), shown[..name_end].to_owned()));
        row.push((Style::default(), shown[name_end..].to_owned()));
    } else {
        row.push((Style::default(), shown));
    }
    let ghost = if line.is_empty() {
        Some(input.placeholder.to_owned())
    } else {
        input.ghost.clone()
    };
    if let Some(g) = ghost {
        row.push((Style::fg(t.muted), g));
    }
    (row, prompt_w + before)
}

pub fn rule(w: usize) -> Row {
    vec![(Style::fg(theme().rule), "─".repeat(w.saturating_sub(1)))]
}

pub fn footer(text: &str, _w: usize) -> Row {
    vec![(Style::fg(theme().muted), format!("  {text}"))]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::row_text;

    fn texts(rows: &[Row]) -> Vec<String> {
        rows.iter().map(row_text).collect()
    }

    #[test]
    fn wrapping_lines_and_pieces() {
        assert_eq!(texts(&line(Kind::Out, "abcdefghij klmn", 13)), [" abcdefghij", " klmn"]);
        let rows = pieces_rows(
            &vec![
                (Kind::Dim, "  • ".into()),
                (
                    Kind::Out,
                    "A note long enough that it has to wrap onto a second row.".into(),
                ),
            ],
            40,
        );
        assert_eq!(
            texts(&rows),
            ["   • A note long enough that it has to", "     wrap onto a second row."]
        );
    }

    #[test]
    fn header_puts_the_text_beside_the_logo() {
        let logo = vec![vec![(Style::default(), "##".into())]; 3];
        let lines = vec![vec![(Style::default(), "Name".into())]];
        let rows = texts(&header(logo, &lines, 40));
        assert_eq!(rows, ["", " ##   ", " ##   Name", " ##   ", ""]);
        let rows = texts(&header(vec![], &lines, 40));
        assert_eq!(rows, ["", " Name", ""]);
    }

    #[test]
    fn menu_marks_the_selection() {
        let items: Vec<_> = (0..10)
            .map(|i| Suggestion::new(format!("/c{i}"), format!("/c{i}"), format!("about {i}")))
            .collect();
        let rows = menu(&items, 9, "/c", 8);
        assert_eq!(rows.len(), 9, "a window of rows plus the count");
        assert!(row_text(&rows[7]).starts_with("  ❯ /c9"));
        assert!(row_text(&rows[8]).contains("10/10"));
    }

    #[test]
    fn the_input_scrolls_to_the_cursor() {
        let long = "x".repeat(50);
        let i = Input {
            prompt: "❯ ",
            line: &long,
            cursor: 50,
            ghost: None,
            placeholder: "",
            known_command: false,
        };
        let (row, col) = input(&i, 20);
        assert_eq!(col, 19);
        assert_eq!(row_text(&row).chars().count(), 19);
        // Wide characters: columns, not chars, decide what fits.
        let wide = "日本語".repeat(10);
        let i = Input {
            line: &wide,
            cursor: 30,
            ..i
        };
        let (row, col) = input(&i, 20);
        assert_eq!(col, 18);
        assert!(crate::style::row_width(&row) <= 19);
    }

    #[test]
    fn an_echo_fits_its_row() {
        for prompt in ["❯ ", "my-app> "] {
            for row in echo(prompt, &"word ".repeat(30), 40) {
                assert!(crate::style::row_width(&row) <= 39, "{prompt:?}: {:?}", row_text(&row));
            }
        }
        let rows = texts(&echo("❯ ", &"x".repeat(80), 40));
        let shown: usize = rows.iter().map(|r| r.matches('x').count()).sum();
        assert_eq!(shown, 80, "no character lost at the edge");
    }

    #[test]
    fn pieces_wrap_on_their_own_rows_when_the_lead_is_wide() {
        let rows = texts(&pieces_rows(
            &vec![
                (Kind::Dim, "a long lead piece ".into()),
                (Kind::Out, "then the text".into()),
            ],
            24,
        ));
        assert_eq!(rows, [" a long lead piece ", " then the text"]);
    }
}
