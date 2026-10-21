//! Every part swapped: theme, logo, spinner, a skin tweak, help, settings,
//! a notice, and a panel of the app's own.
//!
//!     cargo run --example custom
//!     cargo run --example custom -- --inline

use std::sync::{Arc, Mutex};
use std::time::Duration;

use shellkit::skin::{self, Skin};
use shellkit::widgets::{self, Item};
use shellkit::{
    App, Choice, CommandInfo, Completer, Ctx, Frames, Help, KeyEvent, Notice, Panel, PanelEvent, PixelLogo, Row, Shell,
    SimpleSettings, Style, Suggestion, Theme, Tui, outln, rgb,
};

/// What `/release-notes` lists; an app would `include_str!` its CHANGELOG.md.
const CHANGELOG: &str = "# Changelog

## [0.2.0] - 2026-10-21

### Added

- Release notes, from the changelog built into the app.
- /tui default, to run in the terminal's own scrollback.

### Fixed

- The installed-update notice is green.

## [0.1.0] - 2026-10-01

### Added

- The first version.
";

/// Orange on dark grey.
const THEME: Theme = Theme {
    accent: shellkit::Color::Rgb { r: 255, g: 140, b: 0 },
    ..Theme::DEFAULT
};

/// A 6×6 heart, filling in over three frames.
fn heart() -> PixelLogo {
    const ART: [&str; 6] = [".XX.XX", "XXXXXX", "XXXXXX", ".XXXX.", "..XX..", "......"];
    let frame = |rows: usize| {
        ART.iter()
            .enumerate()
            .map(|(r, line)| {
                line.chars()
                    .map(|c| (c == 'X' && r >= 6 - rows).then_some(rgb((255, 90, 90))))
                    .collect()
            })
            .collect()
    };
    PixelLogo {
        frames: vec![frame(2), frame(4), frame(6)],
    }
}

/// The default footer, tagged.
struct Tagged;

impl Skin for Tagged {
    fn footer(&self, text: &str, w: usize) -> Row {
        let mut row = skin::footer(text, w);
        row.push((Style::fg(THEME.accent).bold(), "  · demo build".into()));
        row
    }
}

/// Fruit for `/pick`'s argument.
struct Fruit;

impl Completer for Fruit {
    fn complete(&self, cmd: &str, _flag: Option<&str>, args: &[&str], word: &str) -> Vec<Suggestion> {
        if cmd != "pick" || !args.is_empty() {
            return vec![];
        }
        ["apple", "banana", "cherry"]
            .iter()
            .filter(|f| f.starts_with(word))
            .map(|f| Suggestion::new(*f, *f, "a fruit"))
            .collect()
    }
}

/// A list to choose from, full screen.
pub struct ListPanel {
    sel: usize,
}

pub enum ListEvent {
    Chosen(usize),
}

impl Panel for ListPanel {
    type Event = ListEvent;

    fn key(&mut self, key: KeyEvent) -> PanelEvent<ListEvent> {
        if widgets::is_close(&key) {
            return PanelEvent::Close;
        }
        if key.code == shellkit::KeyCode::Enter {
            return PanelEvent::Event(ListEvent::Chosen(self.sel));
        }
        widgets::step(&mut self.sel, widgets::step_of(&key), 20);
        PanelEvent::None
    }

    fn rows(&mut self, w: usize, h: usize) -> Vec<Row> {
        let items = (0..20).map(|i| Item::new(format!("Item {i}"))).collect();
        let body = widgets::list_rows(items, self.sel, widgets::body_height(h, false));
        widgets::frame(
            w,
            h,
            widgets::title("A list"),
            None,
            body,
            "↑↓ move · Enter picks · Esc closes",
        )
    }
}

pub struct Demo {
    fruit: Arc<Mutex<Option<String>>>,
}

impl App for Demo {
    type Msg = String;
    type Panel = ListPanel;

    fn commands(&self) -> Vec<CommandInfo> {
        vec![
            CommandInfo::new("pick", "Pick a fruit"),
            CommandInfo::new("list", "Open a list"),
            CommandInfo::new("wait", "Wait a few seconds").flag("--secs", "How long", Some("N")),
        ]
    }

