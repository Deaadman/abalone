//! Where an app keeps its files, and its settings in a TOML file.
//!
//! | | Windows | macOS | Linux |
//! |---|---|---|---|
//! | [`default_root`] | `%LOCALAPPDATA%\<app>` | `~/Library/Application Support/<app>` | `~/.local/share/<app>` |
//! | [`data_dir`] | `…\<app>\data` | `…/<app>/data` | `…/<app>/data` |
//!
//! (`$XDG_DATA_HOME` moves Linux's.) The installed program lives in the root
//! too ([`crate::install`]), so the data stays apart from it.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// The app's own folder in the user's local data: see the table above.
pub fn default_root(app: &str) -> Result<PathBuf> {
    directories::BaseDirs::new()
        .map(|d| d.data_local_dir().join(app))
        .ok_or_else(|| anyhow!("cannot find a home directory"))
}

/// `MY_APP_DATA_DIR` for `my-app`.
pub fn data_dir_env(app: &str) -> String {
    format!("{}_DATA_DIR", env_prefix(app))
}

/// `explicit` (a `--data-dir` flag, say), else `$<APP>_DATA_DIR`, else
/// `data` in [`default_root`].
pub fn data_dir(app: &str, explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = explicit {
        return Ok(p.to_path_buf());
    }
    let env = data_dir_env(app);
    if let Some(p) = std::env::var_os(&env).filter(|p| !p.is_empty()) {
        return Ok(PathBuf::from(p));
    }
    default_root(app)
        .map(|r| r.join("data"))
        .map_err(|e| anyhow!("{e}; pass --data-dir or set {env}"))
}

/// `MY_APP` for `my-app`: the start of the app's environment variables.
/// Anything but ASCII letters and digits becomes `_`, so the name is safe
/// in a shell or PowerShell variable.
pub fn env_prefix(app: &str) -> String {
    app.to_uppercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

/// Opens `path` (a folder, say) in the system's file manager or its app:
/// Explorer on Windows, `open` on macOS, `xdg-open` elsewhere.
pub fn open(path: &Path) -> Result<()> {
    #[cfg(windows)]
    let (program, arg) = ("explorer", path.to_string_lossy().replace('/', "\\"));
    #[cfg(target_os = "macos")]
    let (program, arg) = ("open", path.to_string_lossy().into_owned());
    #[cfg(not(any(windows, target_os = "macos")))]
    let (program, arg) = ("xdg-open", path.to_string_lossy().into_owned());
    // Explorer opens Documents instead of a path with forward slashes, hence the replace.
    std::process::Command::new(program)
        .arg(arg)
        .spawn()
        .with_context(|| format!("couldn't open {} with {program}", path.display()))?;
    Ok(())
}

/// Settings from a TOML file; a missing file is the defaults.
pub fn load_toml<T: DeserializeOwned + Default>(path: &Path) -> Result<T> {
    if !path.is_file() {
        return Ok(T::default());
    }
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

/// Writes `value` to a TOML file, creating its folder. It's written to a
/// temporary file beside it and renamed over it, so a crash or a full disk
/// leaves the old file whole rather than half written.
pub fn save_toml<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let text = toml::to_string_pretty(value)?;
    let dir = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => Path::new("."),
    };
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    write_atomic(dir, path, text.as_bytes()).with_context(|| format!("writing {}", path.display()))
}

/// Writes `bytes` to a uniquely named file in `dir`, syncs it, and renames it to `path`.
fn write_atomic(dir: &Path, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let name = path.file_name().map_or_else(|| "file".into(), |n| n.to_string_lossy());
    let tmp = dir.join(format!(
        ".{name}.{}-{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let written = (|| {
        let mut f = std::fs::File::options().write(true).create_new(true).open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Default, PartialEq, Serialize, serde::Deserialize)]
    struct Settings {
        name: Option<String>,
        on: Option<bool>,
    }

    #[test]
    fn env_prefixes_are_safe_names() {
        assert_eq!(env_prefix("my-app"), "MY_APP");
        assert_eq!(env_prefix("bob's $app.v2"), "BOB_S__APP_V2");
    }

    #[test]
    fn settings_round_trip() {
        let path = std::env::temp_dir()
            .join(format!("abalone-files-{}", std::process::id()))
            .join("config.toml");
        assert_eq!(load_toml::<Settings>(&path).unwrap(), Settings::default());
        let s = Settings {
            name: Some("demo".into()),
            on: Some(true),
        };
        save_toml(&path, &s).unwrap();
        assert_eq!(load_toml::<Settings>(&path).unwrap(), s);
        // Saving again replaces the file and leaves no temporary file behind.
        let s = Settings {
            name: Some("again".into()),
            on: None,
        };
        save_toml(&path, &s).unwrap();
        assert_eq!(load_toml::<Settings>(&path).unwrap(), s);
        let names: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["config.toml"]);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn data_dir_prefers_what_it_is_given() {
        assert_eq!(data_dir("demo", Some(Path::new("here"))).unwrap(), Path::new("here"));
        assert_eq!(data_dir_env("my-app"), "MY_APP_DATA_DIR");
        assert!(data_dir("abalone-test-app", None).unwrap().ends_with("data"));
    }
}
