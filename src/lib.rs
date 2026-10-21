//! A full-screen interactive shell for command-line apps.
//!
//! Output fills the top of the terminal and an input box stays pinned to
//! the bottom: `/` opens a menu of commands and their flags, a status bar
//! shows what's running and any notices, and `/help` and `/settings` open
//! panels in place of the input box. Commands run on a worker thread and
//! print through [`out`], so they work the same from the command line.
//!
//! ```no_run
//! use abalone::{App, CommandInfo, Ctx, NoPanel, Shell, outln};
//!
//! struct Hello;
//!
//! impl App for Hello {
//!     type Msg = ();
//!     type Panel = NoPanel;
//!
//!     fn commands(&self) -> Vec<CommandInfo> {
//!         vec![CommandInfo::new("greet", "Say hello").flag("--name", "Who to greet", Some("NAME"))]
//!     }
//!
//!     fn run(&mut self, line: &str, ctx: &mut Ctx<Self>) {
//!         let line = line.to_owned();
//!         ctx.start("Greeting", move || outln!("Hello! You said {line}"));
//!     }
//! }
//!
//! Shell::new(Hello).title("Hello").run().unwrap();
//! ```
//!
//! Every part can be swapped or extended:
//!
//! - colours: [`Theme`] (or a TOML file, with the `theme-file` feature);
//! - the logo: [`Logo`], with [`TextLogo`], [`PixelLogo`] and [`half_blocks`] to build one;
//! - the spinner: [`Spinner`], or any `Fn(Duration) -> (Style, String)`;
//! - the header, menu, input box, status bar and footer: [`skin::Skin`];
//! - help: [`Help`] tabs of text, the commands, or rows of your own;
//! - settings: [`SimpleSettings`], or [`SettingsModel`] for settings that lead to one another;
//! - panels of your own: [`Panel`], built from [`widgets`];
//! - full screen or in the terminal's own scrollback: [`Tui`], or `/tui`;
//! - `/release-notes` from the app's changelog: [`Shell::release_notes`].
//!
//! Around the shell, for any command-line app:
//!
//! - [`log`]: a log file per run, for bug reports, and [`log::guard`] for `main`;
//! - [`files`] (`files` feature): the data folder and settings in a TOML file;
//! - [`install`] and [`update`] (`install`, `update` features): installing
//!   itself and updating from GitHub releases, with the install scripts and
//!   release workflow ([`release`]).
//!
//! It runs on Windows, macOS and Linux.

pub mod editor;
#[cfg(feature = "files")]
pub mod files;
pub mod help;
pub mod image;
pub mod log;
pub mod logo;
pub mod notes;
pub mod out;
pub mod settings;
mod shell;
pub mod skin;
pub mod spinner;
pub mod status;
mod style;
pub mod term;
mod theme;
pub mod widgets;

#[cfg(feature = "install")]
pub mod install;
#[cfg(feature = "update")]
pub mod release;
#[cfg(feature = "update")]
pub mod update;

pub use editor::{CommandInfo, Completer, FlagInfo, NoCompleter, Suggestion, split_words};
pub use help::{Help, HelpTab};
pub use logo::{DefaultLogo, Logo, NoLogo, PixelLogo, TextLogo};
pub use out::{Kind, Line};
pub use settings::{
    Choice, Field, Loaded, Outcome, Picker, SettingRow, SettingsModel, SimpleSettings, StatusGroups, Value,
};
pub use shell::{App, Ctx, Frame, Item, NoPanel, Panel, PanelEvent, Shell, Tui};
pub use spinner::{Classic, Frames, Spinner};
pub use status::{Notice, UpdateNote};
pub use style::{
    Row, Style, char_width, fit_row, fit_width, half_blocks, highlight, pad, rgb, row_text, row_width, text_width, wrap,
};
pub use theme::{Theme, for_light, hex, light_background, readable, set_light_background, set_theme, theme};

/// Keys, for panels.
pub use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
/// The colour type rows are drawn with.
pub use crossterm::style::Color;
