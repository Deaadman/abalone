//! The `/help` panel: tabs of text and a browsable list of the commands,
//! drawn in place of the input box until Esc.
//!
//! ```
//! use shellkit::{Help, HelpTab};
//! let help = Help::new()
//!     .text("General", "My app does things.\n\n# Getting started\n  /init sets it up.")
//!     .commands("Commands")
//!     .link("https://example.com/my-app")
//!     .issues("https://example.com/my-app/issues");
//! assert_eq!(help.tabs.len(), 2);
//! ```

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::editor::CommandInfo;
use crate::style::{Row, Style, wrap};
use crate::theme::theme;
use crate::widgets::{accent_rule, title_with_tabs};

/// The keys the shell itself handles, added to the end of the first text tab.
pub const SHELL_KEYS: &str = "# Keys
  Enter            run the line, or the highlighted command
  Tab, →           complete the highlighted suggestion
  ↑ ↓              move in the menu; otherwise walk history
  Esc              hide the menu, or close a panel
  PgUp PgDn        scroll the output
  Ctrl+L           clear the screen
  Ctrl+C           clear the line; twice on an empty line quits";

/// One line of a text tab; long ones wrap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpLine {
    pub bold: bool,
    pub text: String,
}

/// Lines from text: `# Title` is a bold heading, everything else as written.
pub fn help_lines(text: &str) -> Vec<HelpLine> {
    text.lines()
        .map(|l| match l.strip_prefix("# ") {
            Some(h) => HelpLine {
                bold: true,
                text: h.to_owned(),
            },
            None => HelpLine {
                bold: false,
                text: l.to_owned(),
            },
        })
        .collect()
}

pub enum HelpTab {
    /// Text that scrolls.
    Text { name: String, lines: Vec<HelpLine> },
    /// The commands, two rows each; Enter puts one in the input line.
    Commands { name: String },
    /// Rows of your own for a `w`-column screen; they scroll like text.
    Custom {
        name: String,
        rows: Box<dyn Fn(usize) -> Vec<Row>>,
    },
}

impl HelpTab {
    pub fn name(&self) -> &str {
        match self {
            HelpTab::Text { name, .. } | HelpTab::Commands { name } | HelpTab::Custom { name, .. } => name,
        }
    }
}

/// What the help panel shows.
pub struct Help {
    pub title: String,
    pub tabs: Vec<HelpTab>,
    /// Shown as "For more help: …".
    pub more: Option<String>,
    /// Shown in the hint as "something wrong? …".
    pub issues: Option<String>,
    /// Add [`SHELL_KEYS`] to the first text tab.
    pub keys: bool,
}

impl Default for Help {
    /// The shell's keys, then the commands.
    fn default() -> Self {
        Help::new().text("General", "").commands("Commands")
    }
}

impl Help {
    /// No tabs yet; add them with [`Help::text`], [`Help::commands`] and [`Help::tab`].
    pub fn new() -> Self {
        Help {
            title: "Help".into(),
            tabs: vec![],
            more: None,
            issues: None,
            keys: true,
        }
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// A text tab; see [`help_lines`] for the format.
    pub fn text(mut self, name: impl Into<String>, text: &str) -> Self {
        let mut lines = help_lines(text);
        while lines.first().is_some_and(|l| l.text.is_empty()) {
            lines.remove(0);
        }
        self.tabs.push(HelpTab::Text {
            name: name.into(),
            lines,
        });
        self
    }

    pub fn commands(mut self, name: impl Into<String>) -> Self {
        self.tabs.push(HelpTab::Commands { name: name.into() });
        self
    }

    pub fn tab(mut self, tab: HelpTab) -> Self {
        self.tabs.push(tab);
        self
    }

    pub fn link(mut self, url: impl Into<String>) -> Self {
        self.more = Some(url.into());
        self
    }

    pub fn issues(mut self, url: impl Into<String>) -> Self {
        self.issues = Some(url.into());
        self
    }

    /// Leave the shell's keys out of the first text tab.
    pub fn without_keys(mut self) -> Self {
        self.keys = false;
        self
    }

    /// A tab's lines with the keys added where they go.
    fn lines(&self, tab: usize) -> Vec<HelpLine> {
        let HelpTab::Text { lines, .. } = &self.tabs[tab] else {
            return vec![];
        };
        let first_text = self.tabs.iter().position(|t| matches!(t, HelpTab::Text { .. }));
        let mut lines = lines.clone();
        if self.keys && first_text == Some(tab) {
            if !lines.is_empty() {
                lines.push(HelpLine {
                    bold: false,
                    text: String::new(),
                });
            }
            lines.extend(help_lines(SHELL_KEYS));
        }
        lines
    }
}

pub enum HelpAction {
    None,
    Close,
    /// Close and put this in the input line.
    Use(String),
}

#[derive(Default)]
pub struct HelpPanel {
    tab: usize,
    /// Highlighted command on a commands tab.
    selected: usize,
    /// First row shown on a text tab.
    scroll: usize,
}

impl HelpPanel {
    fn on_commands(&self, help: &Help) -> bool {
        matches!(help.tabs.get(self.tab), Some(HelpTab::Commands { .. }))
    }

