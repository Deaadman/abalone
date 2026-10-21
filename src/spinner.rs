//! What turns at the left of the status bar while a command runs.
//!
//! The default is the classic `| / - \`. Any `Fn(Duration) -> (Style, String)`
//! is a spinner too, so a custom one is a closure:
//!
//! ```
//! use std::time::Duration;
//! use shellkit::{Spinner, Style};
//! let dots = |t: Duration| (Style::default(), ".".repeat(1 + (t.as_millis() / 300 % 3) as usize));
//! assert_eq!(dots.frame(Duration::ZERO).1, ".");
//! ```

use std::time::Duration;

use crate::style::Style;
use crate::theme::theme;

pub trait Spinner {
    /// The glyph `elapsed` into the command, and its style.
    fn frame(&self, elapsed: Duration) -> (Style, String);
}

impl<F: Fn(Duration) -> (Style, String)> Spinner for F {
    fn frame(&self, elapsed: Duration) -> (Style, String) {
        self(elapsed)
    }
}

/// `| / - \`, a step every `step`.
#[derive(Debug, Clone, Copy)]
pub struct Classic {
    pub step: Duration,
}

impl Default for Classic {
    fn default() -> Self {
        Classic {
            step: Duration::from_millis(100),
        }
    }
}

impl Spinner for Classic {
    fn frame(&self, elapsed: Duration) -> (Style, String) {
        const GLYPHS: [&str; 4] = ["|", "/", "-", "\\"];
        let i = elapsed.as_millis() / self.step.as_millis().max(1);
        (Style::fg(theme().accent), GLYPHS[i as usize % 4].to_owned())
    }
}

/// Glyphs shown in turn, a step each: `Frames::new(&["⠋", "⠙", "⠹", "⠸"])`.
#[derive(Debug, Clone)]
pub struct Frames {
    pub glyphs: Vec<String>,
    pub step: Duration,
    /// The style; the theme's accent when unset.
    pub style: Option<Style>,
}

impl Frames {
    pub fn new(glyphs: &[&str]) -> Self {
        Frames {
            glyphs: glyphs.iter().map(|g| (*g).to_owned()).collect(),
            step: Duration::from_millis(80),
            style: None,
        }
    }
}

impl Spinner for Frames {
    fn frame(&self, elapsed: Duration) -> (Style, String) {
        let i = elapsed.as_millis() / self.step.as_millis().max(1);
        let glyph = self
            .glyphs
            .get(i as usize % self.glyphs.len().max(1))
            .cloned()
            .unwrap_or_default();
        (self.style.unwrap_or(Style::fg(theme().accent)), glyph)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classic_turns() {
        let s = Classic::default();
        let at = |ms| s.frame(Duration::from_millis(ms)).1;
        assert_eq!([at(0), at(100), at(200), at(300), at(400)], ["|", "/", "-", "\\", "|"]);
    }

    #[test]
    fn frames_cycle() {
        let s = Frames::new(&["a", "b"]);
        assert_eq!(s.frame(Duration::from_millis(80)).1, "b");
        assert_eq!(s.frame(Duration::from_millis(160)).1, "a");
    }
}
