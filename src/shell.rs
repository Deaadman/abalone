//! The shell: output fills the top of the terminal and the input box stays
//! pinned to the bottom, with the command menu opening above it. Commands
//! run on a worker thread and print into the transcript through
//! [`crate::out`], so the screen keeps drawing while they work.
//!
//! An app implements [`App`]: its commands, what a submitted line does, and
//! optionally its header, notices, panels and periodic work. [`Shell`] does
//! the rest, and its builder methods swap any part: the logo, spinner,
//! theme, skin, help and settings.
//!
//! It draws one of two ways ([`Tui`], switched with `/tui`): over the whole
//! terminal on the alternate screen, or in the terminal's own scrollback,
//! where finished output stays and only the input box and what's around it
//! are redrawn.

use std::collections::VecDeque;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossterm::cursor::{Hide, MoveTo, MoveToColumn, MoveUp, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use crossterm::queue;
use crossterm::style::Print;
use crossterm::terminal::{self, Clear, ClearType};

use crate::editor::{Action, CommandInfo, Completer, Context, Editor, NoCompleter, Suggestion};
use crate::help::{Help, HelpAction, HelpPanel};
use crate::logo::{DefaultLogo, Logo};
use crate::notes::{self, NotesAction, NotesPanel, Release};
use crate::out::{self, Kind, Line};
use crate::settings::{SettingsAction, SettingsModel, SettingsPanel};
use crate::skin::{self, DefaultSkin, Skin};
use crate::spinner::{Classic, Spinner};
use crate::status::{Notice, UpdateNote, job_left, scrolled_left};
use crate::style::{Row, Style};
use crate::term::{Screen, print_row};
use crate::theme::{Theme, set_theme};

/// What a full-screen (or bottom) panel the app opens does.
pub trait Panel {
    /// What the panel asks the app to do, handed to [`App::panel_event`].
    type Event;

    fn key(&mut self, key: KeyEvent) -> PanelEvent<Self::Event>;

    /// The panel for a `w`×`h` screen: `h` rows, or fewer for a bottom panel.
    fn rows(&mut self, w: usize, h: usize) -> Vec<Row>;

    /// Covers the whole screen (true), or replaces the input box like help
    /// and settings do, with the output above it (false).
    fn full_screen(&self) -> bool {
        true
    }

    fn paste(&mut self, text: &str) {
        let _ = text;
    }

    /// True while it's waiting on something, so the shell keeps redrawing.
    fn busy(&mut self) -> bool {
        false
    }
}

pub enum PanelEvent<E> {
    None,
    Close,
    Event(E),
}

/// For an app with no panels of its own: `type Panel = NoPanel;`.
pub enum NoPanel {}

impl Panel for NoPanel {
    type Event = std::convert::Infallible;

    fn key(&mut self, _key: KeyEvent) -> PanelEvent<Self::Event> {
        match *self {}
    }

    fn rows(&mut self, _w: usize, _h: usize) -> Vec<Row> {
        match *self {}
    }
}

/// What the app tells the shell.
pub trait App: Sized {
    /// What a command's worker thread hands back, to [`App::done`].
    type Msg: Send + 'static;
    /// The app's own panels.
    type Panel: Panel;

    /// The commands for the menu and `/help` (the shell adds its built-ins after them).
    fn commands(&self) -> Vec<CommandInfo>;

    /// A line the user sent that isn't a built-in. Long work goes on a worker
    /// thread with [`Ctx::start`]; output goes through [`crate::out`].
    fn run(&mut self, line: &str, ctx: &mut Ctx<Self>);

    /// When the shell opens, after the screen is up.
    fn launch(&mut self, ctx: &mut Ctx<Self>) {
        let _ = ctx;
    }

    /// A worker thread finished with `msg`.
    fn done(&mut self, msg: Self::Msg, ctx: &mut Ctx<Self>) {
        let _ = (msg, ctx);
    }

    /// The open panel asked for something.
    fn panel_event(&mut self, event: <Self::Panel as Panel>::Event, ctx: &mut Ctx<Self>) {
        let _ = (event, ctx);
    }

    /// Every tick (about 80 ms): background checks, timers.
    fn tick(&mut self, ctx: &mut Ctx<Self>) {
        let _ = ctx;
    }

    /// After a command or a settings change: re-read what may have changed.
    /// The shell reads the header, notices, footer and placeholder again after.
    fn refresh(&mut self) {}

    /// Beside the logo. The shell shows its title when this is empty.
    fn header(&self) -> Vec<Row> {
        vec![]
    }

    /// Messages for the right of the status bar, most important first.
    fn notices(&self) -> Vec<Notice> {
        vec![]
    }

    /// Under the input box.
    fn footer(&self) -> String {
        "/help for commands and keys".into()
    }

    /// In the empty input box.
    fn placeholder(&self) -> String {
        "Type / for commands".into()
    }

    /// Values for the command menu.
    fn completer(&self) -> &dyn Completer {
        &NoCompleter
    }

    /// Said when a worker thread panics.
    fn crashed(&self) -> String {
        "internal error: the command crashed".into()
    }
}

type Work<M> = Box<dyn FnOnce() -> M + Send + 'static>;

type PanicHook = Box<dyn Fn(&std::panic::PanicHookInfo<'_>) + Send + Sync + 'static>;

enum Do<M> {
    Start(String, Work<M>),
    Clear,
    Exit,
    Help,
    Settings,
    SetLine(String),
    Refresh,
}

/// What app callbacks can ask of the shell.
pub struct Ctx<'a, A: App> {
    panel: &'a mut Option<A::Panel>,
    job: Option<(String, Instant)>,
    todo: Vec<Do<A::Msg>>,
    redraw: bool,
}

impl<A: App> Ctx<'_, A> {
    /// Run `work` on a worker thread, `label` beside the spinner meanwhile.
    /// One runs at a time; later ones wait their turn.
    pub fn start(&mut self, label: impl Into<String>, work: impl FnOnce() -> A::Msg + Send + 'static) {
        self.todo.push(Do::Start(label.into(), Box::new(work)));
    }

    /// The running command's label and when it started.
    pub fn job(&self) -> Option<(&str, Instant)> {
        self.job.as_ref().map(|(l, t)| (l.as_str(), *t))
    }

    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    pub fn open(&mut self, panel: A::Panel) {
        *self.panel = Some(panel);
        self.redraw = true;
    }

    pub fn close_panel(&mut self) {
        *self.panel = None;
        self.redraw = true;
    }

    pub fn panel(&mut self) -> Option<&mut A::Panel> {
        self.panel.as_mut()
    }

    pub fn open_help(&mut self) {
        self.todo.push(Do::Help);
    }

    pub fn open_settings(&mut self) {
        self.todo.push(Do::Settings);
    }

    /// Empty the transcript down to the header.
    pub fn clear(&mut self) {
        self.todo.push(Do::Clear);
    }

    pub fn exit(&mut self) {
        self.todo.push(Do::Exit);
    }

    /// Put `line` in the input box.
    pub fn set_line(&mut self, line: impl Into<String>) {
        self.todo.push(Do::SetLine(line.into()));
    }

    /// Re-read the app's header, notices and footer.
    pub fn refresh(&mut self) {
        self.todo.push(Do::Refresh);
    }

    /// Draw again this tick.
    pub fn redraw(&mut self) {
        self.redraw = true;
    }
}

