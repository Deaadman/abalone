//! The line editor behind the input box: the buffer, history, the command
//! menu and key handling. No terminal I/O here, so it is unit-tested.
//!
//! Lines that start with `/` name a command. The menu offers command names,
//! then each command's flags; a [`Completer`] adds values (for a flag, or
//! the command's own arguments).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// One command the shell knows, with its flags.
#[derive(Debug, Clone, Default)]
pub struct CommandInfo {
    pub name: String,
    pub about: String,
    pub flags: Vec<FlagInfo>,
}

impl CommandInfo {
    pub fn new(name: impl Into<String>, about: impl Into<String>) -> Self {
        CommandInfo {
            name: name.into(),
            about: about.into(),
            flags: vec![],
        }
    }

    /// Adds a flag: `--long`, what it does, and `Some("VALUE")` when it takes one.
    pub fn flag(mut self, long: impl Into<String>, help: impl Into<String>, value: Option<&str>) -> Self {
        self.flags.push(FlagInfo {
            long: long.into(),
            help: help.into(),
            value: value.map(Into::into),
        });
        self
    }

    /// The subcommands of a clap parser and their long flags; hidden ones and
    /// global flags are left out.
    #[cfg(feature = "clap")]
    pub fn from_clap(cmd: &clap::Command) -> Vec<CommandInfo> {
        cmd.get_subcommands()
            .filter(|sub| !sub.is_hide_set())
            .map(|sub| CommandInfo {
                name: sub.get_name().to_owned(),
                about: sub.get_about().map(|s| s.to_string()).unwrap_or_default(),
                flags: sub
                    .get_arguments()
                    .filter(|a| !a.is_global_set() && !a.is_hide_set())
                    .filter_map(|a| {
                        let long = a.get_long()?;
                        Some(FlagInfo {
                            long: format!("--{long}"),
                            help: a.get_help().map(|s| s.to_string()).unwrap_or_default(),
                            value: a.get_action().takes_values().then(|| {
                                a.get_value_names()
                                    .and_then(|v| v.first())
                                    .map_or_else(|| long.to_uppercase(), |v| v.to_string())
                            }),
                        })
                    })
                    .collect(),
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct FlagInfo {
    /// `--match`.
    pub long: String,
    pub help: String,
    /// `ID` when the flag takes a value.
    pub value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    /// What replaces the word being typed.
    pub value: String,
    /// What the menu shows.
    pub label: String,
    pub description: String,
}

impl Suggestion {
    pub fn new(value: impl Into<String>, label: impl Into<String>, description: impl Into<String>) -> Self {
        Suggestion {
            value: value.into(),
            label: label.into(),
            description: description.into(),
        }
    }
}

/// Values the menu offers beyond command names and flags.
pub trait Completer {
    /// Suggestions for `word`, the word being typed after `/cmd`:
    ///
    /// - `flag` is `Some("--map")` when the word is that flag's value;
    /// - otherwise the word is one of the command's own arguments, and `args`
    ///   holds the words typed before it. An empty `word` here is a bare
    ///   `/cmd `; what's returned is listed before the command's flags.
    fn complete(&self, cmd: &str, flag: Option<&str>, args: &[&str], word: &str) -> Vec<Suggestion> {
        let _ = (cmd, flag, args, word);
        vec![]
    }
}

/// Command names and flags only.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoCompleter;

impl Completer for NoCompleter {}

/// What the editor completes from.
#[derive(Clone, Copy)]
pub struct Context<'a> {
    pub commands: &'a [CommandInfo],
    pub completer: &'a dyn Completer,
}

#[derive(Debug, Default)]
pub struct Suggestions {
    /// Char index where the replaced word starts.
    pub start: usize,
    pub items: Vec<Suggestion>,
    /// The cursor is in the command name; Enter runs the highlighted command.
    pub command_word: bool,
}

/// Suggestions for the text before the cursor.
pub fn suggest(before: &str, ctx: &Context) -> Suggestions {
    let chars: Vec<char> = before.chars().collect();
    let start = chars.iter().rposition(|c| c.is_whitespace()).map_or(0, |i| i + 1);
    let word: String = chars[start..].iter().collect();
    // Only spaces before the word: it's the command name.
    if chars[..start].iter().all(|c| c.is_whitespace()) {
        let Some(prefix) = word.strip_prefix('/') else {
            return Suggestions::default();
        };
        let prefix = prefix.to_lowercase();
        // The command named exactly first, so Enter on a whole name runs it.
        let exact = ctx.commands.iter().filter(|c| c.name == prefix);
        let starts = exact.chain(
            ctx.commands
                .iter()
                .filter(|c| c.name != prefix && c.name.starts_with(&prefix)),
        );
        let contains = ctx
            .commands
            .iter()
            .filter(|c| !c.name.starts_with(&prefix) && c.name.contains(&prefix));
        let items = starts
            .chain(contains)
            .map(|c| Suggestion {
                value: format!("/{}", c.name),
                label: format!("/{}", c.name),
                description: c.about.clone(),
            })
            .collect();
        return Suggestions {
            start,
            items,
            command_word: true,
        };
    }

    let head: String = chars[..start].iter().collect();
    let tokens: Vec<&str> = head.split_whitespace().collect();
    let name = tokens[0].trim_start_matches('/').to_lowercase();
    let Some(cmd) = ctx.commands.iter().find(|c| c.name == name) else {
        return Suggestions::default();
    };
    let prev = tokens.last().copied().unwrap_or_default();
    let mut items = Vec::new();
    if let Some(flag) = cmd.flags.iter().find(|f| f.long == prev && f.value.is_some()) {
        items = ctx.completer.complete(&cmd.name, Some(&flag.long), &tokens[1..], &word);
    } else if !word.starts_with('-') {
        items = ctx.completer.complete(&cmd.name, None, &tokens[1..], &word);
    }
    if word.is_empty() || word.starts_with('-') {
        items.extend(
            cmd.flags
                .iter()
                .filter(|f| f.long.starts_with(&word) && !tokens.contains(&f.long.as_str()))
                .filter(|_| !cmd.flags.iter().any(|f| f.long == prev && f.value.is_some()))
                .map(|f| Suggestion {
                    value: f.long.clone(),
                    label: match &f.value {
                        Some(v) => format!("{} <{v}>", f.long),
                        None => f.long.clone(),
                    },
                    description: f.help.clone(),
                }),
        );
    }
    Suggestions {
        start,
        items,
        command_word: false,
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Submit(String),
    Exit,
    ClearScreen,
}

#[derive(Debug, Default)]
pub struct Editor {
    buf: Vec<char>,
    cursor: usize,
    selected: usize,
    menu_hidden: bool,
    history: Vec<String>,
    /// Index into `history` while browsing it.
    hist_pos: Option<usize>,
    /// The line being typed before browsing history.
    draft: Vec<char>,
    /// Ctrl+C on an empty line once; a second one exits.
    ctrl_c_armed: bool,
}

impl Editor {
    pub fn new(history: Vec<String>) -> Self {
        Editor {
            history,
            ..Editor::default()
        }
    }

    pub fn line(&self) -> String {
        self.buf.iter().collect()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn ctrl_c_armed(&self) -> bool {
        self.ctrl_c_armed
    }

    /// The menu as it is drawn now: nothing while hidden or browsing history.
    pub fn suggestions(&self, ctx: &Context) -> Suggestions {
        if self.menu_hidden || self.hist_pos.is_some() {
            return Suggestions::default();
        }
        let before: String = self.buf[..self.cursor].iter().collect();
        suggest(&before, ctx)
    }

    /// The highlighted menu row, clamped to the menu.
    pub fn selected(&self, n: usize) -> usize {
        self.selected.min(n.saturating_sub(1))
    }

    /// Muted text after the cursor: the rest of the highlighted command name.
    pub fn ghost(&self, ctx: &Context) -> Option<String> {
        if self.cursor != self.buf.len() {
            return None;
        }
        let s = self.suggestions(ctx);
        if !s.command_word || s.items.is_empty() {
            return None;
        }
        let typed: String = self.buf[s.start..].iter().collect::<String>().to_lowercase();
        let value = &s.items[self.selected(s.items.len())].value;
        value
            .strip_prefix(typed.as_str())
            .filter(|rest| !rest.is_empty())
            .map(str::to_owned)
    }

    /// Replace the line, cursor at the end.
    pub fn set_line(&mut self, s: &str) {
        self.buf.clear();
        self.cursor = 0;
        self.insert_str(s);
    }

    /// Insert `s` at the cursor. The line is one line: line breaks and tabs
    /// in pasted text become spaces, and other control characters go.
    pub fn insert_str(&mut self, s: &str) {
        let s = s.replace("\r\n", "\n");
        let chars = s
            .chars()
            .map(|c| if matches!(c, '\n' | '\r' | '\t') { ' ' } else { c })
            .filter(|c| !c.is_control());
        for c in chars {
            self.buf.insert(self.cursor, c);
            self.cursor += 1;
        }
        self.edited();
    }

    fn edited(&mut self) {
        self.menu_hidden = false;
        self.selected = 0;
        self.hist_pos = None;
    }

    fn set(&mut self, line: &[char]) {
        self.buf = line.to_vec();
        self.cursor = self.buf.len();
    }

    /// Replace the word being completed with the highlighted suggestion.
    fn accept(&mut self, s: &Suggestions) {
        let item = &s.items[self.selected(s.items.len())];
        // The whole word goes, the part after the cursor too, and the
        // spaces after it, which the one added below stands in for.
        let mut end = self.cursor;
        while end < self.buf.len() && !self.buf[end].is_whitespace() {
            end += 1;
        }
        while end < self.buf.len() && self.buf[end].is_whitespace() {
            end += 1;
        }
        let tail: Vec<char> = self.buf[end..].to_vec();
        self.buf.truncate(s.start);
        self.buf.extend(item.value.chars());
        self.buf.push(' ');
        self.cursor = self.buf.len();
        self.buf.extend(tail);
        self.edited();
    }

    fn submit(&mut self) -> Action {
        let line = self.line().trim().to_owned();
        if !line.is_empty() && self.history.last() != Some(&line) {
            self.history.push(line.clone());
        }
        self.buf.clear();
        self.cursor = 0;
        self.edited();
        Action::Submit(line)
    }

    fn history_step(&mut self, back: bool) {
        let n = self.history.len();
        let next = match (self.hist_pos, back) {
            (None, true) if n > 0 => {
                self.draft = self.buf.clone();
                Some(n - 1)
            }
            (Some(i), true) => Some(i.saturating_sub(1)),
            (Some(i), false) if i + 1 < n => Some(i + 1),
            (Some(_), false) => {
                let draft = std::mem::take(&mut self.draft);
                self.set(&draft);
                self.hist_pos = None;
                return;
            }
            _ => return,
        };
        if let Some(i) = next {
            let line: Vec<char> = self.history[i].chars().collect();
            self.set(&line);
            self.hist_pos = Some(i);
        }
    }

    pub fn handle(&mut self, key: KeyEvent, ctx: &Context) -> Action {
        // Windows reports AltGr (how many layouts type @ \ { } and €) as
        // Ctrl+Alt: a character typed with both is text.
        let altgr = key.modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::ALT);
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL) && !altgr;
        let s = self.suggestions(ctx);
        let menu = !s.items.is_empty();
        if !(ctrl && key.code == KeyCode::Char('c')) {
            self.ctrl_c_armed = false;
        }
        match key.code {
            KeyCode::Enter => {
                if menu && s.command_word {
                    self.accept(&s);
                }
                return self.submit();
            }
            KeyCode::Tab if menu => self.accept(&s),
            KeyCode::Up if menu => self.selected = (self.selected(s.items.len()) + s.items.len() - 1) % s.items.len(),
            KeyCode::Down if menu => self.selected = (self.selected(s.items.len()) + 1) % s.items.len(),
            KeyCode::Up => self.history_step(true),
            KeyCode::Down => self.history_step(false),
            KeyCode::Esc => self.menu_hidden = true,
            KeyCode::Right if self.cursor == self.buf.len() && self.ghost(ctx).is_some() => self.accept(&s),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(self.buf.len()),
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.buf.len(),
            KeyCode::Backspace if self.cursor > 0 => {
                self.cursor -= 1;
                self.buf.remove(self.cursor);
                self.edited();
            }
            KeyCode::Delete if self.cursor < self.buf.len() => {
                self.buf.remove(self.cursor);
                self.edited();
            }
            KeyCode::Char(c) if ctrl => match c {
                'c' if self.buf.is_empty() => {
                    if self.ctrl_c_armed {
                        return Action::Exit;
                    }
                    self.ctrl_c_armed = true;
                }
                'c' | 'u' => {
                    self.buf.drain(..self.cursor);
                    self.cursor = 0;
                    if c == 'c' {
                        self.buf.clear();
                    }
                    self.edited();
                }
                'd' if self.buf.is_empty() => return Action::Exit,
                'a' => self.cursor = 0,
                'e' => self.cursor = self.buf.len(),
                'l' => return Action::ClearScreen,
                'w' => {
                    let mut i = self.cursor;
                    while i > 0 && self.buf[i - 1].is_whitespace() {
                        i -= 1;
                    }
                    while i > 0 && !self.buf[i - 1].is_whitespace() {
                        i -= 1;
                    }
                    self.buf.drain(i..self.cursor);
                    self.cursor = i;
                    self.edited();
                }
                _ => {}
            },
            KeyCode::Char(c) if altgr || !key.modifiers.contains(KeyModifiers::ALT) => {
                self.buf.insert(self.cursor, c);
                self.cursor += 1;
                self.edited();
            }
            _ => {}
        }
        Action::None
    }
}

/// Split on whitespace; double quotes group. Backslashes stay literal for Windows paths.
pub fn split_words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let (mut quoted, mut any) = (false, false);
    for c in line.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any {
                    words.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            c => {
                cur.push(c);
                any = true;
            }
        }
    }
    if any {
        words.push(cur);
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Offers ids for `--item` and after a bare `/show`, and colours as `/paint`'s argument.
    struct Items;

    impl Completer for Items {
        fn complete(&self, cmd: &str, flag: Option<&str>, args: &[&str], word: &str) -> Vec<Suggestion> {
            let ids = |prefix: &str| {
                ["aaaa", "bbbb", "cccc"]
                    .iter()
                    .filter(|i| i.starts_with(word))
                    .map(|i| Suggestion::new(format!("{prefix}{i}"), *i, "an item"))
                    .collect()
            };
            match (cmd, flag) {
                (_, Some("--item")) => ids(""),
                ("show", None) if args.is_empty() && word.is_empty() => ids("--item "),
                ("paint", None) if args.is_empty() => ["red", "green"]
                    .iter()
                    .filter(|c| c.starts_with(word))
                    .map(|c| Suggestion::new(*c, *c, ""))
                    .collect(),
                _ => vec![],
            }
        }
    }

    fn commands() -> Vec<CommandInfo> {
        vec![
            CommandInfo::new("scan", "about scan")
                .flag("--keep-raw", "help", None)
                .flag("--retry-failed", "help", None),
            CommandInfo::new("show", "about show")
                .flag("--item", "help", Some("ID"))
                .flag("--me", "help", Some("ME"))
                .flag("--out", "help", Some("OUT")),
            CommandInfo::new("status", "about status"),
            CommandInfo::new("trends", "about trends").flag("--last", "help", Some("N")),
            CommandInfo::new("paint", "about paint"),
        ]
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn typed(ed: &mut Editor, ctx: &Context, s: &str) {
        for c in s.chars() {
            ed.handle(key(KeyCode::Char(c)), ctx);
        }
    }

    fn values(s: &Suggestions) -> Vec<&str> {
        s.items.iter().map(|i| i.value.as_str()).collect()
    }

    #[test]
    fn command_names() {
        let cmds = commands();
        let c = Context {
            commands: &cmds,
            completer: &NoCompleter,
        };
        assert!(suggest("", &c).items.is_empty());
        assert!(suggest("status", &c).items.is_empty(), "only after a slash");
        assert_eq!(values(&suggest("/", &c)).len(), 5);
        // Prefix matches first, then names containing the text.
        assert_eq!(values(&suggest("/s", &c)), ["/scan", "/show", "/status", "/trends"]);
        assert_eq!(values(&suggest("/tr", &c)), ["/trends"]);
        assert!(suggest("/st", &c).command_word);
    }

    #[test]
    fn arguments() {
        let cmds = commands();
        let c = Context {
            commands: &cmds,
            completer: &Items,
        };
        // A bare `/show ` offers what the completer has, then the flags.
        let s = suggest("/show ", &c);
        assert!(!s.command_word);
        assert_eq!(
            values(&s),
            ["--item aaaa", "--item bbbb", "--item cccc", "--item", "--me", "--out"]
        );
        assert_eq!(values(&suggest("/show --item b", &c)), ["bbbb"]);
        assert_eq!(values(&suggest("/show --m", &c)), ["--me"]);
        assert_eq!(values(&suggest("/paint g", &c)), ["green"]);
        assert_eq!(values(&suggest("/paint red ", &c)), Vec::<&str>::new());
        // Used flags drop out; values of other flags get nothing.
        assert_eq!(values(&suggest("/scan --keep-raw ", &c)), ["--retry-failed"]);
        assert!(suggest("/trends --last ", &c).items.is_empty());
        assert!(suggest("/show --out C:/x", &c).items.is_empty());
        assert!(suggest("/nope ", &c).items.is_empty());
    }

    #[test]
    fn enter_runs_the_highlighted_command() {
        let cmds = commands();
        let c = Context {
            commands: &cmds,
            completer: &Items,
        };
        let mut ed = Editor::new(vec![]);
        typed(&mut ed, &c, "/sta");
        assert_eq!(ed.ghost(&c).as_deref(), Some("tus"));
        assert_eq!(ed.handle(key(KeyCode::Enter), &c), Action::Submit("/status".into()));
        assert_eq!(ed.line(), "");

        // Down moves the highlight; Enter runs that one.
        typed(&mut ed, &c, "/s");
        ed.handle(key(KeyCode::Down), &c);
        assert_eq!(ed.handle(key(KeyCode::Enter), &c), Action::Submit("/show".into()));

        // Past the command name, Enter submits what was typed.
        typed(&mut ed, &c, "/trends --last 10");
        assert_eq!(
            ed.handle(key(KeyCode::Enter), &c),
            Action::Submit("/trends --last 10".into())
        );
    }

    #[test]
    fn tab_completes() {
        let cmds = commands();
        let c = Context {
            commands: &cmds,
            completer: &Items,
        };
        let mut ed = Editor::new(vec![]);
        typed(&mut ed, &c, "/sho");
        ed.handle(key(KeyCode::Tab), &c);
        assert_eq!(ed.line(), "/show ");
        ed.handle(key(KeyCode::Down), &c);
        ed.handle(key(KeyCode::Tab), &c);
        assert_eq!(ed.line(), "/show --item bbbb ");
        // Right arrow at the end takes the ghost text.
        let mut ed = Editor::new(vec![]);
        typed(&mut ed, &c, "/tre");
        ed.handle(key(KeyCode::Right), &c);
        assert_eq!(ed.line(), "/trends ");
    }

    #[test]
    fn esc_hides_the_menu_until_the_next_edit() {
        let cmds = commands();
        let c = Context {
            commands: &cmds,
            completer: &NoCompleter,
        };
        let mut ed = Editor::new(vec![]);
        typed(&mut ed, &c, "/st");
        ed.handle(key(KeyCode::Esc), &c);
        assert!(ed.suggestions(&c).items.is_empty());
        assert_eq!(ed.handle(key(KeyCode::Enter), &c), Action::Submit("/st".into()));
        typed(&mut ed, &c, "/s");
        ed.handle(key(KeyCode::Esc), &c);
        typed(&mut ed, &c, "c");
        assert_eq!(values(&ed.suggestions(&c)), ["/scan"]);
    }

    #[test]
    fn history_and_keys() {
        let cmds = commands();
        let c = Context {
            commands: &cmds,
            completer: &NoCompleter,
        };
        let mut ed = Editor::new(vec!["/scan".into(), "/status".into()]);
        typed(&mut ed, &c, "dra");
        ed.handle(key(KeyCode::Up), &c);
        assert_eq!(ed.line(), "/status");
        assert!(ed.suggestions(&c).items.is_empty(), "no menu while browsing history");
        ed.handle(key(KeyCode::Up), &c);
        assert_eq!(ed.line(), "/scan");
        ed.handle(key(KeyCode::Down), &c);
        ed.handle(key(KeyCode::Down), &c);
        assert_eq!(ed.line(), "dra");

        ed.handle(ctrl('w'), &c);
        assert_eq!(ed.line(), "");
        typed(&mut ed, &c, "ab");
        ed.handle(key(KeyCode::Left), &c);
        ed.handle(key(KeyCode::Backspace), &c);
        assert_eq!((ed.line().as_str(), ed.cursor()), ("b", 0));

        // Ctrl+C clears a line; twice on an empty line exits.
        assert_eq!(ed.handle(ctrl('c'), &c), Action::None);
        assert_eq!(ed.line(), "");
        assert_eq!(ed.handle(ctrl('c'), &c), Action::None);
        assert!(ed.ctrl_c_armed());
        assert_eq!(ed.handle(ctrl('c'), &c), Action::Exit);
        assert_eq!(ed.handle(ctrl('d'), &c), Action::Exit);
    }

    #[test]
    fn words() {
        assert_eq!(split_words("/review  --match ab"), ["/review", "--match", "ab"]);
        assert_eq!(
            split_words(r#"/scan --demos-dir "C:\My Demos""#),
            ["/scan", "--demos-dir", r"C:\My Demos"]
        );
        assert!(split_words("   ").is_empty());
    }

    #[cfg(feature = "clap")]
    #[test]
    fn commands_from_clap() {
        use clap::{CommandFactory, Parser, Subcommand};

        #[derive(Parser)]
        struct Cli {
            #[arg(long, global = true)]
            data_dir: Option<String>,
            #[command(subcommand)]
            command: Option<Cmd>,
        }
        #[derive(Subcommand)]
        enum Cmd {
            /// Show one
            Show {
                /// Which one
                #[arg(long = "item", value_name = "ITEM")]
                item: Option<String>,
                /// Every one
                #[arg(long)]
                all: bool,
            },
            #[command(hide = true)]
            Secret,
        }
        let cmds = CommandInfo::from_clap(&Cli::command());
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].about, "Show one");
        let flags: Vec<(&str, Option<&str>)> = cmds[0]
            .flags
            .iter()
            .map(|f| (f.long.as_str(), f.value.as_deref()))
            .collect();
        assert_eq!(flags, [("--item", Some("ITEM")), ("--all", None)]);
    }

    #[test]
    fn a_leading_space_is_no_command_and_no_crash() {
        let cmds = commands();
        let c = Context {
            commands: &cmds,
            completer: &NoCompleter,
        };
        assert!(suggest(" ", &c).items.is_empty());
        assert!(suggest("\u{3000}", &c).items.is_empty());
        assert_eq!(values(&suggest("  /st", &c)), ["/status"]);
    }

    #[test]
    fn an_exact_name_comes_first() {
        let cmds = vec![CommandInfo::new("scan-all", ""), CommandInfo::new("scan", "")];
        let c = Context {
            commands: &cmds,
            completer: &NoCompleter,
        };
        assert_eq!(values(&suggest("/scan", &c)), ["/scan", "/scan-all"]);
        let mut ed = Editor::default();
        typed(&mut ed, &c, "/scan");
        assert_eq!(ed.handle(key(KeyCode::Enter), &c), Action::Submit("/scan".into()));
    }

    #[test]
    fn accepting_mid_word_replaces_the_whole_word() {
        let cmds = commands();
        let c = Context {
            commands: &cmds,
            completer: &NoCompleter,
        };
        let mut ed = Editor::default();
        ed.set_line("/status foo");
        ed.cursor = 3;
        assert_eq!(ed.handle(key(KeyCode::Enter), &c), Action::Submit("/status foo".into()));
        ed.set_line("/sta foo");
        ed.cursor = 2;
        ed.handle(key(KeyCode::Tab), &c);
        assert_eq!(ed.line(), "/scan foo");
    }

    #[test]
    fn altgr_types_text() {
        let cmds = commands();
        let c = Context {
            commands: &cmds,
            completer: &NoCompleter,
        };
        let mut ed = Editor::default();
        let altgr = |ch| KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL | KeyModifiers::ALT);
        ed.handle(key(KeyCode::Char('a')), &c);
        ed.handle(altgr('@'), &c);
        ed.handle(altgr('\\'), &c);
        assert_eq!(ed.line(), "a@\\");
    }

    #[test]
    fn pasted_lines_become_one_line() {
        let mut ed = Editor::default();
        ed.insert_str("/greet\r\nrm -rf x\tnow\n");
        assert_eq!(ed.line(), "/greet rm -rf x now ");
    }

    #[test]
    fn the_ghost_ignores_case() {
        let cmds = commands();
        let c = Context {
            commands: &cmds,
            completer: &NoCompleter,
        };
        let mut ed = Editor::default();
        typed(&mut ed, &c, "/STA");
        assert_eq!(ed.ghost(&c).as_deref(), Some("tus"));
    }
}
