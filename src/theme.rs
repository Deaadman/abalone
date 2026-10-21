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
//!
//! The colours are picked for a dark background. On a light one the shell
//! uses [`Theme::on_light`] of the default theme; an app with its own
//! colours passes them through [`Theme::for_terminal`], and the colours of
//! its own panels through [`readable`].

use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use crossterm::style::Color;

use crate::style::rgb;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// The highlighted menu row, command names, panel titles and their rules.
    pub accent: Color,
    /// Strong text: the selected tab and echoed lines, both on a background
    /// of their own. (Matched letters in the menu are bold in the terminal's
    /// own text colour, which reads on any background.)
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

    /// This theme for a light background: `bright` turns near black (with
    /// `on_bright` white), `echo_bg` light, and the other colours darken
    /// until they read on white.
    pub fn on_light(self) -> Theme {
        let bright = if contrast(self.bright, WHITE) >= TEXT {
            self.bright
        } else {
            Color::Rgb { r: 30, g: 30, b: 30 }
        };
        let echo_bg = match self.echo_bg {
            Color::Rgb { r, g, b } if luminance((r, g, b)) < 0.5 => Color::Rgb {
                r: 255 - r,
                g: 255 - g,
                b: 255 - b,
            },
            c => c,
        };
        Theme {
            accent: for_light(self.accent),
            bright,
            muted: for_light(self.muted),
            rule: darken(self.rule, LINE),
            danger: for_light(self.danger),
            warn: for_light(self.warn),
            good: for_light(self.good),
            echo_bg,
            on_bright: if bright == self.bright {
                self.on_bright
            } else {
                Color::White
            },
        }
    }

    /// This theme, or [`Theme::on_light`] of it when the terminal's
    /// background is light ([`light_background`]).
    pub fn for_terminal(self) -> Theme {
        if light_background() { self.on_light() } else { self }
    }

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
/// Whether the app chose a theme, so the shell leaves it be.
static CHOSEN: AtomicBool = AtomicBool::new(false);

/// The active theme.
pub fn theme() -> Theme {
    *THEME.read().unwrap_or_else(|e| e.into_inner())
}

/// Make `t` the active theme, for everything drawn from now on.
pub fn set_theme(t: Theme) {
    CHOSEN.store(true, Ordering::Relaxed);
    *THEME.write().unwrap_or_else(|e| e.into_inner()) = t;
}

/// The default theme for this terminal's background, unless the app chose one.
pub(crate) fn default_for_terminal() {
    if !CHOSEN.load(Ordering::Relaxed) {
        *THEME.write().unwrap_or_else(|e| e.into_inner()) = Theme::DEFAULT.for_terminal();
    }
}

/// Text needs this much contrast with the background (WCAG AA), lines less.
const TEXT: f64 = 4.5;
const LINE: f64 = 3.0;
const WHITE: Color = Color::Rgb { r: 255, g: 255, b: 255 };

/// Unknown (0), dark (1) or light (2); see [`light_background`].
static BACKGROUND: AtomicU8 = AtomicU8::new(0);

/// Whether the terminal's background is light: as set with
/// [`set_light_background`], else as the terminal says (`COLORFGBG`, or the
/// console's colours on Windows). Terminals that say nothing, Windows
/// Terminal among them, count as dark.
pub fn light_background() -> bool {
    match BACKGROUND.load(Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            let light = detect_light();
            BACKGROUND.store(if light { 2 } else { 1 }, Ordering::Relaxed);
            light
        }
    }
}

/// Say whether the background is light, for terminals that can't tell (from
/// a setting of the app's, say). Colours picked after this follow it: call
/// it before [`Theme::for_terminal`] and [`readable`].
pub fn set_light_background(light: bool) {
    BACKGROUND.store(if light { 2 } else { 1 }, Ordering::Relaxed);
}

/// `c`, darkened on a light background until it reads there. For colours an
/// app draws with itself: `Style::fg(abalone::readable(MY_GREEN))`.
pub fn readable(c: Color) -> Color {
    if light_background() { for_light(c) } else { c }
}

/// `c` darkened until text in it reads on white. Named colours are left to
/// the terminal's palette.
pub fn for_light(c: Color) -> Color {
    darken(c, TEXT)
}