/// One entry in the transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// The logo and the app's header lines.
    Header,
    Line(Kind, String),
    /// Pieces in their own styles.
    Row(Line),
    /// A line the user sent.
    Echo(String),
    /// Rows drawn as they are.
    Rows(Vec<Row>),
}

struct Job<M> {
    label: String,
    started: Instant,
    handle: JoinHandle<M>,
}

/// Checks for updates on a thread now and then; see [`Shell::update_check`].
type Check = Arc<dyn Fn(Option<UpdateNote>) -> Option<UpdateNote> + Send + Sync>;

struct Updates {
    check: Check,
    every: Duration,
    next: Instant,
    rx: Option<Receiver<Option<UpdateNote>>>,
    note: Option<UpdateNote>,
    command: String,
}

/// How the shell draws; `/tui fullscreen` and `/tui default` switch it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tui {
    /// The whole terminal, on the alternate screen; the terminal's own
    /// scrollback comes back untouched at the end.
    #[default]
    Fullscreen,
    /// In the terminal's own scrollback: finished output stays there, and
    /// only the input box and what's around it are redrawn. Panels that
    /// cover the whole screen still open on the alternate screen.
    Inline,
}

impl Tui {
    /// What `/tui` calls it.
    pub fn name(self) -> &'static str {
        match self {
            Tui::Fullscreen => "fullscreen",
            Tui::Inline => "default",
        }
    }

    pub fn parse(word: &str) -> Option<Tui> {
        match word.trim().to_lowercase().as_str() {
            "fullscreen" | "full" => Some(Tui::Fullscreen),
            "default" | "inline" => Some(Tui::Inline),
            _ => None,
        }
    }
}

/// Where the inline view left off.
#[derive(Debug, Default)]
struct InlineState {
    /// Transcript items printed into the scrollback for good.
    committed: usize,
    /// Rows from the top of the redrawn part down to the cursor.
    cursor_row: usize,
    /// Clear the screen before the next draw.
    wipe: bool,
}

/// The app's completer, with the values of `/tui`.
struct Completions<'a> {
    app: &'a dyn Completer,
    tui: bool,
}

impl Completer for Completions<'_> {
    fn complete(&self, cmd: &str, flag: Option<&str>, args: &[&str], word: &str) -> Vec<Suggestion> {
        if self.tui && cmd == "tui" && flag.is_none() {
            return [Tui::Fullscreen, Tui::Inline]
                .into_iter()
                .filter(|t| t.name().starts_with(&word.to_lowercase()))
                .map(|t| {
                    let about = match t {
                        Tui::Fullscreen => "Take over the whole terminal",
                        Tui::Inline => "Run in the terminal's own scrollback",
                    };
                    Suggestion::new(t.name(), t.name(), about)
                })
                .collect();
        }
        self.app.complete(cmd, flag, args, word)
    }
}

/// The rows of one frame and where the cursor goes.
pub struct Frame {
    pub rows: Vec<Row>,
    pub cursor: Option<(usize, usize)>,
}

/// The shell around an [`App`]. Build it with the methods below, then [`Shell::run`].
pub struct Shell<A: App> {
    app: A,
    title: String,
    prompt: String,
    logo: Box<dyn Logo>,
    spinner: Box<dyn Spinner>,
    skin: Box<dyn Skin>,
    help: Option<Help>,
    settings: Option<Box<dyn FnMut() -> Box<dyn SettingsModel>>>,
    history_path: Option<PathBuf>,
    history_lines: usize,
    tick: Duration,
    menu_rows: usize,
    releases: Option<Vec<Release>>,
    tui: Tui,
    tui_path: Option<PathBuf>,

    editor: Editor,
    commands: Vec<CommandInfo>,
    /// Built-ins the shell answers itself, by name.
    builtins: Vec<&'static str>,
    transcript: Vec<Item>,
    /// Rows scrolled up from the newest output.
    scroll: usize,
    rx: Option<Receiver<Line>>,
    job: Option<Job<A::Msg>>,
    queue: VecDeque<(String, Work<A::Msg>)>,
    help_panel: Option<HelpPanel>,
    settings_panel: Option<SettingsPanel>,
    notes_panel: Option<NotesPanel>,
    panel: Option<A::Panel>,
    inline: InlineState,
    launched: Instant,
    logo_frame: usize,
    header: Vec<Row>,
    notices: Vec<Notice>,
    footer: String,
    placeholder: String,
    updates: Option<Updates>,
    /// A job drawn as running, for screenshots.
    pretend: Option<(String, Instant)>,
}

impl<A: App> Shell<A> {
    pub fn new(app: A) -> Self {
        let mut shell = Shell {
            app,
            title: "Shell".into(),
            prompt: "❯ ".into(),
            logo: Box::new(DefaultLogo),
            spinner: Box::new(Classic::default()),
            skin: Box::new(DefaultSkin),
            help: Some(Help::default()),
            settings: None,
            history_path: None,
            history_lines: 500,
            tick: Duration::from_millis(80),
            menu_rows: 8,
            releases: None,
            tui: Tui::default(),
            tui_path: None,
            editor: Editor::default(),
            commands: vec![],
            builtins: vec![],
            transcript: vec![Item::Header],
            scroll: 0,
            rx: None,
            job: None,
            queue: VecDeque::new(),
            help_panel: None,
            settings_panel: None,
            notes_panel: None,
            panel: None,
            inline: InlineState::default(),
            launched: Instant::now(),
            logo_frame: 0,
            header: vec![],
            notices: vec![],
            footer: String::new(),
            placeholder: String::new(),
            updates: None,
            pretend: None,
        };
        shell.refresh();
        shell
    }

