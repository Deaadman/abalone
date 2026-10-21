//! `/release-notes`: every version's notes, from the app's own
//! `CHANGELOG.md` built into it, so they show offline and match the build.
//! The changelog follows [Keep a Changelog](https://keepachangelog.com/en/2.0.0/)
//! (`## [1.2.0] - 2026-10-21`, `- ` entries under `### Added` and the like),
//! which `release-notes.ps1` checks too.
//!
//! ```
//! # struct App;
//! # impl abalone::App for App {
//! #     type Msg = (); type Panel = abalone::NoPanel;
//! #     fn commands(&self) -> Vec<abalone::CommandInfo> { vec![] }
//! #     fn run(&mut self, _: &str, _: &mut abalone::Ctx<Self>) {}
//! # }
//! # let changelog = "# Changelog\n";
//! // include_str!("../CHANGELOG.md") in an app.
//! let shell = abalone::Shell::new(App).release_notes(changelog);
//! ```

use crossterm::event::{KeyCode, KeyEvent};

use crate::out::{Kind, Line};
use crate::style::{Row, Style, pad};
use crate::theme::theme;
use crate::widgets::{Item, is_close, list_rows, step, step_of};

/// One released version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub date: Option<String>,
    /// Its entries, in the order written, whatever their type.
    pub items: Vec<String>,
}

/// The released versions in `changelog`, newest first as written.
/// `[Unreleased]` and anything that isn't an entry are left out.
pub fn parse(changelog: &str) -> Vec<Release> {
    let mut releases: Vec<Release> = Vec::new();
    let mut current: Option<Release> = None;
    let mut in_comment = false;
    for line in changelog.lines() {
        let trimmed = line.trim();
        if in_comment {
            in_comment = !trimmed.contains("-->");
            continue;
        }
        if trimmed.starts_with("<!--") {
            in_comment = !trimmed.contains("-->");
            continue;
        }
        if let Some(heading) = line.strip_prefix("## ") {
            releases.extend(current.take());
            let (name, date) = match heading.split_once(" - ") {
                Some((n, d)) => (n, Some(d.trim().to_owned())),
                None => (heading, None),
            };
            let name = name.trim().trim_start_matches('[').trim_end_matches(']').trim();
            if !name.eq_ignore_ascii_case("unreleased") && !name.is_empty() {
                current = Some(Release {
                    version: name.trim_start_matches('v').to_owned(),
                    date,
                    items: vec![],
                });
            }
            continue;
        }
        let Some(release) = &mut current else { continue };
        if line.starts_with(['-', '*'])
            && let Some(entry) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* "))
        {
            release.items.push(entry.trim().to_owned());
            continue;
        }
        // An entry wrapped onto the next lines, indented under it.
        if !trimmed.is_empty()
            && line.starts_with([' ', '\t'])
            && let Some(last) = release.items.last_mut()
        {
            last.push(' ');
            last.push_str(trimmed);
        }
    }
    releases.extend(current);
    releases
}

/// `● Version 1.2.0:` and a `·` line for each entry.
pub fn lines(release: &Release) -> Vec<Line> {
    let mut lines = vec![vec![
        (Kind::Bold, "● ".to_owned()),
        (Kind::Bold, format!("Version {}:", release.version)),
    ]];
    for item in &release.items {
        lines.push(vec![(Kind::Dim, "  · ".to_owned()), (Kind::Out, item.clone())]);
    }
    lines
}

/// The version picker: "Show all", then each version and how many entries it has.
pub struct NotesPanel {
    sel: usize,
    scroll: usize,
}

pub enum NotesAction {
    None,
    Close,
    /// Print these versions' notes.
    Show(Vec<usize>),
}

impl NotesPanel {
    pub fn new() -> Self {
        NotesPanel { sel: 0, scroll: 0 }
    }

    pub fn key(&mut self, key: KeyEvent, releases: &[Release]) -> NotesAction {
        if is_close(&key) {
            return NotesAction::Close;
        }
        if releases.is_empty() {
            return if key.code == KeyCode::Enter {
                NotesAction::Close
            } else {
                NotesAction::None
            };
        }
        match key.code {
            KeyCode::Enter => NotesAction::Show(match self.sel {
                0 => (0..releases.len()).collect(),
                n => vec![n - 1],
            }),
            _ => {
                step(&mut self.sel, step_of(&key), releases.len() + 1);
                NotesAction::None
            }
        }
    }

