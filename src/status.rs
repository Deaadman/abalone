//! The status bar above the input box: on the left, what is running (the
//! spinner, its label and the seconds so far) or how far back the output is
//! scrolled; on the right, notices, most important first, as many as fit.

use std::time::Duration;

use crossterm::style::Color;

use crate::spinner::Spinner;
use crate::style::{Row, Style, row_width};
use crate::theme::theme;

/// A message on the right of the status bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub color: Color,
    pub text: String,
}

impl Notice {
    pub fn new(color: Color, text: impl Into<String>) -> Self {
        Notice {
            color,
            text: text.into(),
        }
    }

    /// In the theme's warning colour.
    pub fn warn(text: impl Into<String>) -> Self {
        Notice::new(theme().warn, text)
    }

    /// In the theme's danger colour.
    pub fn danger(text: impl Into<String>) -> Self {
        Notice::new(theme().danger, text)
    }

    /// In the theme's accent colour.
    pub fn info(text: impl Into<String>) -> Self {
        Notice::new(theme().accent, text)
    }

    /// In the theme's colour for good news.
    pub fn good(text: impl Into<String>) -> Self {
        Notice::new(theme().good, text)
    }
}

/// What a background update check found, shown first on the status bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateNote {
    /// A newer version exists but wasn't installed (automatic updates are
    /// off, or this copy wasn't installed by the installer).
    Available,
    /// Installed; it runs from the next launch.
    Installed,
    Failed,
}

impl UpdateNote {
    /// The note, naming `command` (like `/update`) where it helps.
    pub fn notice(self, command: &str) -> Notice {
        match self {
            UpdateNote::Available => Notice::warn(format!("Update available! {command} to install it")),
            UpdateNote::Installed => Notice::good("Update installed! Restart to apply"),
            UpdateNote::Failed => Notice::danger(format!("Update failed · {command} to try again")),
        }
    }
}

/// `  ◆ Label… (12s)`: a running command, with `spinner`'s glyph.
pub fn job_left(spinner: &dyn Spinner, label: &str, elapsed: Duration) -> Row {
    let t = theme();
    let (style, glyph) = spinner.frame(elapsed);
    vec![
        (style, format!("  {glyph} ")),
        (Style::fg(t.accent), format!("{label}… ")),
        (Style::fg(t.muted), format!("({}s)", elapsed.as_secs())),
    ]
}

/// `  ↓ 40 more lines · PgDn`, while the output is scrolled back.
pub fn scrolled_left(lines: usize) -> Row {
    vec![(Style::fg(theme().muted), format!("  ↓ {lines} more lines · PgDn"))]
}

/// `left`, then the notices that fit beside it, right-aligned. A notice
/// that doesn't fit is dropped with every one after it.
pub fn status_row<'a>(mut left: Row, notices: impl IntoIterator<Item = &'a Notice>, w: usize) -> Row {
    let lw = row_width(&left);
    let mut right = Row::new();
    for n in notices {
        let mut piece: Row = Vec::new();
        if !right.is_empty() {
            piece.push((Style::fg(theme().muted), " · ".into()));
        }
        piece.push((Style::fg(n.color), n.text.clone()));
        if lw + row_width(&right) + row_width(&piece) + 3 > w {
            break;
        }
        right.extend(piece);
    }
    let rw = row_width(&right);
    if rw > 0 {
        left.push((Style::default(), " ".repeat(w - lw - rw - 2)));
        left.extend(right);
    }
    left
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::row_text;

    #[test]
    fn keeps_what_fits_most_important_first() {
        let notices = [
            Notice::good("Update installed! Restart to apply"),
            Notice::warn("Version 13.10 isn't supported · wait for an update"),
        ];
        let wide = row_text(&status_row(Row::new(), &notices, 140));
        assert!(wide.contains("Restart to apply") && wide.contains("isn't supported"));
        let narrow = row_text(&status_row(Row::new(), &notices, 45));
        assert!(narrow.contains("Restart to apply") && !narrow.contains("isn't supported"));
    }

    #[test]
    fn a_job_shows_its_seconds() {
        let row = job_left(
            &crate::spinner::Classic::default(),
            "Running /scan",
            Duration::from_secs(3),
        );
        assert_eq!(row_text(&row), "  - Running /scan… (3s)");
    }
}