    pub fn key(&mut self, key: KeyEvent, help: &Help, commands: &[CommandInfo]) -> HelpAction {
        let n = commands.len();
        let tabs = help.tabs.len().max(1);
        let list = self.on_commands(help);
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return HelpAction::Close,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return HelpAction::Close,
            KeyCode::Left | KeyCode::BackTab => {
                self.tab = (self.tab + tabs - 1) % tabs;
                self.scroll = 0;
            }
            KeyCode::Right | KeyCode::Tab => {
                self.tab = (self.tab + 1) % tabs;
                self.scroll = 0;
            }
            KeyCode::Up if list => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down if list => self.selected = (self.selected + 1).min(n.saturating_sub(1)),
            KeyCode::PageUp if list => self.selected = self.selected.saturating_sub(5),
            KeyCode::PageDown if list => self.selected = (self.selected + 5).min(n.saturating_sub(1)),
            KeyCode::Home if list => self.selected = 0,
            KeyCode::End if list => self.selected = n.saturating_sub(1),
            KeyCode::Up => self.scroll = self.scroll.saturating_sub(1),
            KeyCode::Down => self.scroll += 1,
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(10),
            KeyCode::PageDown => self.scroll += 10,
            KeyCode::Enter if list => {
                if let Some(c) = commands.get(self.selected) {
                    return HelpAction::Use(format!("/{} ", c.name));
                }
            }
            _ => {}
        }
        HelpAction::None
    }

    /// The panel for a `w`-column screen: as tall as its tallest tab, at most `h` rows.
    pub fn rows(&mut self, help: &Help, commands: &[CommandInfo], w: usize, h: usize) -> Vec<Row> {
        let t = theme();
        let names: Vec<&str> = help.tabs.iter().map(HelpTab::name).collect();
        self.tab = self.tab.min(names.len().saturating_sub(1));
        let top: Vec<Row> = vec![
            accent_rule(w),
            vec![],
            title_with_tabs(&help.title, &names, self.tab),
            vec![],
        ];

        let mut bottom: Vec<Row> = vec![vec![]];
        if let Some(more) = &help.more {
            bottom.push(vec![
                (Style::fg(t.muted), "  For more help: ".into()),
                (Style::default(), more.clone()),
            ]);
            bottom.push(vec![]);
        }
        let keys = if self.on_commands(help) {
            "↑↓ move · Enter uses the command"
        } else {
            "↑↓ scroll"
        };
        let tabs = if names.len() > 1 { " · ←→ tabs" } else { "" };
        let issues = help
            .issues
            .as_ref()
            .map_or(String::new(), |u| format!(" · something wrong? {u}"));
        bottom.push(vec![(
            Style::fg(t.muted),
            format!("  {keys}{tabs} · Esc closes{issues}"),
        )]);

        // As tall as the tallest tab needs, so switching tabs keeps the height.
        let pages: Vec<Option<Vec<Row>>> = (0..help.tabs.len())
            .map(|i| match &help.tabs[i] {
                HelpTab::Text { .. } => Some(text_rows(&help.lines(i), w)),
                HelpTab::Custom { rows, .. } => Some(rows(w)),
                HelpTab::Commands { .. } => None,
            })
            .collect();
        let needed = pages
            .iter()
            .map(|p| p.as_ref().map_or(commands_height(commands.len()), Vec::len))
            .max()
            .unwrap_or(0);
        let body_h = needed.min(h.saturating_sub(top.len() + bottom.len())).max(1);
        let mut body = match pages.into_iter().nth(self.tab) {
            Some(Some(lines)) => self.scrolled(lines, body_h),
            Some(None) => self.commands(commands, body_h),
            None => vec![],
        };
        body.resize(body_h, vec![]);
        let mut rows = top;
        rows.extend(body);
        rows.extend(bottom);
        rows.truncate(h);
        rows
    }

    fn scrolled(&mut self, lines: Vec<Row>, h: usize) -> Vec<Row> {
        let max = lines.len().saturating_sub(h);
        self.scroll = self.scroll.min(max);
        let mut rows: Vec<Row> = lines.into_iter().skip(self.scroll).take(h).collect();
        if self.scroll < max
            && let Some(last) = rows.last_mut()
        {
            *last = vec![(Style::fg(theme().muted), format!("  ↓ {} more", max - self.scroll))];
        }
        rows
    }

