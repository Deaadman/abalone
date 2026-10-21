//! One log file per run, for people to attach to a bug report: the version,
//! what ran, warnings, errors and crashes. The home folder and the user's
//! name are scrubbed from every line, and only the newest 20 logs are kept.
//!
//! [`guard`] wraps `main`: it starts logging output's warnings and errors,
//! records crashes with a backtrace, and asks for the log when one happens:
//!
//! ```no_run
//! let data_dir = std::path::PathBuf::from("data");
//! abalone::log::start(&data_dir.join("logs"), "my-app", "My App 1.0.0", "shell");
//! abalone::log::guard("My App", || {
//!     // the app
//!     Ok(())
//! });
//! ```
//!
//! Write what's worth knowing with [`write()`] (`"CMD"`, `"INFO"`…); keep
//! people's own data, like command arguments, out of it.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use crate::out::{self, Kind};

/// Logs kept, counting this run's; older ones are deleted at startup.
const KEEP: usize = 20;

struct Log {
    file: File,
    path: PathBuf,
    started: Instant,
    /// `(text, replacement)` pairs, longest first.
    scrub: Vec<(String, &'static str)>,
}

static LOG: Mutex<Option<Log>> = Mutex::new(None);

/// Opens this run's log in `dir`, named `<app>-<time>-<pid>.log`. `title` is
/// the app and version, `mode` what this run does (`shell`, or a subcommand).
/// Failing to create the file only means this run isn't logged.
pub fn start(dir: &Path, app: &str, title: &str, mode: &str) {
    if fs::create_dir_all(dir).is_err() {
        return;
    }
    let prefix = format!("{app}-");
    prune(dir, &prefix, KEEP - 1);
    let now = utc_now();
    let path = dir.join(format!("{prefix}{}-{}.log", now.replace(':', "-"), std::process::id()));
    let Ok(file) = File::options().create_new(true).append(true).open(&path) else {
        return;
    };
    let log = Log {
        file,
        path,
        started: Instant::now(),
        scrub: scrub_list(),
    };
    if let Ok(mut slot) = LOG.lock() {
        *slot = Some(log);
    }
    write(
        "INFO",
        &format!(
            "{title} ({} {}) · {mode} · started {now}",
            std::env::consts::OS,
            std::env::consts::ARCH,
        ),
    );
}

/// This run's log file, if one is open.
pub fn path() -> Option<PathBuf> {
    LOG.lock().ok()?.as_ref().map(|l| l.path.clone())
}

/// One entry; continuation lines line up under its text.
pub fn write(level: &str, text: &str) {
    let Ok(mut slot) = LOG.lock() else {
        return;
    };
    let Some(log) = slot.as_mut() else {
        return;
    };
    let text = scrub(text, &log.scrub);
    let secs = log.started.elapsed().as_secs_f64();
    let body = text.trim_end().replace('\n', &format!("\n{:19}", ""));
    let _ = writeln!(log.file, "[{secs:>9.3}s] {level:<5} {body}");
}

/// " · details in `<log>`", for messages that ask people to report something.
pub fn hint() -> String {
    path().map_or_else(String::new, |p| format!(" · details in {}", p.display()))
}

/// Logs every warning and error printed through [`crate::out`] from now on.
pub fn log_output() {
    out::set_log(Some(|kind, text| {
        write(if kind == Kind::Error { "ERROR" } else { "WARN" }, text)
    }));
}

/// Logs every panic (with a backtrace) before the default hook runs. In the
/// shell the default hook's message would land on the alternate screen and
/// vanish, so it only runs outside it.
pub fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let backtrace = std::backtrace::Backtrace::force_capture();
        write(
            "PANIC",
            &format!("thread '{}' {info}\n{backtrace}", thread.name().unwrap_or("worker")),
        );
        if !out::captured() {
            default(info);
        }
    }));
}

