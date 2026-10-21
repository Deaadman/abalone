//! The logo at the left of the header. It may animate: the shell shows
//! frame 0 at launch and one more every tick, then stays on the last.
//!
//! [`DefaultLogo`] is a small terminal window. For your own, implement
//! [`Logo`], or use [`TextLogo`] for fixed rows and [`crate::half_blocks`]
//! for pixel art.

use crate::style::{Row, Style, half_blocks};
use crate::theme::theme;

pub trait Logo {
    /// How many frames the launch animation has; 1 for a still logo.
    fn frames(&self) -> usize {
        1
    }

    /// The logo at `frame` (`frames() - 1` and later is the resting logo).
    /// Every row should be the same width.
    fn rows(&self, frame: usize) -> Vec<Row>;
}

/// A terminal window with a prompt in it, in the theme's colours.
#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultLogo;

impl Logo for DefaultLogo {
    fn frames(&self) -> usize {
        6
    }

    fn rows(&self, frame: usize) -> Vec<Row> {
        let t = theme();
        let (frame_style, prompt) = (Style::fg(t.accent), Style::default().bold());
        // The cursor blinks twice, then stays.
        let cursor = if frame >= 5 || frame.is_multiple_of(2) {
            "▁"
        } else {
            " "
        };
        vec![
            vec![(frame_style, "╭─────╮".into())],
            vec![
                (frame_style, "│ ".into()),
                (prompt, "❯".into()),
                (Style::fg(t.muted), cursor.into()),
                (frame_style, "  │".into()),
            ],
            vec![(frame_style, "╰─────╯".into())],
        ]
    }
}

/// Fixed rows, for a logo drawn by hand.
#[derive(Debug, Clone, Default)]
pub struct TextLogo(pub Vec<Row>);

impl TextLogo {
    /// Plain lines, all in one style.
    pub fn lines(lines: &[&str], style: Style) -> Self {
        TextLogo(lines.iter().map(|l| vec![(style, (*l).to_owned())]).collect())
    }

    /// A block of text, one row per line, as from `include_str!("logo.txt")`.
    pub fn text(text: &str, style: Style) -> Self {
        TextLogo(text.lines().map(|l| vec![(style, l.to_owned())]).collect())
    }
}

impl Logo for TextLogo {
    fn rows(&self, _frame: usize) -> Vec<Row> {
        self.0.clone()
    }
}

/// Pixel art in half blocks, one image per frame.
#[derive(Debug, Clone, Default)]
pub struct PixelLogo {
    pub frames: Vec<Vec<Vec<Option<crossterm::style::Color>>>>,
}

impl Logo for PixelLogo {
    fn frames(&self) -> usize {
        self.frames.len().max(1)
    }

    fn rows(&self, frame: usize) -> Vec<Row> {
        match self.frames.get(frame.min(self.frames.len().saturating_sub(1))) {
            Some(px) => half_blocks(px),
            None => vec![],
        }
    }
}

/// No logo: the header is just its text.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoLogo;

impl Logo for NoLogo {
    fn rows(&self, _frame: usize) -> Vec<Row> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{row_text, row_width};

    #[test]
    fn default_logo_rows_line_up() {
        for f in 0..DefaultLogo.frames() + 2 {
            let rows = DefaultLogo.rows(f);
            assert_eq!(rows.len(), 3);
            assert!(
                rows.iter().all(|r| row_width(r) == 7),
                "{:?}",
                rows.iter().map(row_text).collect::<Vec<_>>()
            );
        }
    }
}