    /// Two rows per command (name, then what it does), the highlighted one kept in view.
    fn commands(&mut self, commands: &[CommandInfo], h: usize) -> Vec<Row> {
        let t = theme();
        let n = commands.len();
        self.selected = self.selected.min(n.saturating_sub(1));
        let mut rows: Vec<Row> = vec![vec![(Style::default(), "  Browse commands".into())]];
        // Everything, or as many as fit with a row for each "more" line.
        let fit = if commands_height(n) <= h {
            n
        } else {
            (h.saturating_sub(3) / 2).max(1)
        };
        let first = self
            .selected
            .saturating_sub(fit.saturating_sub(1))
            .min(n.saturating_sub(fit));
        if first > 0 {
            rows.push(vec![(Style::fg(t.muted), format!("    ↑ {first} more"))]);
        } else {
            rows.push(vec![]);
        }
        for (i, c) in commands.iter().enumerate().skip(first).take(fit) {
            let (lead, style) = if i == self.selected {
                ("  ❯ ", Style::fg(t.accent))
            } else {
                ("    ", Style::default())
            };
            rows.push(vec![(style, lead.into()), (style, format!("/{}", c.name))]);
            rows.push(vec![(Style::fg(t.muted), format!("    {}", c.about))]);
        }
        let below = n.saturating_sub(first + fit);
        if below > 0 {
            rows.push(vec![(Style::fg(t.muted), format!("    ↓ {below} more"))]);
        }
        rows
    }
}

/// A text tab wrapped to the screen.
fn text_rows(lines: &[HelpLine], w: usize) -> Vec<Row> {
    let text_w = w.saturating_sub(6).max(10);
    let mut rows = Vec::new();
    for l in lines {
        let style = if l.bold {
            Style::default().bold()
        } else {
            Style::default()
        };
        for piece in wrap(&l.text, text_w) {
            rows.push(vec![(style, format!("  {piece}"))]);
        }
    }
    rows
}

/// Rows a commands tab needs to list `n` commands: a title, the row above
/// the list, and two rows per command.
fn commands_height(n: usize) -> usize {
    2 + 2 * n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::row_text;
    use crossterm::event::KeyEventKind;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            kind: KeyEventKind::Press,
            ..KeyEvent::new(code, KeyModifiers::NONE)
        }
    }

    fn commands(n: usize) -> Vec<CommandInfo> {
        (0..n)
            .map(|i| CommandInfo::new(format!("cmd{i}"), format!("does {i}")))
            .collect()
    }

    fn text(rows: &[Row]) -> String {
        rows.iter().map(row_text).collect::<Vec<_>>().join("\n")
    }

    fn help() -> Help {
        Help::new()
            .text("General", "What it does.\n\n# Section\n  more")
            .commands("Commands")
            .link("https://example.com")
            .issues("https://example.com/issues")
    }

    #[test]
    fn fits_its_content_and_the_screen() {
        // Few commands: as tall as the General text needs, on either tab, with no room to spare.
        let (help, c) = (help(), commands(3));
        let mut p = HelpPanel::default();
        let natural = p.rows(&help, &c, 80, 200).len();
        assert!(natural < 200);
        p.key(key(KeyCode::Right), &help, &c);
        assert_eq!(p.rows(&help, &c, 80, 200).len(), natural);
        // Many commands: capped to the screen, never taller.
        let c = commands(60);
        for h in [8, 14, 40] {
            assert!(p.rows(&help, &c, 80, h).len() <= h);
        }
        assert_eq!(p.rows(&help, &c, 80, 40).len(), 40);
    }

    #[test]
    fn keeps_the_selected_command_in_view_and_uses_it() {
        let (help, c) = (help(), commands(30));
        let mut p = HelpPanel::default();
        p.key(key(KeyCode::Tab), &help, &c);
        for _ in 0..20 {
            p.key(key(KeyCode::Down), &help, &c);
        }
        let shown = text(&p.rows(&help, &c, 80, 20));
        assert!(shown.contains("❯ /cmd20"));
        assert!(shown.contains("↑") && shown.contains("↓"));
        assert!(matches!(p.key(key(KeyCode::Enter), &help, &c), HelpAction::Use(s) if s == "/cmd20 "));
        assert!(matches!(p.key(key(KeyCode::Esc), &help, &c), HelpAction::Close));
    }

    #[test]
    fn the_first_text_tab_ends_with_the_keys() {
        let (help, c) = (help(), commands(1));
        let shown = text(&HelpPanel::default().rows(&help, &c, 80, 200));
        assert!(shown.contains("  What it does.") && shown.contains("  Section"));
        assert!(shown.contains("Ctrl+L") && shown.contains("For more help: https://example.com"));
        let shown = text(&HelpPanel::default().rows(&help.without_keys(), &c, 80, 200));
        assert!(!shown.contains("Ctrl+L"));
    }
}
