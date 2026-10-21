# Abalone

[![CI](https://img.shields.io/github/actions/workflow/status/Deaadman/abalone/ci.yml?branch=main&style=flat-square&label=CI)](https://github.com/Deaadman/abalone/actions/workflows/ci.yml) ![Windows, macOS, Linux](https://img.shields.io/badge/Windows%20%7C%20macOS%20%7C%20Linux-blue?style=flat-square) [![License](https://img.shields.io/badge/license-MIT-blue?style=flat-square)](LICENSE)

Abalone gives your Rust command-line app a full-screen shell. Output fills the screen and an input box stays at the bottom. Typing `/` opens a menu of your commands, a status bar shows what's running, and `/help` and `/settings` open in place of the input box.

Every part has a sensible default, and you can change any of it: the colours, logo, spinner, help, settings, and panels of your own.

It was inspired by [Claude Code](https://claude.com/claude-code)'s terminal interface, to make apps that feel like it easy to build. It isn't made by or affiliated with Anthropic.

![The custom example: a pixel-art logo, a sent command, and a running one on the status bar](docs/images/custom.svg)

## Add it

Abalone isn't on crates.io yet. Add it from GitHub:

```toml
[dependencies]
abalone = { git = "https://github.com/Deaadman/abalone", tag = "1.0.0" }
```

## Your first app

Say what your commands are, and what to do when someone sends a line:

```rust
use abalone::{App, CommandInfo, Ctx, NoPanel, Shell, outln};

struct Hello;

impl App for Hello {
    type Msg = ();
    type Panel = NoPanel;

    fn commands(&self) -> Vec<CommandInfo> {
        vec![CommandInfo::new("greet", "Say hello")]
    }

    fn run(&mut self, line: &str, ctx: &mut Ctx<Self>) {
        ctx.start("Greeting", || outln!("Hello!"));
    }
}

fn main() -> anyhow::Result<()> {
    Shell::new(Hello).title("Hello").run()
}
```

That's it. You get the menu, `/help`, `/clear`, `/exit`, input history, and a spinner while `/greet` runs. Try the examples:

```sh
cargo run --example minimal
cargo run --example custom
```

## What's in it

| Part | What you get |
| --- | --- |
| The shell | A header and logo, the input box, the command menu, a status bar and a footer |
| Panels | `/help`, `/settings` and `/release-notes`, and panels of your own |
| Two ways to draw | Full screen, or in the terminal's own scrollback (`/tui`) |
| Logs | A log file per run for bug reports, and a clear message after a crash |
| Files | Where your app keeps its data, and its settings in a TOML file |
| Installing and updating | Install scripts for Windows, macOS and Linux, and updates from GitHub releases |

## Learn more

- [Getting started](docs/getting-started.md): commands, running work, printing, keys
- [Making it yours](docs/look-and-feel.md): colours, logo, spinner, header, status bar and footer
- [Panels](docs/panels.md): help, settings, release notes, your own, full screen or inline
- [Logs and files](docs/logs-and-files.md): crash reports, the data folder, settings files, pictures
- [Installing and updating](docs/install-and-update.md): install scripts, updates and releases
- [Working on Abalone](docs/development.md): building, testing and releasing it

## License

[MIT](LICENSE).
