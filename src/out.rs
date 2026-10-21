//! Where command output goes: stdout and stderr, or the shell's transcript
//! while the shell runs a command. Commands print through here (or the
//! [`outln!`](crate::outln) and [`errln!`](crate::errln) macros) and work the
//! same from the command line and inside the shell.

use std::sync::Mutex;
use std::sync::mpsc::Sender;

use crossterm::style::Color;

use crate::style::Style;
use crate::theme::theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Ordinary output (stdout).
    Out,
    /// Warnings and notes (stderr).
    Warn,
    /// A failed command.
    Error,
    /// Hints, in the muted colour.
    Dim,
    /// Section titles.
    Bold,
    /// Something that worked.
    Good,
    /// Any colour, bold or not.
    Paint(Color, bool),
}

impl Kind {
    /// How the transcript draws it, in the active theme.
    pub fn style(self) -> Style {
        let t = theme();
        match self {
            Kind::Out => Style::default(),
            Kind::Warn => Style::fg(t.warn),
            Kind::Error => Style::fg(t.danger),
            Kind::Dim => Style::fg(t.muted),
            Kind::Bold => Style::default().bold(),
            Kind::Good => Style::fg(t.good),
            Kind::Paint(c, true) => Style::fg(c).bold(),
            Kind::Paint(c, false) => Style::fg(c),
        }
    }

    /// Whether it goes to stdout outside the shell (else stderr).
    fn stdout(self) -> bool {
        !matches!(self, Kind::Warn | Kind::Error)
    }
}

/// One transcript line: pieces in their own styles.
pub type Line = Vec<(Kind, String)>;

static SINK: Mutex<Option<Sender<Line>>> = Mutex::new(None);

/// Where warnings and errors are also logged.
pub type Log = fn(Kind, &str);

static LOG: Mutex<Option<Log>> = Mutex::new(None);

/// Send output to `tx` instead of the terminal; `None` restores the terminal.
pub fn capture(tx: Option<Sender<Line>>) {
    if let Ok(mut sink) = SINK.lock() {
        *sink = tx;
    }
}

/// Whether output is going to the shell's transcript.
pub fn captured() -> bool {
    SINK.lock().is_ok_and(|s| s.is_some())
}

/// Also hand warnings and errors to `log` (a session log, say).
pub fn set_log(log: Option<Log>) {
    if let Ok(mut l) = LOG.lock() {
        *l = log;
    }
}

/// Shows `text`; warnings and errors also go to the log set with [`set_log`].
pub fn emit(kind: Kind, text: String) {
    if matches!(kind, Kind::Warn | Kind::Error)
        && let Some(log) = LOG.lock().ok().and_then(|l| *l)
    {
        log(kind, &text);
    }
    show(kind, text);
}

/// Shows `text` without logging it (the user's own typing, echoed back).
pub fn show(kind: Kind, text: String) {
    let sink = SINK.lock().ok().and_then(|s| s.clone());
    match sink {
        Some(tx) => {
            for line in text.split('\n') {
                let _ = tx.send(vec![(kind, plain(line))]);
            }
        }
        None if kind.stdout() => println!("{text}"),
        None => eprintln!("{text}"),
    }
}

/// One line built from styled pieces; plain text on stdout outside the shell.
pub fn row(pieces: Line) {
    let sink = SINK.lock().ok().and_then(|s| s.clone());
    match sink {
        Some(tx) => {
            let _ = tx.send(pieces.into_iter().map(|(k, t)| (k, plain(&t))).collect());
        }
        None => println!("{}", pieces.into_iter().map(|(_, t)| t).collect::<String>()),
    }
}

/// `text` safe to draw in the shell: terminal escape sequences (colours a
/// tool printed, cursor moves) and other control characters taken out, and
/// tabs turned into spaces, so they can't move the cursor or throw the
/// layout off.
pub fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\x1b' => match chars.next() {
                // CSI: parameters and intermediates up to a final byte.
                Some('[') => {
                    for c in chars.by_ref() {
                        if ('\x40'..='\x7e').contains(&c) {
                            break;
                        }
                    }
                }
                // OSC, DCS and the like: up to BEL or ESC \.
                Some(']' | 'P' | 'X' | '^' | '_') => {
                    while let Some(c) = chars.next() {
                        if c == '\x07' {
                            break;
                        }
                        if c == '\x1b' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                // Two-character escapes.
                _ => {}
            },
            '\t' => out.push_str("    "),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// `println!`, or a transcript line in the shell.
#[macro_export]
macro_rules! outln {
    () => {
        $crate::out::emit($crate::out::Kind::Out, String::new())
    };
    ($($t:tt)*) => {
        $crate::out::emit($crate::out::Kind::Out, format!($($t)*))
    };
}

/// `eprintln!`, or a warning line in the shell.
#[macro_export]
macro_rules! errln {
    ($($t:tt)*) => {
        $crate::out::emit($crate::out::Kind::Warn, format!($($t)*))
    };
}

#[cfg(test)]
mod tests {
    use super::plain;

    #[test]
    fn plain_takes_out_escapes_and_controls() {
        assert_eq!(plain("\x1b[1;31mred\x1b[0m done\r"), "red done");
        assert_eq!(plain("\x1b]0;title\x07a\x1b]8;;url\x1b\\b"), "ab");
        assert_eq!(plain("a\tb\x08c"), "a    bc");
        assert_eq!(plain("日本 ok"), "日本 ok");
    }
}
