//! Styled text: a screen row is a list of pieces, each in its own style.
//! Everything shellkit draws is built from these, so a component's output
//! can be changed or extended with ordinary `Vec` operations.

use crossterm::style::Color;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Style {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub bold: bool,
}

impl Style {
    pub fn fg(c: Color) -> Self {
        Style {
            fg: Some(c),
            ..Style::default()
        }
    }

    pub fn bg(self, c: Color) -> Self {
        Style { bg: Some(c), ..self }
    }

    pub fn bold(self) -> Self {
        Style { bold: true, ..self }
    }
}

/// One screen row as styled pieces.
pub type Row = Vec<(Style, String)>;

/// Columns `row` takes (one per char).
pub fn row_width(row: &Row) -> usize {
    row.iter().map(|(_, s)| s.chars().count()).sum()
}

/// The row's text without styles.
pub fn row_text(row: &Row) -> String {
    row.iter().map(|(_, s)| s.as_str()).collect()
}

pub fn rgb((r, g, b): (u8, u8, u8)) -> Color {
    Color::Rgb { r, g, b }
}

/// `text` padded with spaces to `n` columns.
pub fn pad(text: &str, n: usize) -> String {
    let k = text.chars().count();
    if k >= n {
        text.to_owned()
    } else {
        format!("{text}{}", " ".repeat(n - k))
    }
}

/// Wrap at spaces into rows of at most `w` chars, breaking a longer word
/// where it must. Continuation rows keep the line's indent.
pub fn wrap(text: &str, w: usize) -> Vec<String> {
    let w = w.max(1);
    let chars: Vec<char> = text.chars().collect();
    let indent = chars.iter().take_while(|c| **c == ' ').count().min(w / 2);
    let mut rows = Vec::new();
    let mut rest = &chars[..];
    let mut first = true;
    loop {
        let lead = if first { 0 } else { indent };
        let room = w - lead;
        if rest.len() <= room {
            rows.push(" ".repeat(lead) + &rest.iter().collect::<String>());
            return rows;
        }
        // The last space that leaves something on this row, else a hard break.
        let cut = rest[..=room]
            .iter()
            .rposition(|c| *c == ' ')
            .filter(|&i| rest[..i].iter().any(|c| *c != ' '))
            .unwrap_or(room);
        rows.push(" ".repeat(lead) + rest[..cut].iter().collect::<String>().trim_end());
        rest = &rest[cut..];
        while rest.first() == Some(&' ') {
            rest = &rest[1..];
        }
        first = false;
        if rest.is_empty() {
            return rows;
        }
    }
}

/// `text` split into `base` and `mark` pieces where it matches `query`: the
/// first occurrence, or (`words`) every word that starts with it.
pub fn highlight(text: &str, query: &str, words: bool, base: Style, mark: Style) -> Row {
    let chars: Vec<char> = text.chars().collect();
    let lower: Vec<char> = text.to_lowercase().chars().collect();
    let q: Vec<char> = query.chars().collect();
    let mut marked = vec![false; chars.len()];
    if !q.is_empty() && lower.len() == chars.len() {
        let matches_at = |i: usize| lower.get(i..i + q.len()) == Some(&q[..]);
        if words {
            for i in 0..chars.len() {
                let word_start = i == 0 || !chars[i - 1].is_alphanumeric();
                if word_start && matches_at(i) {
                    marked[i..i + q.len()].fill(true);
                }
            }
        } else if let Some(i) = (0..chars.len()).find(|&i| matches_at(i)) {
            marked[i..i + q.len()].fill(true);
        }
    }
    let mut row = Row::new();
    for (c, m) in chars.into_iter().zip(marked) {
        let style = if m { mark } else { base };
        match row.last_mut() {
            Some((s, text)) if *s == style => text.push(c),
            _ => row.push((style, c.to_string())),
        }
    }
    row
}

/// Pixel art drawn with half blocks: two pixel rows to a screen row. `None`
/// pixels are left blank. Rows past the last pair are ignored.
pub fn half_blocks(pixels: &[Vec<Option<Color>>]) -> Vec<Row> {
    pixels
        .chunks(2)
        .map(|pair| {
            let top = &pair[0];
            let bottom = pair.get(1);
            (0..top.len())
                .map(|c| match (top[c], bottom.and_then(|b| b.get(c).copied().flatten())) {
                    (None, None) => (Style::default(), " ".to_owned()),
                    (Some(t), None) => (Style::fg(t), "▀".to_owned()),
                    (None, Some(b)) => (Style::fg(b), "▄".to_owned()),
                    (Some(t), Some(b)) if t == b => (Style::fg(t), "█".to_owned()),
                    (Some(t), Some(b)) => (Style::fg(t).bg(b), "▀".to_owned()),
                })
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapping() {
        assert_eq!(wrap("", 4), [""]);
        assert_eq!(wrap("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        assert_eq!(wrap("3 replays: 0 new, 3 done", 12), ["3 replays: 0", "new, 3 done"]);
        assert_eq!(wrap("  ab cd ef", 6), ["  ab", "  cd", "  ef"]);
        assert_eq!(wrap("  abcdefgh", 6), ["  abcd", "  efgh"]);
    }

    #[test]
    fn highlighting() {
        let base = Style::fg(Color::Grey);
        let bold = Style::fg(Color::White).bold();
        assert_eq!(
            highlight("/model", "m", false, base, bold),
            vec![
                (base, "/".to_owned()),
                (bold, "m".to_owned()),
                (base, "odel".to_owned())
            ]
        );
        let row = highlight("Set the model, mostly", "m", true, base, bold);
        let marked: Vec<&str> = row
            .iter()
            .filter(|(s, _)| *s == bold)
            .map(|(_, t)| t.as_str())
            .collect();
        assert_eq!(marked, ["m", "m"]);
        assert_eq!(highlight("abc", "", true, base, bold), vec![(base, "abc".to_owned())]);
    }

    #[test]
    fn half_blocks_pair_rows() {
        let r = Some(Color::Red);
        let rows = half_blocks(&[vec![r, None, r], vec![r, r, None]]);
        assert_eq!(rows.len(), 1);
        assert_eq!(row_text(&rows[0]), "█▄▀");
    }
}
