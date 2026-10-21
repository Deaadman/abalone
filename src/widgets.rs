//! Building blocks for panels: the frame (rule, title, tabs, body, hint),
//! wrapped paragraphs, labelled fields, bullets, selectable lists, a text
//! field, scrolling, and the keys panels share. The built-in help and
//! settings panels are made of these, and so can yours be.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use crossterm::style::Color;

use crate::style::{Row, Style, pad, wrap};
use crate::theme::theme;

/// Width of a field's label column.
pub const LABEL_W: usize = 16;

/// A full panel: a rule, the title, the tabs, the body cut or padded to fit,
/// and a hint, exactly `h` rows.
pub fn frame(w: usize, h: usize, title: Row, tabs: Option<(&[&str], usize)>, body: Vec<Row>, hint: &str) -> Vec<Row> {
    let mut rows: Vec<Row> = vec![accent_rule(w), title, vec![]];
    if let Some((names, tab)) = tabs {
        rows.push(tabs_row(names, tab));
        rows.push(vec![]);
    }
    let footer = [vec![], vec![(Style::fg(theme().muted), format!("  {hint}"))]];
    let body_h = h.saturating_sub(rows.len() + footer.len()).max(1);
    let mut body = body;
    body.truncate(body_h);
    body.resize(body_h, vec![]);
    rows.extend(body);
    rows.extend(footer);
    rows.truncate(h);
    rows
}

/// Height left for a body under `frame`'s title (and tabs).
pub fn body_height(h: usize, tabs: bool) -> usize {
    h.saturating_sub(if tabs { 7 } else { 5 }).max(1)
}

/// The panel's top edge: a rule in the accent colour.
pub fn accent_rule(w: usize) -> Row {
    vec![(Style::fg(theme().accent), "─".repeat(w.saturating_sub(1)))]
}

/// A panel title: `  Name` in bold accent.
pub fn title(text: &str) -> Row {
    vec![(Style::fg(theme().accent).bold(), format!("  {text}"))]
}

/// The tab names, the selected one drawn on the bright colour.
pub fn tabs_row(names: &[&str], tab: usize) -> Row {
    let mut row: Row = vec![(Style::default(), " ".into())];
    for (i, name) in names.iter().enumerate() {
        row.push((Style::default(), " ".into()));
        row.push((tab_style(i == tab), format!(" {name} ")));
    }
    row
}

/// `title` with the tabs beside it, as the help and settings panels draw them.
pub fn title_with_tabs(title: &str, names: &[&str], tab: usize) -> Row {
    let mut row: Row = vec![(Style::fg(theme().accent).bold(), format!("  {title}"))];
    for (i, name) in names.iter().enumerate() {
        row.push((Style::default(), "  ".into()));
        row.push((tab_style(i == tab), format!(" {name} ")));
    }
    row
}

fn tab_style(on: bool) -> Style {
    if on {
        let t = theme();
        Style::fg(t.on_bright).bg(t.bright)
    } else {
        Style::default()
    }
}

/// `  Text` in bold, optionally coloured.
pub fn heading(text: &str, color: Option<Color>) -> Row {
    let style = color.map_or(Style::default(), Style::fg).bold();
    vec![(style, format!("  {text}"))]
}

/// A paragraph wrapped to the screen, indented `ind` columns.
pub fn para(text: &str, ind: usize, w: usize, style: Style) -> Vec<Row> {
    wrap(text, w.saturating_sub(ind + 2).max(10))
        .into_iter()
        .map(|p| vec![(style, format!("{}{p}", " ".repeat(ind)))])
        .collect()
}

/// A thin rule under a list, before the selected item's detail.
pub fn rule_line(w: usize) -> Row {
    vec![(
        Style::fg(theme().muted),
        format!("    {}", "─".repeat(w.saturating_sub(8).clamp(10, 60))),
    )]
}

/// `label` muted, then `text` wrapped beside it.
pub fn field(label: &str, text: &str, w: usize, style: Style) -> Vec<Row> {
    let muted = Style::fg(theme().muted);
    wrap(text, w.saturating_sub(4 + LABEL_W + 3).max(10))
        .into_iter()
        .enumerate()
        .map(|(i, p)| {
            vec![
                (muted, format!("    {}", pad(if i == 0 { label } else { "" }, LABEL_W))),
                (style, p),
            ]
        })
        .collect()
}

/// Numbered steps beside a label.
pub fn steps(label: &str, items: &[String], w: usize) -> Vec<Row> {
    let t = theme();
    let mut out = Vec::new();
    for (n, text) in items.iter().enumerate() {
        for (i, p) in wrap(text, w.saturating_sub(4 + LABEL_W + 6).max(10))
            .into_iter()
            .enumerate()
        {
            out.push(vec![
                (
                    Style::fg(t.muted),
                    format!("    {}", pad(if n == 0 && i == 0 { label } else { "" }, LABEL_W)),
                ),
                (
                    Style::fg(t.good),
                    if i == 0 { format!("{}. ", n + 1) } else { "   ".into() },
                ),
                (Style::default(), p),
            ]);
        }
    }
    out
}

