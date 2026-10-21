# shellkit

A full-screen interactive shell for Rust command-line apps, on [crossterm](https://crates.io/crates/crossterm).

Output fills the top of the terminal and an input box stays pinned to the bottom. Typing `/` opens a menu of commands and their flags, a status bar shows what's running and any notices, and `/help` and `/settings` open panels in place of the input box. Commands run on a worker thread and print through `shellkit::out`, so the same code works from the command line and inside the shell.

Every part has a default and can be replaced or built on: the theme, logo, spinner, header, menu, input box, status bar, footer, help, settings, and panels of your own. Around the shell it has what most command-line apps end up writing: a log file per run for bug reports, the data folder and a TOML settings file, and (with the `install` and `update` features) installing itself and updating from GitHub releases.

It runs on Windows, macOS and Linux.

## Quick start

```rust
use shellkit::{App, CommandInfo, Ctx, NoPanel, Shell, outln};

struct Hello;

impl App for Hello {
    type Msg = ();
    type Panel = NoPanel;

    fn commands(&self) -> Vec<CommandInfo> {
        vec![CommandInfo::new("greet", "Say hello").flag("--name", "Who to greet", Some("NAME"))]
    }

    fn run(&mut self, line: &str, ctx: &mut Ctx<Self>) {
        let line = line.to_owned();
        ctx.start("Greeting", move || outln!("Hello! You said {line}"));
    }
}

fn main() -> anyhow::Result<()> {
    Shell::new(Hello).title("Hello").run()
}
```

The shell adds `/help`, `/tui`, `/clear` and `/exit` after your commands, with `/settings` and `/release-notes` when you give it settings and a changelog. A command of your own with one of those names replaces the built-in. Try the examples (`--inline` starts `custom` in the terminal's own scrollback):

```sh
cargo run -p shellkit --example minimal
cargo run -p shellkit --example custom
```

`custom` swaps every part listed below.

## Features

| Feature | Adds |
| --- | --- |
| `clap` | `CommandInfo::from_clap(&Cli::command())`: the menu and help from a clap parser |
| `theme-file` | `Theme::from_toml`: themes kept in a file |
| `files` | `files::data_dir`, `files::load_toml` and `save_toml` for settings, `files::open` for a folder |
| `install` | `install::Layout` and self-installation into the user's local data folder, `install::InstallScript` for `install.ps1` and `install.sh`; implies `files` |
| `update` | `update::Updater` for GitHub releases, `release` templates for publishing them; implies `install` |

## Making it yours

### The app

`App` is the one trait you implement. Only `commands` and `run` are required:

| Method | What it's for |
| --- | --- |
| `run(line, ctx)` | A line the user sent. Put slow work on a thread with `ctx.start(label, work)`. |
| `done(msg, ctx)` | A worker thread finished with your `Msg`. |
| `launch(ctx)` | Once, when the screen is up. |
| `tick(ctx)` | Every tick (80 ms): timers and background checks. |
| `refresh()` | After a command or settings change: re-read your state. |
| `header()` | The lines beside the logo. |
| `notices()` | Messages on the right of the status bar, most important first. |
| `footer()`, `placeholder()` | The line under the input box, and the text in an empty one. |
| `completer()` | Values for the menu: a flag's value, or a command's arguments. |
| `panel_event(e, ctx)` | What your own panel asked for. |

`Ctx` is how callbacks talk to the shell: `start`, `open` (a panel), `close_panel`, `open_help`, `open_settings`, `clear`, `exit`, `set_line`, `refresh`.

### Theme

```rust
const MINE: Theme = Theme { accent: Color::Rgb { r: 255, g: 140, b: 0 }, ..Theme::DEFAULT };
Shell::new(app).theme(MINE)
```

The roles are `accent`, `bright`, `muted`, `rule`, `danger`, `warn`, `good`, `echo_bg` and `on_bright`. Your own panels can read the active theme with `shellkit::theme()`. With `theme-file`, `Theme::from_toml(text, Theme::DEFAULT)` reads any of the roles as `"#rrggbb"`.

### Logo

The default is a small terminal window. Use `TextLogo` for rows drawn by hand (`TextLogo::text(include_str!("logo.txt"), style)` loads one from a file), `PixelLogo` for pixel art in half blocks (one image per frame, so it can animate), `NoLogo` for none, or implement `Logo`:

```rust
impl Logo for Mine {
    fn frames(&self) -> usize { 10 }               // the launch animation
    fn rows(&self, frame: usize) -> Vec<Row> { … } // every row the same width
}
```

### Spinner

The default is `| / - \`. `Frames::new(&["⠋", "⠙", "⠹", "⠸"])` turns through any glyphs, and any `Fn(Duration) -> (Style, String)` is a spinner too.

### Header, menu, input, status bar, footer

Each is a method of `skin::Skin` with a default. Override one, and call the free function of the same name to build on the default rather than replace it:

```rust
struct Tagged;
impl Skin for Tagged {
    fn footer(&self, text: &str, w: usize) -> Row {
        let mut row = skin::footer(text, w);
        row.push((Style::default().bold(), "  · beta".into()));
        row
    }
}
Shell::new(app).skin(Tagged)
```

### Help

```rust
Help::new()
    .text("General", "What it does.\n\n# Getting started\n  /init sets it up.")
    .commands("Commands")
    .link("https://example.com/my-app")
    .issues("https://example.com/my-app/issues")
```

`# ` starts a bold heading. The shell's keys are added to the first text tab (`without_keys()` leaves them out). `HelpTab::Custom` adds a tab of rows of your own; `Shell::without_help()` drops `/help` so you can handle it yourself.

### Settings

`SimpleSettings` covers switches, choices and text from closures:

```rust
Shell::new(app).settings(|| {
    SimpleSettings::new()
        .status("Version", || "v1.0".into())
        .toggle("Automatic updates", get, set)
        .choice("Colour", options, get, set)
        .gap()
        .secret("API key", get, set)
})
```

For settings that lead to one another (pick a provider, then type its key; a list fetched in the background), implement `SettingsModel`. Each step returns the next `Outcome`: `Changed`, an `Error`, a `Picker` (or `Picker::loading`), or a `Field`. `settings::status_lines` prints the Status tab outside the shell.

### Panels of your own

Implement `Panel` (`key`, `rows`, and optionally `full_screen`, `paste`, `busy`) and open it with `ctx.open(panel)`. `widgets` has what the built-in panels are made of: `frame`, `tabs_row`, `title`, `para`, `field`, `bullet`, `steps`, `list_rows`, `scrolled`, `text_field`, and the keys (`is_close`, `tab_key`, `step_of`).

### Replacing a built-in page

`/help`, `/settings`, `/clear` and `/exit` are added only when your app has no command of that name. List your own `/help` (or `/settings`) in `App::commands`, open a panel of yours from `App::run`, and it replaces the built-in one.

### Output

Print with `outln!`, `errln!`, `out::emit(kind, text)` or `out::row(pieces)`. Outside the shell they go to stdout and stderr; inside it, to the transcript. `out::set_log` also hands warnings and errors to a log of yours.

### Full screen or inline

`/tui fullscreen` (the default) draws over the whole terminal on the alternate screen, and the terminal's own scrollback comes back untouched at the end. `/tui default` runs in the terminal's own scrollback instead: finished output is printed there for good and only the input box, status bar, menu and panels are redrawn, so the output stays after you quit and the terminal scrolls it. Panels that cover the whole screen still open on the alternate screen. `Shell::tui(Tui::Inline)` starts that way, and `Shell::tui_file(path)` remembers the choice.

### Release notes

`Shell::release_notes(include_str!("../CHANGELOG.md"))` adds `/release-notes`: a list of the versions in a [Keep a Changelog](https://keepachangelog.com/en/2.0.0/) changelog with how many entries each has, and "Show all". The one picked prints as `● Version 1.2.0:` and a line per entry. It's built into the app, so it works offline and matches the build. `notes::parse` reads the changelog for anything else.

### Pictures

`image::Image` draws RGBA pixels in half blocks, one cell per pixel across and two down, so a square picture stays square. `resize` and `turn` shape it, `cells` takes a colour function (to dim it, or decide what's see-through) and gives one piece per column so you can put marks on it, and `image::Cache` keeps the resized copy between frames.

## Around the shell

### `main`, and a log for bug reports

```rust
fn main() {
    let cli = Cli::parse();
    let data_dir = shellkit::files::data_dir("my-app", cli.data_dir.as_deref()).unwrap();
    shellkit::log::start(&data_dir.join("logs"), "my-app", "My App 1.0.0", "shell");
    shellkit::log::guard("My App", || match cli.command {
        Some(command) => run(command),
        None if shellkit::term::interactive() => Shell::new(app).run(),
        None => Ok(Cli::command().print_help()?),
    });
}
```

`log::start` opens one log per run (the newest 20 are kept) with the version and OS, and `log::write(level, text)` adds to it; the home folder and user name are scrubbed from every line. `log::guard` logs printed warnings and errors and any crash with its backtrace, prints an error as `error: …` and exits with 1, and after a crash asks for the log. `log::hint()` is " · details in <log>", for messages asking for a report (`App::crashed`, say).

### Files

`files::data_dir("my-app", flag)` is the flag, else `$MY_APP_DATA_DIR`, else `data` in the app's folder: `%LOCALAPPDATA%\my-app` on Windows, `~/Library/Application Support/my-app` on macOS, `~/.local/share/my-app` on Linux. `files::load_toml::<Settings>(&path)` reads settings (a missing file is `Default`), `save_toml` writes them, and `files::open(&dir)` shows a folder in Explorer, Finder or the Linux file manager.

## Installing and updating

`install::Layout::new("my-app")` is the app's folder (above, or `$MY_APP_HOME`): `versions/<version>/my-app` for each version and `bin/my-app`, the copy on PATH (`.exe` on Windows). Your app's `install` command calls `install::install(&layout, VERSION, &current_exe)`, then `install::report(&layout, "My App", VERSION)` to say where it went.

Release files are named by platform: `my-app-windows-x64.exe`, `my-app-linux-x64`, `my-app-macos-arm64` and so on (`install::platform()` is this build's). Arm Windows and Arm Macs fall back to an x64 build when a release has none of their own.

`update::Updater` reads the latest GitHub release's `manifest.json`, downloads and checks this platform's build, and runs it as `my-app install --update`. Your `update` command is `updater.update_command()`, which installs the newest release or says this one is. In the shell:

```rust
let updater = Updater::new("My App", "my-app", "me/my-app", VERSION, &data_dir)?;
Shell::new(app).update_check(Duration::from_secs(6 * 60 * 60), "/update", updater.background(|| auto_update_on()))
```

The status bar then says when an update is available, installed, or failed. `updater.whats_new()` gives the first launch of the new version a "✓ My App updated to v1.2.0 · what's new" block with the notes (`update::news` draws one for anything else). `MY_APP_DISABLE_AUTOUPDATER=1` turns background updates off, and `MY_APP_UPDATE_URL` serves the manifest and exe from somewhere else, for testing.

The release side comes as templates to keep in your repo, each with a test that fails when it drifts:

- `install::InstallScript::new("My App", "my-app", "me/my-app")`: `.render()` is `install.ps1` for Windows (`irm https://raw.githubusercontent.com/me/my-app/main/install.ps1 | iex`), and `.render_sh()` is `install.sh` for macOS and Linux (`curl -fsSL https://raw.githubusercontent.com/me/my-app/main/install.sh | bash`), which adds `bin` to PATH in the shell's startup file.
- `release::NOTES_SCRIPT`: `scripts/release-notes.ps1`, which turns a version's section of a [Keep a Changelog](https://keepachangelog.com/en/2.0.0/) `CHANGELOG.md` into the release description and the in-app notes.
- `release::Workflow::new("my-app", "my-app-cli").render()`: `.github/workflows/release.yml`, which on a tag builds each platform on its own runner (Windows x64, Linux and macOS on x64 and Arm; `.platforms(&[…])` to choose) and publishes them with `manifest.json`. The scripts it runs are PowerShell, which every GitHub runner has; nobody installing the app needs it.

## Using it

It isn't on crates.io yet; depend on it by git, with the features you need:

```toml
[dependencies]
shellkit = { git = "https://github.com/Deaadman/shellkit", features = ["clap", "update"] }
```

To work on shellkit and an app together, point the app at a local copy without changing its dependency, in the app's `Cargo.toml`:

```toml
[patch."https://github.com/Deaadman/shellkit"]
shellkit = { path = "../shellkit" }
```

## License

[MIT](LICENSE).
