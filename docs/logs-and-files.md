# Logs and files

## A log for bug reports

When something goes wrong, people can send you the log from that run. `log::start` opens a new log file each time your app runs and keeps the newest 20. The first line says which version and system it is.

`log::guard` wraps your `main`. It writes every warning and error your app prints to the log, and any crash with where it happened. A plain error prints as `error: …` and quits. After a crash it tells people which log file to send you:

```
My App crashed. Please send this file with your bug report:
  C:\Users\…\my-app\data\logs\my-app-2026-10-21T09-14-03Z-4120.log
```

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

Logs are safe to share: the home folder becomes `~` and the user's name becomes `<user>` on every line. Write what's worth knowing with `log::write("CMD", "scan")`, but keep people's own data, like what they typed, out of it.

`log::hint()` is " · details in <the log file>", to add to a message asking for a report. `App::crashed` is a good place for it.

`term::interactive()` is true when the app runs in a terminal and not with its output piped somewhere, which is when to open the shell.

## The data folder

`files::data_dir("my-app", flag)` is where your app keeps its files:

1. The folder given on the command line (`--data-dir`, say), if any
2. Else `$MY_APP_DATA_DIR`, if set
3. Else `data` in the app's own folder:

| System | The app's folder |
| --- | --- |
| Windows | `%LOCALAPPDATA%\my-app` |
| macOS | `~/Library/Application Support/my-app` |
| Linux | `~/.local/share/my-app` |

`files::open(&folder)` opens a folder in Explorer, Finder or the Linux file manager, for a "show me my logs" command.

## Settings in a file

Keep your settings in a struct, and read and write it as TOML:

```rust
#[derive(Default, Serialize, Deserialize)]
struct Config {
    name: Option<String>,
    auto_update: Option<bool>,
}

let path = data_dir.join("config.toml");
let mut config: Config = files::load_toml(&path)?; // a missing file gives the defaults
config.name = Some("Sam".into());
files::save_toml(&path, &config)?;
```

These need the `files` feature.

## Pictures

`image::Image` draws a picture in the terminal. Each character is one pixel wide and two tall, so a square picture stays square.

```rust
let img = Image::from_rgba(width, height, &rgba_bytes);
let rows = img.resize(40, 40).rows(); // 20 rows of 40 characters
```

`turn(n)` turns it a quarter turn clockwise `n` times. `cells(colour)` lets you change each pixel's colour, or hide it, before drawing. Each row has one piece per column, so you can put marks on it. `image::Cache` keeps the resized picture, so a panel that redraws often only resizes it when the size changes.
