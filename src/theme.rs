//! The colours every component draws with, by role. One theme is active for
//! the whole process: set it once at startup with [`set_theme`] (or
//! `ShellBuilder::theme`), and components read it with [`theme`].
//!
//! ```
//! use abalone::{Theme, rgb};
//! let mine = Theme { accent: rgb((255, 140, 0)), ..Theme::default() };
//! abalone::set_theme(mine);
//! assert_eq!(abalone::theme().accent, rgb((255, 140, 0)));
//! ```

use std::sync::RwLock;

use crossterm::style::Color;

use crate::style::rgb;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// The highlighted menu row, command names, panel titles and their rules.
    pub accent: Color,
    /// Strong text: the selected tab, matched letters in the menu, echoed lines.
    pub bright: Color,
    /// Hints, labels, the prompt and the footer.
    pub muted: Color,
    /// The rules around the input box.
    pub rule: Color,
    /// Errors and problems.
    pub danger: Color,
    /// Warnings and values that still need setting.
    pub warn: Color,
    /// Something that worked; switches that are on.
    pub good: Color,
    /// Behind a line the user sent, in the transcript.
    pub echo_bg: Color,
    /// The text on the selected tab and the text field's cursor (drawn on `bright`).
    pub on_bright: Color,
}

impl Theme {
    /// Neutral greys with a soft blue accent.
    pub const DEFAULT: Theme = Theme {
        accent: Color::Rgb { r: 122, g: 162, b: 247 },
        bright: Color::Rgb { r: 235, g: 235, b: 235 },
        muted: Color::Rgb { r: 140, g: 140, b: 140 },
        rule: Color::Rgb { r: 100, g: 100, b: 100 },
        danger: Color::Rgb { r: 240, g: 90, b: 90 },
        warn: Color::Rgb { r: 230, g: 190, b: 80 },
        good: Color::Rgb { r: 120, g: 200, b: 120 },
        echo_bg: Color::Rgb { r: 45, g: 45, b: 45 },
        on_bright: Color::Black,
    };

    /// A theme from TOML: any of the roles as `"#rrggbb"`, the rest from `base`.
    ///
    /// ```toml
    /// accent = "#ff8c00"
    /// danger = "#ff4655"
    /// ```
    #[cfg(feature = "theme-file")]
    pub fn from_toml(text: &str, base: Theme) -> anyhow::Result<Theme> {
        use anyhow::Context;

        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct File {
            accent: Option<String>,
            bright: Option<String>,
            muted: Option<String>,
            rule: Option<String>,
            danger: Option<String>,
            warn: Option<String>,
            good: Option<String>,
            echo_bg: Option<String>,
            on_bright: Option<String>,
        }
        let f: File = toml::from_str(text).context("reading the theme")?;
        let pick = |v: Option<String>, d: Color, name: &str| -> anyhow::Result<Color> {
            match v {
                Some(s) => hex(&s).with_context(|| format!("{name} = {s:?} isn't a colour like \"#ff8c00\"")),
                None => Ok(d),
            }
        };
        Ok(Theme {
            accent: pick(f.accent, base.accent, "accent")?,
            bright: pick(f.bright, base.bright, "bright")?,
            muted: pick(f.muted, base.muted, "muted")?,
            rule: pick(f.rule, base.rule, "rule")?,
            danger: pick(f.danger, base.danger, "danger")?,
            warn: pick(f.warn, base.warn, "warn")?,
            good: pick(f.good, base.good, "good")?,
            echo_bg: pick(f.echo_bg, base.echo_bg, "echo_bg")?,
            on_bright: pick(f.on_bright, base.on_bright, "on_bright")?,
        })
    }
}

impl Default for Theme {
    fn default() -> Self {
        Theme::DEFAULT
    }
}

/// `#rrggbb` (or `rrggbb`) as a colour.
pub fn hex(s: &str) -> Option<Color> {
    let s = s.trim();
    let s = s.strip_prefix('#').unwrap_or(s);
    if s.len() != 6 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let part = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).ok();
    Some(rgb((part(0)?, part(2)?, part(4)?)))
}

static THEME: RwLock<Theme> = RwLock::new(Theme::DEFAULT);

/// The active theme.
pub fn theme() -> Theme {
    *THEME.read().unwrap_or_else(|e| e.into_inner())
}

/// Make `t` the active theme, for everything drawn from now on.
pub fn set_theme(t: Theme) {
    *THEME.write().unwrap_or_else(|e| e.into_inner()) = t;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_hex_colours() {
        assert_eq!(hex("#ff8c00"), Some(rgb((255, 140, 0))));
        assert_eq!(hex("00ff00"), Some(rgb((0, 255, 0))));
        assert_eq!(hex("#fff"), None);
        assert_eq!(hex("#gg0000"), None);
        assert_eq!(hex("#+f+f+f"), None);
        assert_eq!(hex("##ff8c00"), None);
        assert_eq!(hex("#-10000"), None);
    }

    #[cfg(feature = "theme-file")]
    #[test]
    fn a_theme_file_overrides_some_roles() {
        let t = Theme::from_toml("accent = \"#ff8c00\"", Theme::DEFAULT).unwrap();
        assert_eq!(t.accent, rgb((255, 140, 0)));
        assert_eq!(t.muted, Theme::DEFAULT.muted);
        assert!(Theme::from_toml("accent = \"orange\"", Theme::DEFAULT).is_err());
        assert!(Theme::from_toml("acent = \"#ffffff\"", Theme::DEFAULT).is_err());
    }
}