    fn run(&mut self, line: &str, ctx: &mut Ctx<Self>) {
        let words = shellkit::split_words(line);
        match words[0].trim_start_matches('/') {
            "pick" => {
                let fruit = words.get(1).cloned();
                *self.fruit.lock().unwrap() = fruit.clone();
                outln!("Picked {}", fruit.as_deref().unwrap_or("nothing"));
                ctx.refresh();
            }
            "list" => ctx.open(ListPanel { sel: 0 }),
            "wait" => {
                let secs = words.get(2).and_then(|s| s.parse().ok()).unwrap_or(3);
                ctx.start("Waiting", move || {
                    std::thread::sleep(Duration::from_secs(secs));
                    format!("waited {secs}s")
                });
            }
            other => shellkit::errln!("No command {other}"),
        }
    }

    fn done(&mut self, msg: String, _ctx: &mut Ctx<Self>) {
        outln!("Done: {msg}");
    }

    fn panel_event(&mut self, ListEvent::Chosen(i): ListEvent, ctx: &mut Ctx<Self>) {
        ctx.close_panel();
        outln!("Chose item {i}");
    }

    fn header(&self) -> Vec<Row> {
        vec![
            vec![(Style::default().bold(), "Custom demo".into())],
            vec![(Style::fg(THEME.muted), "Every part of the shell, swapped".into())],
            vec![(Style::fg(THEME.muted), "Try /pick, /list, /wait and /settings".into())],
        ]
    }

    fn notices(&self) -> Vec<Notice> {
        match &*self.fruit.lock().unwrap() {
            Some(f) => vec![Notice::info(format!("Fruit: {f}"))],
            None => vec![Notice::warn("No fruit picked · /pick one")],
        }
    }

    fn placeholder(&self) -> String {
        "Try /pick apple".into()
    }

    fn completer(&self) -> &dyn Completer {
        &Fruit
    }
}

fn main() -> anyhow::Result<()> {
    // `--inline` starts in the terminal's own scrollback; /tui switches.
    let inline = std::env::args().any(|a| a == "--inline");
    shell(if inline { Tui::Inline } else { Tui::Fullscreen }).run()
}

/// The shell, ready to run (the docs' pictures draw it too).
pub fn shell(tui: Tui) -> Shell<Demo> {
    let fruit = Arc::new(Mutex::new(None));
    let colour = Arc::new(Mutex::new(Some("orange".to_owned())));
    let fast = Arc::new(Mutex::new(true));
    let (f1, c1, c2, s1, s2) = (fruit.clone(), colour.clone(), colour, fast.clone(), fast);
    let help = Help::new()
        .text(
            "General",
            "A demo of everything shellkit can change.\n\n# Commands\n  /pick takes a fruit; Tab completes it.\n  /list opens a panel of the app's own.",
        )
        .commands("Commands")
        .link("https://example.com/demo");
    Shell::new(Demo { fruit })
        .title("Custom demo")
        .theme(THEME)
        .logo(heart())
        .spinner(Frames::new(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]))
        .skin(Tagged)
        .help(help)
        .tui(tui)
        .release_notes(CHANGELOG)
        .settings(move || {
            let (f1, c1, c2, s1, s2) = (f1.clone(), c1.clone(), c2.clone(), s1.clone(), s2.clone());
            SimpleSettings::new()
                .status("Fruit", move || {
                    f1.lock().unwrap().clone().unwrap_or_else(|| "None".into())
                })
                .choice(
                    "Colour",
                    || {
                        vec![
                            Choice::new("orange", "Orange", "warm").styled(Style::fg(rgb((255, 140, 0)))),
                            Choice::new("teal", "Teal", "cool").styled(Style::fg(rgb((0, 160, 160)))),
                        ]
                    },
                    move || c1.lock().unwrap().clone(),
                    move |v| {
                        *c2.lock().unwrap() = Some(v);
                        Ok(())
                    },
                )
                .gap()
                .toggle(
                    "Fast mode",
                    move || *s1.lock().unwrap(),
                    move |v| {
                        *s2.lock().unwrap() = v;
                        Ok(())
                    },
                )
        })
}