/// A bullet: `    + text`, wrapped under itself.
pub fn bullet(mark: &str, color: Color, text: &str, w: usize) -> Vec<Row> {
    wrap(text, w.saturating_sub(10).max(10))
        .into_iter()
        .enumerate()
        .map(|(i, p)| {
            vec![
                (
                    Style::fg(color).bold(),
                    if i == 0 {
                        format!("    {mark} ")
                    } else {
                        "      ".into()
                    },
                ),
                (Style::default(), p),
            ]
        })
        .collect()
}

/// One selectable line: its text, and an optional tail in its own colour.
pub struct Item {
    pub text: String,
    pub tail: Option<(String, Color)>,
}

impl Item {
    pub fn new(text: impl Into<String>) -> Self {
        Item {
            text: text.into(),
            tail: None,
        }
    }

    pub fn tail(mut self, text: impl Into<String>, color: Color) -> Self {
        self.tail = Some((text.into(), color));
        self
    }
}

/// `items` as a list `h` rows tall, the selection marked and kept in view.
pub fn list_rows(items: Vec<Item>, sel: usize, h: usize) -> Vec<Row> {
    let accent = theme().accent;
    let n = items.len();
    let first = sel.saturating_sub(h.saturating_sub(1)).min(n.saturating_sub(h));
    items
        .into_iter()
        .enumerate()
        .skip(first)
        .take(h)
        .map(|(i, it)| {
            let on = i == sel;
            let mut row: Row = vec![(
                if on { Style::fg(accent) } else { Style::default() },
                format!("{}{}", if on { "  ❯ " } else { "    " }, it.text),
            )];
            if let Some((t, c)) = it.tail {
                row.push((Style::fg(if on { accent } else { c }), t));
            }
            row
        })
        .collect()
}

/// `lines` from `scroll`, `h` tall, the last row saying how many more.
pub fn scrolled(scroll: &mut usize, lines: Vec<Row>, h: usize) -> Vec<Row> {
    let max = lines.len().saturating_sub(h);
    *scroll = (*scroll).min(max);
    let mut rows: Vec<Row> = lines.into_iter().skip(*scroll).take(h).collect();
    if *scroll < max
        && let Some(last) = rows.last_mut()
    {
        *last = vec![(Style::fg(theme().muted), format!("    ↓ {} more", max - *scroll))];
    }
    rows
}

/// A one-line text field: the text with a block cursor at `cur`.
pub fn text_field(buf: &[char], cur: usize, base: Style) -> Row {
    let t = theme();
    let mut row: Row = Vec::new();
    if cur > 0 {
        row.push((base, buf[..cur].iter().collect()));
    }
    row.push((
        Style::fg(t.on_bright).bg(t.bright),
        buf.get(cur).map_or(" ".into(), |c| c.to_string()),
    ));
    if cur + 1 < buf.len() {
        row.push((base, buf[cur + 1..].iter().collect()));
    }
    row
}

/// Esc, q or Ctrl+C.
pub fn is_close(key: &KeyEvent) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    matches!(key.code, KeyCode::Esc)
        || (key.code == KeyCode::Char('q') && !ctrl)
        || (ctrl && key.code == KeyCode::Char('c'))
}

/// ←→, Tab and the number keys move between `n` tabs; true when they did.
pub fn tab_key(tab: &mut usize, key: &KeyEvent, n: usize) -> bool {
    match key.code {
        KeyCode::Left | KeyCode::BackTab => *tab = (*tab + n - 1) % n,
        KeyCode::Right | KeyCode::Tab => *tab = (*tab + 1) % n,
        KeyCode::Char(c) if c.is_ascii_digit() && !key.modifiers.contains(KeyModifiers::CONTROL) => {
            let i = c as usize - '1' as usize;
            if c == '0' || i >= n {
                return false;
            }
            *tab = i;
        }
        _ => return false,
    }
    true
}

/// ↑↓, PgUp/PgDn, Home/End as a step.
pub fn step_of(key: &KeyEvent) -> isize {
    match key.code {
        KeyCode::Up => -1,
        KeyCode::Down => 1,
        KeyCode::PageUp => -10,
        KeyCode::PageDown => 10,
        KeyCode::Home => -10_000,
        KeyCode::End => 10_000,
        _ => 0,
    }
}

/// Move `v` by `by`, kept within `n` items.
pub fn step(v: &mut usize, by: isize, n: usize) {
    *v = (*v as isize + by).clamp(0, n.saturating_sub(1) as isize) as usize;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::row_text;

    #[test]
    fn frame_is_exactly_as_tall_as_asked() {
        for h in [3, 8, 20] {
            let rows = frame(60, h, title("T"), Some((&["A", "B"], 1)), vec![vec![]; 40], "hint");
            assert_eq!(rows.len(), h);
        }
    }

    #[test]
    fn field_puts_the_cursor_on_a_character() {
        let buf: Vec<char> = "C:\\x".chars().collect();
        assert_eq!(row_text(&text_field(&buf, 4, Style::default())), "C:\\x ");
        assert_eq!(row_text(&text_field(&buf, 0, Style::default())), "C:\\x");
        assert_eq!(text_field(&buf, 1, Style::default())[1].1, ":");
    }

    #[test]
    fn lists_keep_the_selection_in_view() {
        let items = (0..20).map(|i| Item::new(format!("item {i}"))).collect();
        let rows = list_rows(items, 15, 5);
        assert_eq!(rows.len(), 5);
        assert!(row_text(&rows[4]).starts_with("  ❯ item 15"));
    }
}