/// Runs `main` with [`log_output`] and [`install_panic_hook`]. An error is
/// printed as `error: …` and exits with 1; a crash (the shell has restored
/// the terminal by then) asks for the log and exits with 101.
pub fn guard(name: &str, main: impl FnOnce() -> anyhow::Result<()>) {
    install_panic_hook();
    log_output();
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(main)) {
        Ok(Ok(())) => write("INFO", "exited"),
        Ok(Err(e)) => {
            out::emit(Kind::Error, format!("error: {e:#}"));
            std::process::exit(1);
        }
        Err(_) => {
            match path() {
                Some(p) => eprintln!(
                    "{name} crashed. Please send this file with your bug report:\n  {}",
                    p.display()
                ),
                None => eprintln!("{name} crashed."),
            }
            std::process::exit(101);
        }
    }
}

/// Deletes all but the newest `keep` logs starting with `prefix`.
fn prune(dir: &Path, prefix: &str, keep: usize) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut logs: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension().is_some_and(|x| x == "log")
                && p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(prefix))
        })
        .collect();
    // The names go on with a UTC timestamp, so they sort oldest first.
    logs.sort();
    let extra = logs.len().saturating_sub(keep);
    for p in &logs[..extra] {
        let _ = fs::remove_file(p);
    }
}

fn scrub(text: &str, list: &[(String, &'static str)]) -> String {
    let mut text = text.to_owned();
    for (from, to) in list {
        text = text.replace(from.as_str(), to);
    }
    text
}

/// The home folder becomes `~` and the user name `<user>`, in both slash styles.
fn scrub_list() -> Vec<(String, &'static str)> {
    let mut list = Vec::new();
    for var in ["USERPROFILE", "HOME"] {
        if let Some(home) = std::env::var_os(var).map(|h| h.to_string_lossy().into_owned())
            && home.len() > 3
        {
            list.push((home.replace('/', "\\"), "~"));
            list.push((home.replace('\\', "/"), "~"));
        }
    }
    for var in ["USERNAME", "USER", "LOGNAME"] {
        // A one- or two-letter name would scrub ordinary words.
        if let Ok(name) = std::env::var(var)
            && name.len() >= 3
        {
            list.push((name, "<user>"));
        }
    }
    list.sort_by_key(|(from, _)| std::cmp::Reverse(from.len()));
    list.dedup();
    list
}

/// Now as `YYYY-MM-DDTHH:MM:SSZ`.
fn utc_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    utc(secs)
}

/// Unix seconds as `YYYY-MM-DDTHH:MM:SSZ` (days-to-civil, Howard Hinnant).
fn utc(secs: i64) -> String {
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrubs_home_and_user_name() {
        let list = vec![
            (r"C:\Users\pat".to_owned(), "~"),
            ("C:/Users/pat".to_owned(), "~"),
            ("pat".to_owned(), "<user>"),
        ];
        assert_eq!(
            scrub(r"can't open C:\Users\pat\Videos and C:/Users/pat/x (pat)", &list),
            r"can't open ~\Videos and ~/x (<user>)"
        );
    }

    #[test]
    fn prune_keeps_the_newest() {
        let dir = std::env::temp_dir().join(format!("abalone-prune-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for day in 1..=5 {
            File::create(dir.join(format!("demo-2026-10-0{day}T00-00-00Z-1.log"))).unwrap();
        }
        File::create(dir.join("other-2026-10-01T00-00-00Z-1.log")).unwrap();
        File::create(dir.join("notes.txt")).unwrap();
        prune(&dir, "demo-", 2);
        let mut left: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(
            left,
            [
                "demo-2026-10-04T00-00-00Z-1.log",
                "demo-2026-10-05T00-00-00Z-1.log",
                "notes.txt",
                "other-2026-10-01T00-00-00Z-1.log",
            ]
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn formats_utc() {
        assert_eq!(utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc(1_792_540_800 + 3_661), "2026-10-21T01:01:01Z");
    }
}