    /// The window title and the name in the default header.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self.refresh();
        self
    }

    pub fn prompt(mut self, prompt: impl Into<String>) -> Self {
        self.prompt = prompt.into();
        self
    }

    pub fn logo(mut self, logo: impl Logo + 'static) -> Self {
        self.logo = Box::new(logo);
        self
    }

    pub fn spinner(mut self, spinner: impl Spinner + 'static) -> Self {
        self.spinner = Box::new(spinner);
        self
    }

    /// Make `theme` the active theme.
    pub fn theme(mut self, theme: Theme) -> Self {
        set_theme(theme);
        self.refresh();
        self
    }

    pub fn skin(mut self, skin: impl Skin + 'static) -> Self {
        self.skin = Box::new(skin);
        self
    }

    /// What `/help` shows.
    pub fn help(mut self, help: Help) -> Self {
        self.help = Some(help);
        self.refresh();
        self
    }

    /// No `/help` built-in (the app may handle `help` itself).
    pub fn without_help(mut self) -> Self {
        self.help = None;
        self.refresh();
        self
    }

    /// `/settings` opens a panel on a fresh model from `make` each time.
    /// When the app has a `settings` command of its own, that runs instead
    /// and opens the panel with [`Ctx::open_settings`].
    pub fn settings<M: SettingsModel + 'static>(mut self, mut make: impl FnMut() -> M + 'static) -> Self {
        self.settings = Some(Box::new(move || Box::new(make())));
        self.refresh();
        self
    }

    /// Keep the input history in `path` between sessions.
    pub fn history(mut self, path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        self.editor = Editor::new(load_history(&path));
        self.history_path = Some(path);
        self
    }

    /// `/release-notes` lists the versions in `changelog`, a Keep a
    /// Changelog `CHANGELOG.md` (`include_str!` it), and prints the one picked.
    pub fn release_notes(mut self, changelog: &str) -> Self {
        self.releases = Some(notes::parse(changelog));
        self.refresh();
        self
    }

    /// Draw this way until `/tui` says otherwise ([`Tui::Fullscreen`]).
    pub fn tui(mut self, tui: Tui) -> Self {
        self.tui = tui;
        self
    }

    /// Remember `/tui`'s choice in `path` between sessions.
    pub fn tui_file(mut self, path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        if let Some(t) = std::fs::read_to_string(&path).ok().and_then(|s| Tui::parse(&s)) {
            self.tui = t;
        }
        self.tui_path = Some(path);
        self
    }

    /// How it draws now.
    pub fn tui_mode(&self) -> Tui {
        self.tui
    }

    /// Lines of history kept (500).
    pub fn history_lines(mut self, n: usize) -> Self {
        self.history_lines = n.max(1);
        self
    }

    /// How often the screen can redraw and [`App::tick`] runs (80 ms).
    pub fn tick(mut self, tick: Duration) -> Self {
        self.tick = tick.max(Duration::from_millis(10));
        self
    }

    /// Suggestions the command menu shows at once (8).
    pub fn menu_rows(mut self, n: usize) -> Self {
        self.menu_rows = n.max(1);
        self
    }

    /// Run `check` on a thread at launch and every `every` after. It gets
    /// the note found so far and returns the one to show first on the
    /// status bar, if any. An `Installed` note stays until the restart.
    /// `command` is the app's update command, named in the notes.
    pub fn update_check(
        mut self,
        every: Duration,
        command: impl Into<String>,
        check: impl Fn(Option<UpdateNote>) -> Option<UpdateNote> + Send + Sync + 'static,
    ) -> Self {
        self.updates = Some(Updates {
            check: Arc::new(check),
            every,
            next: Instant::now(),
            rx: None,
            note: None,
            command: command.into(),
        });
        self
    }

    pub fn app(&self) -> &A {
        &self.app
    }

    pub fn app_mut(&mut self) -> &mut A {
        &mut self.app
    }

    pub fn transcript(&self) -> &[Item] {
        &self.transcript
    }

    pub fn transcript_mut(&mut self) -> &mut Vec<Item> {
        &mut self.transcript
    }

    pub fn editor_mut(&mut self) -> &mut Editor {
        &mut self.editor
    }

    pub fn open(&mut self, panel: A::Panel) {
        self.panel = Some(panel);
    }

    pub fn open_help(&mut self) {
        if self.help.is_some() {
            self.help_panel = Some(HelpPanel::default());
        }
    }

    /// `/release-notes`' picker, when there are release notes.
    pub fn open_release_notes(&mut self) {
        if self.releases.is_some() {
            self.notes_panel = Some(NotesPanel::new());
        }
    }

    pub fn open_settings(&mut self) -> Option<&mut SettingsPanel> {
        let make = self.settings.as_mut()?;
        self.settings_panel = Some(SettingsPanel::new(make()));
        self.settings_panel.as_mut()
    }

    /// The commands the menu offers, built-ins included.
    pub fn commands(&self) -> &[CommandInfo] {
        &self.commands
    }

    pub fn settings_panel(&mut self) -> Option<&mut SettingsPanel> {
        self.settings_panel.as_mut()
    }

    pub fn panel(&mut self) -> Option<&mut A::Panel> {
        self.panel.as_mut()
    }

    /// Close help, settings and the app's panel; empty the input line; scroll to the end.
    pub fn reset_view(&mut self) {
        self.help_panel = None;
        self.settings_panel = None;
        self.notes_panel = None;
        self.panel = None;
        self.editor = Editor::default();
        self.scroll = 0;
    }

    // What the screen shows, set directly: for screenshots and tests. The
    // next refresh reads them from the app again.

    /// Show the logo at `frame` (the launch animation advances it by itself).
    pub fn set_logo_frame(&mut self, frame: usize) {
        self.logo_frame = frame;
    }

    pub fn set_header(&mut self, lines: Vec<Row>) {
        self.header = lines;
    }

    pub fn set_notices(&mut self, notices: Vec<Notice>) {
        self.notices = notices;
        if let Some(u) = &mut self.updates {
            u.note = None;
        }
    }

    pub fn set_footer(&mut self, footer: impl Into<String>) {
        self.footer = footer.into();
    }

    pub fn set_placeholder(&mut self, placeholder: impl Into<String>) {
        self.placeholder = placeholder.into();
    }

    /// Draw the status bar as though `label` had been running since `started`.
    pub fn pretend_job(&mut self, job: Option<(String, Instant)>) {
        self.pretend = job;
    }

    /// Re-read the app's commands, header, notices, footer and placeholder.
    pub fn refresh(&mut self) {
        self.app.refresh();
        let mut commands = self.app.commands();
        let mut builtins = vec![];
        let mut add = |name: &'static str, about: String, commands: &mut Vec<CommandInfo>| {
            if !commands.iter().any(|c| c.name == name) {
                commands.push(CommandInfo::new(name, about));
                builtins.push(name);
            }
        };
        if self.help.is_some() {
            add("help", "Show the commands and keys".into(), &mut commands);
        }
        if self.settings.is_some() {
            add("settings", "Change the settings".into(), &mut commands);
        }
        if self.releases.is_some() {
            add("release-notes", "Show what's new in each version".into(), &mut commands);
        }
        add(
            "tui",
            "Full screen, or in the terminal's own scrollback: /tui fullscreen or /tui default".into(),
            &mut commands,
        );
        add("clear", "Clear the screen".into(), &mut commands);
        add("exit", format!("Quit {}", self.title), &mut commands);
        self.commands = commands;
        self.builtins = builtins;
        self.header = self.app.header();
        if self.header.is_empty() {
            self.header = vec![vec![(Style::default().bold(), self.title.clone())]];
        }
        self.notices = self.app.notices();
        self.footer = self.app.footer();
        self.placeholder = self.app.placeholder();
    }

    /// Open the screen and run until the user quits.
    pub fn run(mut self) -> anyhow::Result<()> {
        let (tx, rx) = mpsc::channel();
        out::capture(Some(tx));
        self.rx = Some(rx);
        // A panic's message is printed while the screen is still up, where
        // it would vanish with the alternate screen: keep it, and say it
        // again once the terminal is back.
        let said: Arc<Mutex<Option<String>>> = Arc::default();
        let before: Arc<PanicHook> = Arc::new(std::panic::take_hook());
        let runner = std::thread::current().id();
        {
            let (said, before) = (said.clone(), before.clone());
            std::panic::set_hook(Box::new(move |info| {
                if std::thread::current().id() == runner
                    && let Ok(mut s) = said.lock()
                {
                    *s = Some(info.to_string());
                }
                before(info);
            }));
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            Screen::on(&self.title, self.tui == Tui::Fullscreen)
                .map_err(anyhow::Error::from)
                .and_then(|mut screen| {
                    let result = self.event_loop(&mut screen);
                    // Inline, the output stays and the input box goes.
                    if !screen.alt() {
                        let mut out = io::stdout();
                        self.erase_live(&mut out)?;
                        out.flush()?;
                    }
                    result
                })
        }));
        std::panic::set_hook(Box::new(move |info| before(info)));
        out::capture(None);
        match result {
            Ok(result) => result,
            Err(panic) => {
                if let Some(s) = said.lock().ok().and_then(|mut s| s.take()) {
                    eprintln!("{s}");
                }
                std::panic::resume_unwind(panic)
            }
        }
    }

    /// Run an app callback with a `Ctx`, then do what it asked. False to quit.
    fn with_ctx(&mut self, f: impl FnOnce(&mut A, &mut Ctx<A>)) -> (bool, bool) {
        let mut ctx = Ctx {
            panel: &mut self.panel,
            job: self.job.as_ref().map(|j| (j.label.clone(), j.started)),
            todo: vec![],
            redraw: false,
        };
        f(&mut self.app, &mut ctx);
        let (todo, redraw) = (ctx.todo, ctx.redraw);
        let mut go_on = true;
        let mut redraw = redraw || !todo.is_empty();
        for d in todo {
            match d {
                Do::Start(label, work) => self.queue.push_back((label, work)),
                Do::Clear => self.clear(),
                Do::Exit => go_on = false,
                Do::Help => self.open_help(),
                Do::Settings => {
                    self.open_settings();
                }
                Do::SetLine(l) => self.editor.set_line(&l),
                Do::Refresh => self.refresh(),
            }
        }
        if self.job.is_none()
            && let Some((label, work)) = self.queue.pop_front()
        {
            self.job = Some(Job {
                label,
                started: Instant::now(),
                handle: std::thread::spawn(work),
            });
            redraw = true;
        }
        (go_on, redraw)
    }

    fn clear(&mut self) {
        self.transcript = vec![Item::Header];
        self.scroll = 0;
        self.inline.committed = 0;
        self.inline.wipe = true;
    }

    fn drain(&mut self) -> bool {
        let Some(rx) = &self.rx else { return false };
        let mut any = false;
        while let Ok(mut line) = rx.try_recv() {
            self.transcript.push(match line.len() {
                1 => {
                    let (kind, text) = line.remove(0);
                    Item::Line(kind, text)
                }
                _ => Item::Row(line),
            });
            self.scroll = 0;
            any = true;
        }
        any
    }

    fn poll_updates(&mut self) -> bool {
        let Some(u) = &mut self.updates else { return false };
        let mut changed = false;
        if let Some(rx) = &u.rx {
            match rx.try_recv() {
                Ok(note) => {
                    if u.note != Some(UpdateNote::Installed)
                        && let Some(n) = note
                    {
                        u.note = Some(n);
                        changed = true;
                    }
                    u.rx = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => u.rx = None,
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if u.rx.is_none() && Instant::now() >= u.next {
            let (tx, rx) = mpsc::channel();
            let (check, note) = (u.check.clone(), u.note);
            std::thread::spawn(move || {
                let _ = tx.send(check(note));
            });
            u.rx = Some(rx);
            u.next = Instant::now() + u.every;
        }
        changed
    }

    fn event_loop(&mut self, screen: &mut Screen) -> anyhow::Result<()> {
        let mut out = io::stdout();
        let mut drawn_as = self.tui;
        let (go_on, _) = self.with_ctx(|app, ctx| app.launch(ctx));
        if !go_on {
            return Ok(());
        }
        let mut dirty = true;
        loop {
            dirty |= self.drain();
            dirty |= self.poll_updates();
            if self.job.as_ref().is_some_and(|j| j.handle.is_finished()) {
                let job = self.job.take().expect("checked");
                let result = job.handle.join();
                self.drain();
                let (go_on, _) = match result {
                    Ok(msg) => self.with_ctx(|app, ctx| app.done(msg, ctx)),
                    Err(_) => {
                        let said = self.app.crashed();
                        self.transcript.push(Item::Line(Kind::Error, said));
                        self.with_ctx(|_, _| {})
                    }
                };
                if !go_on {
                    return Ok(());
                }
                self.refresh();
                dirty = true;
            }
            let (go_on, redraw) = self.with_ctx(|app, ctx| app.tick(ctx));
            if !go_on {
                return Ok(());
            }
            dirty |= redraw;
            if self.settings_panel.as_mut().is_some_and(SettingsPanel::busy) {
                dirty = true;
            }
            if self.panel.as_mut().is_some_and(Panel::busy) {
                dirty = true;
            }
            let frame = ((self.launched.elapsed().as_millis() / self.tick.as_millis()) as usize)
                .min(self.logo.frames().saturating_sub(1));
            if frame != self.logo_frame {
                self.logo_frame = frame;
                dirty = true;
            }
            if self.job.is_some() {
                dirty = true;
            }
            if self.tui != drawn_as {
                // From inline, take the input box away before leaving; into
                // inline, start over on the terminal's own screen.
                if drawn_as == Tui::Inline && !screen.alt() {
                    self.erase_live(&mut out)?;
                    out.flush()?;
                }
                self.inline = InlineState::default();
                drawn_as = self.tui;
                dirty = true;
            }
            if dirty {
                let full = self.tui == Tui::Fullscreen || self.panel.as_ref().is_some_and(|p| p.full_screen());
                screen.set_alt(full)?;
                if full {
                    self.draw(&mut out)?;
                } else {
                    self.draw_inline(&mut out)?;
                }
                dirty = false;
            }
            if !event::poll(self.tick)? {
                continue;
            }
            dirty = true;
            match event::read()? {
                Event::Key(k) if k.kind != KeyEventKind::Release => {
                    if !self.key(k) {
                        return Ok(());
                    }
                }
                Event::Paste(s) => {
                    if let Some(p) = &mut self.settings_panel {
                        p.paste(&s);
                    } else if let Some(p) = &mut self.panel {
                        p.paste(&s);
                    } else {
                        self.editor.insert_str(&s);
                    }
                }
                _ => {}
            }
        }
    }

    fn is_builtin(&self, name: &str, builtin: &str) -> bool {
        name == builtin && self.builtins.contains(&builtin)
    }

    /// Handle one key, as if typed. False to quit.
    pub fn key(&mut self, key: KeyEvent) -> bool {
        if let Some(panel) = &mut self.panel {
            match panel.key(key) {
                PanelEvent::None => {}
                PanelEvent::Close => self.panel = None,
                PanelEvent::Event(e) => return self.with_ctx(|app, ctx| app.panel_event(e, ctx)).0,
            }
            return true;
        }
        if let (Some(panel), Some(releases)) = (&mut self.notes_panel, &self.releases) {
            match panel.key(key, releases) {
                NotesAction::None => {}
                NotesAction::Close => self.notes_panel = None,
                NotesAction::Show(picked) => {
                    self.notes_panel = None;
                    for (n, i) in picked.into_iter().enumerate() {
                        if n > 0 {
                            self.transcript.push(Item::Line(Kind::Out, String::new()));
                        }
                        let lines = notes::lines(&releases[i]);
                        self.transcript.extend(lines.into_iter().map(Item::Row));
                    }
                    self.scroll = 0;
                }
            }
            return true;
        }
        if let Some(panel) = &mut self.settings_panel {
            match panel.key(key) {
                SettingsAction::None => {}
                SettingsAction::Close => {
                    self.settings_panel = None;
                    self.refresh();
                }
                SettingsAction::Changed => self.refresh(),
            }
            return true;
        }
        if let (Some(panel), Some(help)) = (&mut self.help_panel, &self.help) {
            match panel.key(key, help, &self.commands) {
                HelpAction::None => {}
                HelpAction::Close => self.help_panel = None,
                HelpAction::Use(line) => {
                    self.help_panel = None;
                    self.editor.set_line(&line);
                }
            }
            return true;
        }
        let typed = command_name(&self.editor.line());
        // Help opens even while a command runs.
        if key.code == KeyCode::Enter && self.job.is_some() && self.is_builtin(&typed, "help") {
            self.editor.set_line("");
            self.open_help();
            return true;
        }
        let page = terminal::size().map_or(10, |(_, h)| (h as usize / 2).max(1));
        match key.code {
            // Inline, the terminal scrolls its own output.
            KeyCode::PageUp | KeyCode::PageDown if self.tui == Tui::Inline => return true,
            KeyCode::PageUp => {
                self.scroll += page;
                return true;
            }
            KeyCode::PageDown => {
                self.scroll = self.scroll.saturating_sub(page);
                return true;
            }
            // One command at a time; the typed line waits.
            KeyCode::Enter if self.job.is_some() => return true,
            _ => {}
        }
        let completer = Completions {
            app: self.app.completer(),
            tui: self.builtins.contains(&"tui"),
        };
        let ctx = Context {
            commands: &self.commands,
            completer: &completer,
        };
        match self.editor.handle(key, &ctx) {
            Action::None => true,
            Action::Exit => false,
            Action::ClearScreen => {
                self.clear();
                true
            }
            Action::Submit(line) if line.is_empty() => true,
            Action::Submit(line) => self.submit(&line),
        }
    }

    /// Run `line` as if typed and sent. False to quit.
    pub fn submit(&mut self, line: &str) -> bool {
        if let Some(path) = &self.history_path {
            save_history(path, line, self.history_lines);
        }
        let name = command_name(line);
        if self.is_builtin(&name, "help") {
            self.open_help();
            return true;
        }
        self.transcript.push(Item::Echo(line.to_owned()));
        self.scroll = 0;
        // `quit` is another name for the built-in exit, unless the app has a `quit` of its own.
        let quit = name == "quit" && self.builtins.contains(&"exit") && !self.commands.iter().any(|c| c.name == "quit");
        if self.is_builtin(&name, "exit") || quit {
            return false;
        }
        if self.is_builtin(&name, "clear") {
            self.clear();
            return true;
        }
        if self.is_builtin(&name, "settings") {
            self.open_settings();
            return true;
        }
        if self.is_builtin(&name, "release-notes") {
            self.open_release_notes();
            return true;
        }
        if self.is_builtin(&name, "tui") {
            self.tui_command(line.split_whitespace().nth(1));
            return true;
        }
        self.with_ctx(|app, ctx| app.run(line, ctx)).0
    }

    /// `/tui`, `/tui fullscreen` or `/tui default`.
    fn tui_command(&mut self, arg: Option<&str>) {
        let say = |tui: Tui| match tui {
            Tui::Fullscreen => "Full screen: the whole terminal, until you quit (/tui fullscreen).",
            Tui::Inline => "In the terminal's own scrollback: output stays after you quit (/tui default).",
        };
        let Some(arg) = arg else {
            let other = match self.tui {
                Tui::Fullscreen => Tui::Inline,
                Tui::Inline => Tui::Fullscreen,
            };
            self.transcript.push(Item::Line(Kind::Out, say(self.tui).into()));
            self.transcript
                .push(Item::Line(Kind::Dim, format!("/tui {} to switch.", other.name())));
            return;
        };
        let Some(tui) = Tui::parse(arg) else {
            self.transcript.push(Item::Line(
                Kind::Error,
                format!("/tui takes fullscreen or default, not {arg:?}"),
            ));
            return;
        };
        self.tui = tui;
        if let Some(path) = &self.tui_path {
            let _ = std::fs::write(path, format!("{}\n", tui.name()));
        }
        self.transcript.push(Item::Line(Kind::Good, say(tui).into()));
    }

    /// The whole screen as rows, for a `w`×`h` terminal.
    pub fn render(&mut self, w: usize, h: usize) -> Frame {
        let (w, h) = (w.max(20), h.max(8));
        if let Some(panel) = self.panel.as_mut().filter(|p| p.full_screen()) {
            // Exactly `h` rows, so nothing of the last frame is left below.
            let mut rows = panel.rows(w, h);
            rows.resize(h, vec![]);
            return Frame { rows, cursor: None };
        }
        if self.scroll > 0 {
            // Clamped as `compose` will, under the status row and input box,
            // so the status row never claims more than there is.
            let rows = self.transcript_rows(w).len();
            self.scroll = self.scroll.min(rows.saturating_sub(h.saturating_sub(5)));
        }
        let panel_h = (h * 4 / 5).max(14).min(h);
        let (mut bottom, cursor) = self.bottom(w, panel_h, h);
        // Too tall for the screen: the top goes, so the input box stays.
        let over = bottom.len().saturating_sub(h);
        bottom.drain(..over);
        let cursor = cursor.and_then(|(col, row)| row.checked_sub(over).map(|r| (col, h - bottom.len() + r)));
        self.compose(w, h, bottom, cursor)
    }

    /// What goes under the output: an open panel (no cursor), or the
    /// status bar or menu, the input box and the footer, with the cursor's
    /// column and row among them. Panels get `panel_h` rows, and the menu
    /// fits in `h` with the input box.
    fn bottom(&mut self, w: usize, panel_h: usize, h: usize) -> (Vec<Row>, Option<(usize, usize)>) {
        if let Some(panel) = &mut self.panel {
            return (panel.rows(w, panel_h), None);
        }
        if let (Some(panel), Some(help)) = (&mut self.help_panel, &self.help) {
            return (panel.rows(help, &self.commands, w, panel_h), None);
        }
        if let Some(panel) = &mut self.settings_panel {
            return (panel.rows(w, panel_h), None);
        }
        if let (Some(panel), Some(releases)) = (&mut self.notes_panel, &self.releases) {
            return (panel.rows(releases, w, panel_h), None);
        }

        let completer = Completions {
            app: self.app.completer(),
            tui: self.builtins.contains(&"tui"),
        };
        let ctx = Context {
            commands: &self.commands,
            completer: &completer,
        };
        let s = self.editor.suggestions(&ctx);
        let mut bottom = if s.items.is_empty() {
            vec![self.status_row(w)]
        } else {
            let typed: String = self
                .editor
                .line()
                .chars()
                .take(self.editor.cursor())
                .skip(s.start)
                .collect();
            // The input box's four rows and the menu's count row left over.
            let max = self.menu_rows.min(h.saturating_sub(5)).max(1);
            self.skin
                .menu(&s.items, self.editor.selected(s.items.len()), &typed, max)
        };
        let line = self.editor.line();
        let name = command_name(&line);
        let input = skin::Input {
            prompt: &self.prompt,
            line: &line,
            cursor: self.editor.cursor(),
            ghost: self.editor.ghost(&ctx),
            placeholder: &self.placeholder,
            known_command: line.starts_with('/') && self.commands.iter().any(|c| c.name == name),
        };
        let (input, cursor_col) = self.skin.input(&input, w);
        bottom.push(self.skin.rule(w));
        let input_at = bottom.len();
        bottom.push(input);
        bottom.push(self.skin.rule(w));
        let footer = if self.editor.ctrl_c_armed() {
            "Press Ctrl+C again to quit"
        } else {
            self.footer.as_str()
        };
        bottom.push(self.skin.footer(footer, w));
        (bottom, Some((cursor_col, input_at)))
    }

    /// The transcript in the rows `bottom` leaves, then `bottom`.
    fn compose(&mut self, w: usize, h: usize, bottom: Vec<Row>, cursor: Option<(usize, usize)>) -> Frame {
        let area = h.saturating_sub(bottom.len());
        let rows = self.transcript_rows(w);
        let max_scroll = rows.len().saturating_sub(area);
        self.scroll = self.scroll.min(max_scroll);
        let end = rows.len() - self.scroll;
        let mut out: Vec<Row> = rows[end.saturating_sub(area)..end].to_vec();
        out.resize(area, vec![]);
        out.extend(bottom);
        out.truncate(h);
        Frame { rows: out, cursor }
    }

    /// The transcript as screen rows.
    pub fn transcript_rows(&self, w: usize) -> Vec<Row> {
        self.item_rows(&self.transcript, w)
    }

    fn item_rows(&self, items: &[Item], w: usize) -> Vec<Row> {
        let mut rows = Vec::new();
        for item in items {
            match item {
                Item::Header => rows.extend(self.skin.header(self.logo.rows(self.logo_frame), &self.header, w)),
                Item::Line(kind, text) => rows.extend(self.skin.line(*kind, text, w)),
                Item::Row(pieces) => rows.extend(self.skin.pieces(pieces, w)),
                Item::Echo(text) => rows.extend(self.skin.echo(&self.prompt, text, w)),
                Item::Rows(r) => rows.extend(r.iter().cloned()),
            }
        }
        rows
    }

    /// Above the box: what is running or how far back you scrolled, and
    /// notices on the right, most important first, as many as fit.
    fn status_row(&self, w: usize) -> Row {
        let job = self.job.as_ref().map(|j| (j.label.as_str(), j.started));
        let left = if let Some((label, started)) = job.or(self.pretend.as_ref().map(|(l, t)| (l.as_str(), *t))) {
            job_left(self.spinner.as_ref(), label, started.elapsed())
        } else if self.scroll > 0 {
            scrolled_left(self.scroll)
        } else {
            Row::new()
        };
        let mut notices: Vec<Notice> = Vec::new();
        if let Some(u) = &self.updates
            && let Some(n) = u.note
        {
            notices.push(n.notice(&u.command));
        }
        for n in &self.notices {
            if !notices.iter().any(|m| m.text == n.text) {
                notices.push(n.clone());
            }
        }
        self.skin.status(left, &notices, w)
    }

    /// Inline: print the output not yet printed into the scrollback, then
    /// redraw the input box (or panel) under it in place of the last one.
    fn draw_inline(&mut self, out: &mut impl Write) -> io::Result<()> {
        let (w, h) = terminal::size().map(|(w, h)| (w as usize, h as usize))?;
        let (w, h) = (w.max(20), h.max(4));
        // The header stays live while the logo animates, then is printed.
        let animating = self.inline.committed == 0 && self.logo_frame + 1 < self.logo.frames();
        let done = if animating { 0 } else { self.transcript.len() };
        let commit = self.item_rows(&self.transcript[self.inline.committed.min(done)..done], w);
        let mut live = if animating { self.transcript_rows(w) } else { vec![] };
        let above = live.len();
        let (bottom, cursor) = self.bottom(w, (h * 4 / 5).max(14).min(h - 1), h - 1);
        live.extend(bottom);
        // Never taller than the screen, or the top couldn't be redrawn.
        let cut = live.len().saturating_sub(h - 1);
        live.drain(..cut);
        let cursor = cursor.and_then(|(col, row)| (above + row).checked_sub(cut).map(|r| (col, r)));

        queue!(out, Hide)?;
        if std::mem::take(&mut self.inline.wipe) {
            queue!(out, Clear(ClearType::All), MoveTo(0, 0))?;
        } else {
            self.to_live_top(out)?;
        }
        queue!(out, Clear(ClearType::FromCursorDown))?;
        for row in &commit {
            print_row(out, row, w)?;
            queue!(out, Print("\r\n"))?;
        }
        for (i, row) in live.iter().enumerate() {
            if i > 0 {
                queue!(out, Print("\r\n"))?;
            }
            print_row(out, row, w)?;
        }
        let last = live.len().saturating_sub(1);
        match cursor {
            Some((col, row)) => {
                if last > row {
                    queue!(out, MoveUp((last - row) as u16))?;
                }
                queue!(out, MoveToColumn(col as u16), Show)?;
                self.inline.cursor_row = row;
            }
            None => {
                queue!(out, MoveToColumn(0))?;
                self.inline.cursor_row = last;
            }
        }
        self.inline.committed = done;
        out.flush()
    }

    /// The cursor to the top of what `draw_inline` redraws.
    fn to_live_top(&self, out: &mut impl Write) -> io::Result<()> {
        if self.inline.cursor_row > 0 {
            queue!(out, MoveUp(self.inline.cursor_row as u16))?;
        }
        queue!(out, MoveToColumn(0))
    }

    /// Inline: clear the input box and what's around it, leaving the output.
    fn erase_live(&mut self, out: &mut impl Write) -> io::Result<()> {
        self.to_live_top(out)?;
        queue!(out, Clear(ClearType::FromCursorDown), Show)?;
        self.inline.cursor_row = 0;
        Ok(())
    }

    fn draw(&mut self, out: &mut impl Write) -> io::Result<()> {
        let (w, h) = terminal::size().map(|(w, h)| (w as usize, h as usize))?;
        let frame = self.render(w, h);
        // Smaller than `render` draws: the bottom of the frame, input box and all.
        let skip = frame.rows.len().saturating_sub(h);
        queue!(out, Hide)?;
        for (y, row) in frame.rows.iter().skip(skip).enumerate() {
            queue!(out, MoveTo(0, y as u16))?;
            print_row(out, row, w)?;
            queue!(out, Clear(ClearType::UntilNewLine))?;
        }
        if let Some((x, y)) = frame.cursor
            && y >= skip
        {
            queue!(out, MoveTo(x.min(w.saturating_sub(1)) as u16, (y - skip) as u16), Show)?;
        }
        out.flush()
    }
}

/// The command a line names: its first word without the slash, lower case.
fn command_name(line: &str) -> String {
    line.split_whitespace()
        .next()
        .unwrap_or("")
        .trim_start_matches('/')
        .to_lowercase()
}

fn load_history(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .map(|s| s.lines().filter(|l| !l.trim().is_empty()).map(str::to_owned).collect())
        .unwrap_or_default()
}

fn save_history(path: &Path, line: &str, keep: usize) {
    let mut lines = load_history(path);
    if lines.last().map(String::as_str) == Some(line) {
        return;
    }
    lines.push(line.to_owned());
    let skip = lines.len().saturating_sub(keep);
    let _ = std::fs::write(path, lines[skip..].join("\n") + "\n");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::row_text;
    use crossterm::event::KeyModifiers;

    struct Echoer {
        ran: Vec<String>,
    }

    impl App for Echoer {
        type Msg = String;
        type Panel = NoPanel;

        fn commands(&self) -> Vec<CommandInfo> {
            vec![CommandInfo::new("greet", "Say hello")]
        }

        fn run(&mut self, line: &str, ctx: &mut Ctx<Self>) {
            self.ran.push(line.to_owned());
            let line = line.to_owned();
            ctx.start("Greeting", move || format!("hello from {line}"));
        }

        fn done(&mut self, msg: String, _ctx: &mut Ctx<Self>) {
            self.ran.push(msg);
        }

        fn notices(&self) -> Vec<Notice> {
            vec![Notice::warn("Not set up yet")]
        }
    }

    fn text(f: &Frame) -> String {
        f.rows.iter().map(row_text).collect::<Vec<_>>().join("\n")
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn a_frame_fills_the_screen_with_the_input_at_the_bottom() {
        let mut shell = Shell::new(Echoer { ran: vec![] }).title("Demo");
        let f = shell.render(80, 24);
        assert_eq!(f.rows.len(), 24);
        let shown = text(&f);
        assert!(shown.contains("Demo"), "the default header names the app");
        assert!(shown.contains("Not set up yet"));
        assert!(shown.contains("Type / for commands"));
        assert_eq!(f.cursor, Some((2, 21)));
    }

    #[test]
    fn builtins_follow_the_apps_commands() {
        let shell = Shell::new(Echoer { ran: vec![] }).title("Demo");
        let names: Vec<&str> = shell.commands.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["greet", "help", "tui", "clear", "exit"]);
        let shell = shell.without_help();
        assert!(!shell.commands.iter().any(|c| c.name == "help"));
    }

    #[test]
    fn lines_go_to_the_app_and_builtins_to_the_shell() {
        let mut shell = Shell::new(Echoer { ran: vec![] });
        assert!(shell.submit("/greet now"));
        assert_eq!(shell.app().ran, ["/greet now"]);
        assert_eq!(shell.transcript().last(), Some(&Item::Echo("/greet now".into())));
        // The work waits on a thread; joining it hands the result back.
        let job = shell.job.take().unwrap();
        let msg = job.handle.join().unwrap();
        shell.with_ctx(|app, ctx| app.done(msg, ctx));
        assert_eq!(shell.app().ran[1], "hello from /greet now");

        assert!(shell.submit("/help"));
        assert!(shell.help_panel.is_some());
        assert!(text(&shell.render(80, 30)).contains("Ctrl+L"));
        shell.key(key(KeyCode::Esc));
        assert!(shell.help_panel.is_none());
        assert!(shell.submit("/clear"));
        assert_eq!(shell.transcript(), [Item::Header]);
        assert!(!shell.submit("/exit"));
    }

    #[test]
    fn the_menu_opens_above_the_input() {
        let mut shell = Shell::new(Echoer { ran: vec![] });
        shell.key(key(KeyCode::Char('/')));
        shell.key(key(KeyCode::Char('g')));
        let shown = text(&shell.render(80, 24));
        assert!(shown.contains("❯ /greet") && shown.contains("Say hello"));
    }

    #[test]
    fn tui_switches_and_remembers() {
        let path = std::env::temp_dir().join(format!("abalone-tui-{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut shell = Shell::new(Echoer { ran: vec![] }).tui_file(&path);
        assert_eq!(shell.tui_mode(), Tui::Fullscreen, "full screen by default");
        assert!(shell.submit("/tui default"));
        assert_eq!(shell.tui_mode(), Tui::Inline);
        assert!(shell.app().ran.is_empty(), "the shell answers /tui itself");
        let again = Shell::new(Echoer { ran: vec![] }).tui_file(&path);
        assert_eq!(again.tui_mode(), Tui::Inline, "the choice is kept");
        shell.submit("/tui sideways");
        assert!(matches!(shell.transcript().last(), Some(Item::Line(Kind::Error, _))));
        shell.submit("/tui fullscreen");
        assert_eq!(shell.tui_mode(), Tui::Fullscreen);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_input_stays_on_a_short_screen_with_the_menu_open() {
        for h in [8, 10, 12] {
            let mut shell = Shell::new(Echoer { ran: vec![] });
            shell.key(key(KeyCode::Char('/')));
            let f = shell.render(60, h);
            assert_eq!(f.rows.len(), h);
            let (_, y) = f.cursor.expect("a cursor");
            assert!(y < h, "the cursor is on screen at h={h}");
            assert!(row_text(&f.rows[y]).starts_with("❯ /"), "the input box at h={h}");
            assert!(text(&f).contains("/help for commands"), "the footer at h={h}");
        }
    }

    #[test]
    fn the_status_row_never_claims_more_than_there_is() {
        let mut shell = Shell::new(Echoer { ran: vec![] });
        shell.set_notices(vec![]);
        shell.key(key(KeyCode::PageUp));
        assert!(!text(&shell.render(60, 24)).contains("more lines"), "nothing to scroll");
        for i in 0..40 {
            shell.transcript_mut().push(Item::Line(Kind::Out, format!("line {i}")));
        }
        for _ in 0..6 {
            shell.key(key(KeyCode::PageUp));
        }
        let shown = text(&shell.render(60, 24));
        let rows = shell.transcript_rows(60).len();
        assert!(shown.contains(&format!("↓ {} more lines", rows - 19)), "{shown}");
    }

    #[test]
    fn an_apps_own_quit_is_the_apps() {
        struct Game;
        impl App for Game {
            type Msg = ();
            type Panel = NoPanel;
            fn commands(&self) -> Vec<CommandInfo> {
                vec![CommandInfo::new("quit", "Quit the game")]
            }
            fn run(&mut self, _line: &str, _ctx: &mut Ctx<Self>) {}
        }
        assert!(Shell::new(Game).submit("/quit"), "the app's quit runs");
        assert!(!Shell::new(Echoer { ran: vec![] }).submit("/quit"), "else it exits");
    }

    #[test]
    fn a_space_in_an_empty_box_is_just_a_space() {
        let mut shell = Shell::new(Echoer { ran: vec![] });
        shell.key(key(KeyCode::Char(' ')));
        shell.render(60, 12);
    }

    #[test]
    fn the_menu_offers_tuis_two_ways() {
        let mut shell = Shell::new(Echoer { ran: vec![] });
        for c in "/tui ".chars() {
            shell.key(key(KeyCode::Char(c)));
        }
        let shown = text(&shell.render(80, 24));
        assert!(shown.contains("fullscreen") && shown.contains("default"), "{shown}");
    }

    #[test]
    fn release_notes_print_the_version_picked() {
        let changelog = "# Changelog\n\n## [1.1.0] - 2026-10-21\n\n### Fixed\n\n- A fix.\n\n## [1.0.0] - 2026-10-01\n\n### Added\n\n- First.\n";
        let mut shell = Shell::new(Echoer { ran: vec![] }).release_notes(changelog);
        assert!(shell.commands().iter().any(|c| c.name == "release-notes"));
        assert!(shell.submit("/release-notes"));
        let shown = text(&shell.render(80, 30));
        assert!(shown.contains("Release notes") && shown.contains("Version 1.1.0"));
        shell.key(key(KeyCode::Down));
        shell.key(key(KeyCode::Enter));
        let rows: Vec<String> = shell.transcript_rows(80).iter().map(row_text).collect();
        assert!(rows.iter().any(|r| r.contains("● Version 1.1.0:")));
        assert!(rows.iter().any(|r| r.contains("· A fix.")));
        assert!(!rows.iter().any(|r| r.contains("Version 1.0.0")));
        let plain = Shell::new(Echoer { ran: vec![] });
        assert!(
            !plain.commands().iter().any(|c| c.name == "release-notes"),
            "only with notes"
        );
    }
}
