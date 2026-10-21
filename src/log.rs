//! One log file per run, for people to attach to a bug report: the version,
//! what ran, warnings, errors and crashes. The home folder and the user's
//! name are scrubbed from every line, and only the newest 20 logs are kept.
//! A log stops growing at 10 MB, past which only crashes are written.
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
/// Bytes a log grows to; past it a note says so and only crashes are written.
const MAX_BYTES: u64 = 10 * 1024 * 1024;

/// Text scrubbed from the log, and what it becomes.
struct Scrub {
    from: String,
    to: &'static str,
    /// Matched ignoring case, with `/` and `\` the same (a Windows path).
    fold: bool,
}

struct Log {
    file: File,
    path: PathBuf,
    started: Instant,
    /// Bytes written so far.
    size: u64,
    /// Longest first.
    scrub: Vec<Scrub>,
}

static LOG: Mutex<Option<Log>> = Mutex::new(None);

/// Opens this run's log in `dir`, named `<app>-<time>-<pid>.log`. `title` is
/// the app and version, `mode` what this run does (`shell`, or a subcommand).
/// Failing to create the file only means this run isn't logged.
pub fn start(dir: &Path, app: &str, title: &str, mode: &str) {
    if fs::create_dir_all(dir).is_err() {
        return;
    }
    prune(dir, app, KEEP - 1);
    let now = utc_now();
    let path = dir.join(format!("{app}-{}-{}.log", now.replace(':', "-"), std::process::id()));
    let Ok(file) = File::options().create_new(true).append(true).open(&path) else {
        return;
    };
    let log = Log {
        file,
        path,
        started: Instant::now(),
        size: 0,
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

/// One entry; continuation lines line up under its text. Past 10 MB only
/// `"PANIC"` entries are written.
pub fn write(level: &str, text: &str) {
    let Ok(mut slot) = LOG.lock() else {
        return;
    };
    let Some(log) = slot.as_mut() else {
        return;
    };
    if log.size >= MAX_BYTES && level != "PANIC" {
        return;
    }
    let text = scrub(text, &log.scrub);
    let secs = log.started.elapsed().as_secs_f64();
    let body = text.trim_end().replace('\n', &format!("\n{:19}", ""));
    let mut entry = format!("[{secs:>9.3}s] {level:<5} {body}\n");
    if level != "PANIC" && log.size + entry.len() as u64 >= MAX_BYTES {
        entry = format!("[{secs:>9.3}s] INFO  the log is full, so only crashes are written from here\n");
        log.size = MAX_BYTES;
    } else {
        log.size += entry.len() as u64;
    }
    let _ = log.file.write_all(entry.as_bytes());
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

/// Deletes all but the newest `keep` of `app`'s logs.
fn prune(dir: &Path, app: &str, keep: usize) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut logs: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| is_log(n, app)))
        .collect();
    // The names go on with a UTC timestamp, so they sort oldest first.
    logs.sort();
    let extra = logs.len().saturating_sub(keep);
    for p in &logs[..extra] {
        let _ = fs::remove_file(p);
    }
}