    /// `h` rows at most.
    pub fn rows(&mut self, releases: &[Release], w: usize, h: usize) -> Vec<Row> {
        let t = theme();
        let muted = Style::fg(t.muted);
        let mut rows: Vec<Row> = vec![
            vec![(Style::fg(t.accent), "─".repeat(w.saturating_sub(1)))],
            vec![(Style::fg(t.accent).bold(), "  Release notes".into())],
            vec![],
        ];
        if releases.is_empty() {
            rows.push(vec![(muted, "  No releases yet.".into())]);
            rows.extend([vec![], vec![(muted, "  Esc to close".into())]]);
            return rows;
        }
        rows.extend([vec![(muted, "  Select a version to view its notes.".into())], vec![]]);
        let count = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        let label_w = releases
            .iter()
            .map(|r| r.version.chars().count() + 8)
            .max()
            .unwrap_or(0)
            .max(8)
            + 3;
        let mut items =
            vec![Item::new(pad("Show all", label_w)).tail(count(releases.len(), "version", "versions"), t.muted)];
        items.extend(releases.iter().map(|r| {
            Item::new(pad(&format!("Version {}", r.version), label_w))
                .tail(count(r.items.len(), "item", "items"), t.muted)
        }));
        let footer = [vec![], vec![(muted, "  Enter to confirm · Esc to cancel".into())]];
        let list_h = h.saturating_sub(rows.len() + footer.len()).max(3);
        let n = items.len();
        if n <= list_h {
            rows.extend(list_rows(items, self.sel, n));
        } else {
            // The last row says how many are below; the selection stays in view.
            let window = list_h - 1;
            self.scroll = self.scroll.min(self.sel).max(self.sel + 1 - window.min(self.sel + 1));
            let first = self.scroll.min(n - window);
            let shown: Vec<Item> = items.into_iter().skip(first).take(window).collect();
            rows.extend(list_rows(shown, self.sel - first, window));
            let below = n - first - window;
            rows.push(if below > 0 {
                vec![(muted, format!("    ↓ {below} more"))]
            } else {
                vec![]
            });
        }
        rows.extend(footer);
        rows
    }
}

impl Default for NotesPanel {
    fn default() -> Self {
        NotesPanel::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::row_text;
    use crossterm::event::KeyModifiers;

    const CHANGELOG: &str = "# Changelog

<!--
- not an entry
-->

## [Unreleased]

### Added

- Something coming.

## [1.1.0] - 2026-10-21

### Fixed

- A long fix that wraps
  onto the next line.

### Added

- A new thing.

## [1.0.0] - 2026-10-01

### Added

- The first release.

[Unreleased]: https://example.com
[1.1.0]: https://example.com/1.1.0
";

    #[test]
    fn reads_released_versions_and_their_entries() {
        let r = parse(CHANGELOG);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].version, "1.1.0");
        assert_eq!(r[0].date.as_deref(), Some("2026-10-21"));
        assert_eq!(
            r[0].items,
            ["A long fix that wraps onto the next line.", "A new thing."]
        );
        assert_eq!(r[1].items, ["The first release."]);
    }

    #[test]
    fn picks_one_version_or_all() {
        let r = parse(CHANGELOG);
        let mut p = NotesPanel::new();
        let shown: Vec<String> = p.rows(&r, 60, 20).iter().map(row_text).collect();
        assert!(
            shown
                .iter()
                .any(|l| l.starts_with("  ❯ Show all") && l.ends_with("2 versions"))
        );
        assert!(
            shown
                .iter()
                .any(|l| l.contains("Version 1.0.0") && l.ends_with("1 item"))
        );
        let key = |c| KeyEvent::new(c, KeyModifiers::NONE);
        assert!(matches!(p.key(key(KeyCode::Enter), &r), NotesAction::Show(v) if v == [0, 1]));
        p.key(key(KeyCode::Down), &r);
        p.key(key(KeyCode::Down), &r);
        assert!(matches!(p.key(key(KeyCode::Enter), &r), NotesAction::Show(v) if v == [1]));
        assert!(matches!(p.key(key(KeyCode::Esc), &r), NotesAction::Close));
    }

    #[test]
    fn a_long_list_scrolls_with_the_selection() {
        let r: Vec<Release> = (0..40)
            .map(|i| Release {
                version: format!("1.0.{i}"),
                date: None,
                items: vec!["x".into()],
            })
            .collect();
        let mut p = NotesPanel::new();
        let key = |c| KeyEvent::new(c, KeyModifiers::NONE);
        for _ in 0..30 {
            p.key(key(KeyCode::Down), &r);
        }
        let rows: Vec<String> = p.rows(&r, 60, 20).iter().map(row_text).collect();
        assert!(rows.len() <= 20);
        assert!(rows.iter().any(|l| l.starts_with("  ❯ Version 1.0.29")), "{rows:#?}");
        assert!(rows.iter().any(|l| l.contains("more")));
    }
}
