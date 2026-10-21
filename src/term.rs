//! The terminal: raw mode, the alternate screen, and printing rows.

use std::io::{self, Write};

use crossterm::cursor::Show;
use crossterm::event;
use crossterm::style::{Attribute, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen, SetTitle};
use crossterm::{execute, queue};

use crate::style::{Row, fit_width, text_width};
use crate::theme::readable;

/// Raw mode with bracketed paste, titled, on the alternate screen or the
/// terminal's own; all of it is restored (the old title too) however the
/// shell ends.
pub struct Screen {
    old_title: Option<String>,
    alt: bool,
}

impl Screen {
    /// `alt`: on the alternate screen, which the terminal swaps back out at
    /// the end; otherwise in the terminal's own scrollback.
    pub fn on(title: &str, alt: bool) -> io::Result<Self> {
        let old_title = console_title();
        if old_title.is_none() {
            // Terminals elsewhere keep a stack of titles: push the current one.
            let _ = io::stdout().write_all(b"\x1b[22;0t");
        }
        execute!(io::stdout(), SetTitle(title))?;
        let mut screen = Screen { old_title, alt: false };
        screen.set_alt(alt)?;
        terminal::enable_raw_mode()?;
        let _ = execute!(io::stdout(), event::EnableBracketedPaste);
        Ok(screen)
    }

    pub fn alt(&self) -> bool {
        self.alt
    }

    /// Onto the alternate screen, or back to the terminal's own (which
    /// comes back as it was, cursor and all).
    pub fn set_alt(&mut self, on: bool) -> io::Result<()> {
        if on != self.alt {
            if on {
                execute!(io::stdout(), EnterAlternateScreen)?;
            } else {
                execute!(io::stdout(), LeaveAlternateScreen)?;
            }
            self.alt = on;
        }
        Ok(())
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), event::DisableBracketedPaste);
        let _ = terminal::disable_raw_mode();
        let _ = execute!(io::stdout(), Show);
        let _ = self.set_alt(false);
        match &self.old_title {
            Some(t) => {
                let _ = execute!(io::stdout(), SetTitle(t));
            }
            None => {
                let _ = io::stdout().write_all(b"\x1b[23;0t");
                let _ = io::stdout().flush();
            }
        }
    }
}

/// The console window's title now, so it can be restored. Only Windows can
/// say; other terminals push and pop it instead.
#[cfg(windows)]
fn console_title() -> Option<String> {
    let mut buf = [0u16; 1024];
    // SAFETY: the buffer outlives the call and its length is passed with it.
    let n = unsafe { windows_sys::Win32::System::Console::GetConsoleTitleW(buf.as_mut_ptr(), buf.len() as u32) };
    (n > 0).then(|| String::from_utf16_lossy(&buf[..(n as usize).min(buf.len())]))
}

#[cfg(not(windows))]
fn console_title() -> Option<String> {
    None
}

/// Whether stdin and stdout are both a terminal: run the shell then, and
/// print help or plain output when either is piped.
pub fn interactive() -> bool {
    use std::io::IsTerminal;
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

/// Print a row clipped to `w` columns (one short, so the last column never wraps).
///
/// On a light background, text with no background of its own is darkened
/// until it reads there (see `readable`), so an app's colours chosen for dark
/// terminals still work. Block characters are pictures (images, logos), not
/// text, and keep their colours.
pub fn print_row(out: &mut impl Write, row: &Row, w: usize) -> io::Result<()> {
    let mut left = w.saturating_sub(1);
    for (style, text) in row {
        if left == 0 {
            break;
        }
        let piece = fit_width(text, left);
        left -= text_width(piece);
        let picture = piece.chars().any(|c| matches!(c, '▀' | '▄' | '█'));
        if let Some(c) = style.fg {
            let c = if style.bg.is_none() && !picture { readable(c) } else { c };
            queue!(out, SetForegroundColor(c))?;
        }
        if let Some(c) = style.bg {
            queue!(out, SetBackgroundColor(c))?;
        }
        if style.bold {
            queue!(out, SetAttribute(Attribute::Bold))?;
        }
        queue!(out, Print(piece), SetAttribute(Attribute::Reset), ResetColor)?;
    }
    Ok(())
}