/// True for a name [`start`] gives `app`'s logs: `<app>-YYYY-MM-DDTHH-MM-SSZ-<pid>.log`.
fn is_log(name: &str, app: &str) -> bool {
    let Some(rest) = name
        .strip_prefix(app)
        .and_then(|r| r.strip_prefix('-'))
        .and_then(|r| r.strip_suffix(".log"))
    else {
        return false;
    };
    let shape = b"0000-00-00T00-00-00Z";
    let Some((time, pid)) = rest.split_at_checked(shape.len()) else {
        return false;
    };
    time.bytes()
        .zip(shape)
        .all(|(b, &s)| if s == b'0' { b.is_ascii_digit() } else { b == s })
        && pid
            .strip_prefix('-')
            .is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

fn scrub(text: &str, list: &[Scrub]) -> String {
    let mut text = text.to_owned();
    for s in list {
        text = replace_whole(&text, &s.from, s.to, s.fold);
    }
    text
}

/// Letters, digits and `_`: what a match mustn't run into.
fn word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `c` lowercased, and `/` as `\`.
fn folded(c: char) -> char {
    if c == '/' {
        return '\\';
    }
    let mut l = c.to_lowercase();
    match (l.next(), l.next()) {
        (Some(l), None) => l,
        _ => c,
    }
}

/// `text` with each `from` that isn't part of a longer word or name made `to`.
fn replace_whole(text: &str, from: &str, to: &str, fold: bool) -> String {
    let norm = |c: char| if fold { folded(c) } else { c };
    let pat: Vec<char> = from.chars().map(norm).collect();
    let (Some(&first), Some(&last)) = (pat.first(), pat.last()) else {
        return text.to_owned();
    };
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let end = i + pat.len();
        let hit = end <= chars.len()
            && chars[i..end].iter().zip(&pat).all(|(&c, &p)| norm(c) == p)
            && !(word(first) && i > 0 && word(chars[i - 1]))
            && !(word(last) && chars.get(end).is_some_and(|&c| word(c)));
        if hit {
            out.push_str(to);
            i = end;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// The home folder becomes `~` and the user name `<user>`, each only as a
/// whole name. On Windows the home folder matches in any case and either
/// slash style; elsewhere in both slash styles.
fn scrub_list() -> Vec<Scrub> {
    let mut list: Vec<Scrub> = Vec::new();
    let mut add = |from: String, to, fold| {
        if !list.iter().any(|s| s.from == from) {
            list.push(Scrub { from, to, fold });
        }
    };
    for var in ["USERPROFILE", "HOME"] {
        if let Some(home) = std::env::var_os(var).map(|h| h.to_string_lossy().into_owned())
            && home.len() > 3
        {
            let home = home.trim_end_matches(['/', '\\']);
            if cfg!(windows) {
                add(home.to_owned(), "~", true);
            } else {
                add(home.replace('/', "\\"), "~", false);
                add(home.replace('\\', "/"), "~", false);
            }
        }
    }
    for var in ["USERNAME", "USER", "LOGNAME"] {
        // A one- or two-letter name would scrub ordinary words.
        if let Ok(name) = std::env::var(var)
            && name.len() >= 3
        {
            add(name, "<user>", false);
        }
    }
    list.sort_by_key(|s| std::cmp::Reverse(s.from.len()));
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

    fn s(from: &str, to: &'static str, fold: bool) -> Scrub {
        Scrub {
            from: from.into(),
            to,
            fold,
        }
    }

    #[test]
    fn scrubs_home_and_user_name() {
        let list = vec![
            s(r"C:\Users\pat", "~", false),
            s("C:/Users/pat", "~", false),
            s("pat", "<user>", false),
        ];
        assert_eq!(
            scrub(r"can't open C:\Users\pat\Videos and C:/Users/pat/x (pat)", &list),
            r"can't open ~\Videos and ~/x (<user>)"
        );
    }

    #[test]
    fn scrubs_only_whole_names() {
        let list = vec![s("/home/admin", "~", false), s("admin", "<user>", false)];
        assert_eq!(
            scrub("admin ran /home/admin/x as administrator, not /home/admin2", &list),
            "<user> ran ~/x as administrator, not /home/admin2"
        );
    }

    #[test]
    fn scrubs_a_windows_home_in_any_case_and_slash() {
        let list = vec![s(r"C:\Users\Pat", "~", true)];
        assert_eq!(
            scrub(r"c:\users\pat\a, C:/USERS/PAT/b and C:\Users\Patrick", &list),
            r"~\a, ~/b and C:\Users\Patrick"
        );
    }

    #[test]
    fn knows_its_own_log_names() {
        assert!(is_log("foo-2026-10-01T00-00-00Z-12.log", "foo"));
        assert!(!is_log("foo-beta-2026-10-01T00-00-00Z-12.log", "foo"));
        assert!(!is_log("foo-2026-10-01T00-00-00Z-.log", "foo"));
        assert!(!is_log("foo-2026-10-01T00-00-00Z-12.txt", "foo"));
        assert!(!is_log("foo-notes.log", "foo"));
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
        File::create(dir.join("demo-beta-2026-10-01T00-00-00Z-1.log")).unwrap();
        File::create(dir.join("notes.txt")).unwrap();
        prune(&dir, "demo", 2);
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
                "demo-beta-2026-10-01T00-00-00Z-1.log",
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