/// `c` scaled toward black until it has `ratio` contrast with white.
fn darken(c: Color, ratio: f64) -> Color {
    let Color::Rgb { r, g, b } = c else {
        return c;
    };
    let mut k = 1.0;
    loop {
        let scaled = |v: u8| (f64::from(v) * k).round() as u8;
        let out = Color::Rgb {
            r: scaled(r),
            g: scaled(g),
            b: scaled(b),
        };
        if k <= 0.0 || contrast(out, WHITE) >= ratio {
            return out;
        }
        k = (k - 0.02f64).max(0.0);
    }
}

/// WCAG relative luminance.
fn luminance((r, g, b): (u8, u8, u8)) -> f64 {
    let lin = |v: u8| {
        let v = f64::from(v) / 255.0;
        if v <= 0.040_45 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
}

/// WCAG contrast ratio; named colours other than black and white count as
/// mid grey.
fn contrast(a: Color, b: Color) -> f64 {
    let l = |c: Color| match c {
        Color::Rgb { r, g, b } => luminance((r, g, b)),
        Color::Black => 0.0,
        Color::White => 1.0,
        _ => 0.2,
    };
    let (x, y) = (l(a), l(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

fn detect_light() -> bool {
    std::env::var("COLORFGBG")
        .ok()
        .and_then(|v| colorfgbg_light(&v))
        .or_else(console_light)
        .unwrap_or(false)
}

/// `COLORFGBG` is `fg;bg` (or `fg;default;bg`) in ANSI colour numbers: a
/// white or light grey background is light.
fn colorfgbg_light(v: &str) -> Option<bool> {
    let bg: u8 = v.rsplit(';').next()?.trim().parse().ok()?;
    Some(matches!(bg, 7 | 15))
}

/// The console's background colour, which the classic Windows console
/// reports (Windows Terminal reports black whatever it shows).
#[cfg(windows)]
fn console_light() -> Option<bool> {
    use windows_sys::Win32::System::Console::{
        CONSOLE_SCREEN_BUFFER_INFO, GetConsoleScreenBufferInfo, GetStdHandle, STD_OUTPUT_HANDLE,
    };
    // SAFETY: the struct is plain data that the call fills in, through a
    // pointer that outlives the call.
    unsafe {
        let mut info: CONSOLE_SCREEN_BUFFER_INFO = std::mem::zeroed();
        if GetConsoleScreenBufferInfo(GetStdHandle(STD_OUTPUT_HANDLE), &mut info) == 0 {
            return None;
        }
        Some(matches!((info.wAttributes >> 4) & 0xF, 7 | 15))
    }
}

#[cfg(not(windows))]
fn console_light() -> Option<bool> {
    None
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

    #[test]
    fn reads_colorfgbg() {
        assert_eq!(colorfgbg_light("0;15"), Some(true));
        assert_eq!(colorfgbg_light("0;default;7"), Some(true));
        assert_eq!(colorfgbg_light("15;0"), Some(false));
        assert_eq!(colorfgbg_light("15;default"), None);
    }

    #[test]
    fn the_light_theme_reads_on_white() {
        let t = Theme::DEFAULT.on_light();
        for c in [t.accent, t.bright, t.muted, t.danger, t.warn, t.good] {
            assert!(contrast(c, WHITE) >= TEXT, "{c:?}");
        }
        assert!(contrast(t.rule, WHITE) >= LINE);
        assert!(
            contrast(t.bright, t.echo_bg) >= TEXT,
            "the echo's text on its background"
        );
        assert!(contrast(t.on_bright, t.bright) >= TEXT, "the selected tab");
        // Colours dark enough already, and named ones, stay as they are.
        assert_eq!(for_light(rgb((20, 20, 120))), rgb((20, 20, 120)));
        assert_eq!(for_light(Color::Yellow), Color::Yellow);
        // A pastel darkens but stays the same hue.
        let Color::Rgb { r, g, b } = for_light(rgb((177, 185, 249))) else {
            unreachable!()
        };
        assert!(b > r && b > g && contrast(rgb((r, g, b)), WHITE) >= TEXT);
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
