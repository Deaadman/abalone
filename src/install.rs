//! An installed copy of the app in [`files::default_root`] (or `$<APP>_HOME`):
//! `%LOCALAPPDATA%\<app>` on Windows, `~/.local/share/<app>` on Linux and
//! `~/Library/Application Support/<app>` on macOS.
//!
//! - `versions/<version>/<app>[.exe]`: every installed version.
//! - `bin/<app>[.exe]`: a copy of the current one; `bin` is on PATH.
//!
//! A version installs itself (`<app> install`): it copies itself into
//! `versions` and swaps the copy in `bin`. Windows lets a running exe be
//! renamed but not overwritten, so the old launcher is renamed aside first
//! and deleted on a later install (elsewhere the rename is just harmless).
//! A running copy keeps working; the next launch starts the new version.
//!
//! [`install`] does all of it. Apps with more to fetch (a helper tool, say)
//! call the steps themselves: [`copy_in`], then their own, then
//! [`place_launcher`] and [`cleanup`].
//!
//! [`InstallScript`] writes the scripts people run to get the app in the
//! first place: `install.ps1` on Windows (`irm https://…/install.ps1 | iex`)
//! and `install.sh` on macOS and Linux (`curl -fsSL https://…/install.sh | bash`).
//!
//! Release files are named by [`platform`]: `windows-x64`, `linux-arm64`,
//! `macos-arm64` and so on.

use std::cmp::Ordering;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use sha2::{Digest, Sha256};

use crate::files;

/// Installed versions kept besides the current one.
const KEEP_OLDER: usize = 1;
/// Larger than any release asset; a guard against a runaway response.
const MAX_DOWNLOAD: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct Layout {
    pub root: PathBuf,
    /// The exe's name without `.exe`: `vantage`.
    pub app: String,
}

/// This build's platform, as release files and manifests name it:
/// `windows-x64`, `linux-arm64`, `macos-arm64`…
pub fn platform() -> String {
    let os = std::env::consts::OS;
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        "x86" => "x86",
        other => other,
    };
    format!("{os}-{arch}")
}

/// What runs on `platform` when a release has no build for it: x64 builds
/// run on Arm Windows (emulated) and Arm Macs (Rosetta).
pub fn fallback_platform(platform: &str) -> Option<&'static str> {
    match platform {
        "windows-arm64" => Some("windows-x64"),
        "macos-arm64" => Some("macos-x64"),
        _ => None,
    }
}

/// `my-app-linux-x64`, `my-app-windows-x64.exe`: a release's file for `platform`.
pub fn asset_name(app: &str, platform: &str) -> String {
    let exe = if platform.starts_with("windows") { ".exe" } else { "" };
    format!("{app}-{platform}{exe}")
}

impl Layout {
    /// `$<APP>_HOME`, else [`files::default_root`].
    pub fn new(app: &str) -> Result<Self> {
        let env = home_env(app);
        let root = match std::env::var_os(&env).filter(|p| !p.is_empty()) {
            Some(p) => PathBuf::from(p),
            None => files::default_root(app).map_err(|e| anyhow!("{e}; set {env}"))?,
        };
        Ok(Layout::at(root, app))
    }

    pub fn at(root: impl Into<PathBuf>, app: &str) -> Self {
        Layout {
            root: root.into(),
            app: app.to_owned(),
        }
    }

    /// `vantage.exe` (just `vantage` off Windows).
    pub fn exe_name(&self) -> String {
        format!("{}{}", self.app, std::env::consts::EXE_SUFFIX)
    }

    pub fn bin(&self) -> PathBuf {
        self.root.join("bin")
    }

    /// `bin/<app>[.exe]`, the one on PATH.
    pub fn launcher(&self) -> PathBuf {
        self.bin().join(self.exe_name())
    }

    pub fn versions(&self) -> PathBuf {
        self.root.join("versions")
    }

    pub fn version_exe(&self, version: &str) -> PathBuf {
        self.versions().join(version).join(self.exe_name())
    }

    /// True when `exe` is the launcher or an installed version, i.e. this
    /// copy was installed rather than built or copied somewhere else.
    pub fn contains(&self, exe: &Path) -> bool {
        let (Ok(exe), Ok(root)) = (fs::canonicalize(exe), fs::canonicalize(&self.root)) else {
            return false;
        };
        exe.starts_with(root.join("bin")) || exe.starts_with(root.join("versions"))
    }

    /// True when the running exe is in this layout.
    pub fn runs_this(&self) -> bool {
        std::env::current_exe().is_ok_and(|e| self.contains(&e))
    }

    /// Whether `bin` is on this process's PATH.
    pub fn on_path(&self) -> bool {
        std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d == self.bin()))
    }
}

/// `VANTAGE_HOME` for `vantage`.
pub fn home_env(app: &str) -> String {
    format!("{}_HOME", env_prefix(app))
}

pub use crate::files::env_prefix;

/// Installs `exe` (the running binary, version `version`): copies it into
/// `versions`, points `bin` at it and removes old versions.
pub fn install(layout: &Layout, version: &str, exe: &Path) -> Result<PathBuf> {
    let target = copy_in(layout, version, exe)?;
    place_launcher(layout, &target)?;
    cleanup(layout, version);
    Ok(target)
}

/// What `<app> install` says when it's done: where it went, and how to
/// put it on PATH when it isn't (the install scripts do that).
pub fn report(layout: &Layout, name: &str, version: &str) {
    crate::outln!("installed {name} v{version} to {}", layout.launcher().display());
    if !layout.on_path() {
        crate::outln!(
            "add {} to PATH to run `{}` from any terminal",
            layout.bin().display(),
            layout.app
        );
    }
}

/// Copies `exe` to `versions/<version>` unless it's already there. Returns the copy.
pub fn copy_in(layout: &Layout, version: &str, exe: &Path) -> Result<PathBuf> {
    let target = layout.version_exe(version);
    if !same_file(exe, &target) {
        copy_verified(exe, &target)?;
    }
    Ok(target)
}

/// Makes `bin/<app>[.exe]` a copy of `src`. A launcher that is running
/// cannot be overwritten, so it is renamed aside first.
pub fn place_launcher(layout: &Layout, src: &Path) -> Result<()> {
    let launcher = layout.launcher();
    if launcher.is_file() && sha256_file(&launcher)? == sha256_file(src)? {
        return Ok(());
    }
    fs::create_dir_all(layout.bin()).with_context(|| format!("creating {}", layout.bin().display()))?;
    let name = layout.exe_name();
    let staged = layout.bin().join(format!("{name}.new"));
    fs::copy(src, &staged).with_context(|| format!("copying {} to {}", src.display(), staged.display()))?;
    let mut aside = None;
    if launcher.exists() {
        let to = layout.bin().join(format!("{name}.old-{}", unique_suffix()));
        if let Err(e) = fs::rename(&launcher, &to) {
            let _ = fs::remove_file(&staged);
            return Err(e).with_context(|| format!("moving the old {} aside", launcher.display()));
        }
        aside = Some(to);
    }
    if let Err(e) = fs::rename(&staged, &launcher) {
        // Put the old launcher back, so PATH still finds one.
        if let Some(aside) = &aside {
            let _ = fs::rename(aside, &launcher);
        }
        let _ = fs::remove_file(&staged);
        return Err(e).with_context(|| format!("placing {}", launcher.display()));
    }
    Ok(())
}

/// Best effort: deletes launchers renamed aside (those still running stay
/// locked and are retried next time), and all but the newest older version.
pub fn cleanup(layout: &Layout, current: &str) {
    let aside = format!("{}.old-", layout.exe_name());
    if let Ok(entries) = fs::read_dir(layout.bin()) {
        for e in entries.flatten() {
            if e.file_name().to_string_lossy().starts_with(&aside) {
                let _ = fs::remove_file(e.path());
            }
        }
    }
    prune(&layout.versions(), current);
}

/// Removes subdirectories of `dir` except `keep` and the newest other
/// version. A directory whose exe is running cannot be removed and is left
/// for next time.
pub fn prune(dir: &Path, keep: &str) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut others: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter(|e| e.path().is_dir() && e.file_name() != keep)
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .collect();
    others.sort_by(|a, b| compare_versions(&a.0, &b.0).then_with(|| a.0.cmp(&b.0)));
    let n = others.len().saturating_sub(KEEP_OLDER);
    for (_, path) in others.into_iter().take(n) {
        let _ = fs::remove_dir_all(path);
    }
}

/// `v1.2.3` or `1.2.3-beta` as numbers; non-numeric parts are 0. It drops
/// the pre-release, so order versions with [`compare_versions`].
pub fn version_key(v: &str) -> Vec<u64> {
    v.trim_start_matches('v')
        .split(['-', '+'])
        .next()
        .unwrap_or("")
        .split('.')
        .map(|p| p.parse().unwrap_or(0))
        .collect()
}

/// Orders versions as SemVer does: `1.0.0-beta.2` < `1.0.0-rc.1` < `1.0.0`.
/// A leading `v` and build metadata (`+…`) are ignored, and missing
/// numbers count as 0 (`1.2` is `1.2.0`).
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    let (a_core, a_pre) = split_version(a);
    let (b_core, b_pre) = split_version(b);
    let (a_key, b_key) = (version_key(a_core), version_key(b_core));
    let len = a_key.len().max(b_key.len());
    let number = |key: &[u64], i: usize| key.get(i).copied().unwrap_or(0);
    let core = (0..len)
        .map(|i| number(&a_key, i).cmp(&number(&b_key, i)))
        .find(|o| o.is_ne())
        .unwrap_or(Ordering::Equal);
    core.then_with(|| match (a_pre, b_pre) {
        (None, None) => Ordering::Equal,
        // A pre-release comes before its version.
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (Some(a), Some(b)) => compare_pre_release(a, b),
    })
}

/// `v1.2.3-rc.1+build` as `1.2.3` and `rc.1`.
fn split_version(v: &str) -> (&str, Option<&str>) {
    let v = v.trim_start_matches('v');
    let v = v.split_once('+').map_or(v, |(v, _)| v);
    match v.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (v, None),
    }
}

/// Dot-separated parts in turn: numbers by value and before words, words
/// by ASCII order; when one runs out first, it's the lower.
fn compare_pre_release(a: &str, b: &str) -> Ordering {
    let mut a = a.split('.');
    let mut b = b.split('.');
    loop {
        let o = match (a.next(), b.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) => match (numeric(x), numeric(y)) {
                (Some(x), Some(y)) => x.len().cmp(&y.len()).then_with(|| x.cmp(y)),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => x.cmp(y),
            },
        };
        if o.is_ne() {
            return o;
        }
    }
}

/// A number part without its leading zeros, which then orders by length
/// and then by digits, however long it is.
fn numeric(part: &str) -> Option<&str> {
    (!part.is_empty() && part.bytes().all(|b| b.is_ascii_digit())).then(|| part.trim_start_matches('0'))
}

/// True when `candidate` is a higher version than `current` ([`compare_versions`]).
pub fn is_newer(candidate: &str, current: &str) -> bool {
    compare_versions(candidate, current) == Ordering::Greater
}

fn same_file(a: &Path, b: &Path) -> bool {
    matches!((fs::canonicalize(a), fs::canonicalize(b)), (Ok(a), Ok(b)) if a == b)
}

/// Copies `src` to `dst` through a temporary name, so `dst` is never half written.
fn copy_verified(src: &Path, dst: &Path) -> Result<()> {
    if dst.is_file() && sha256_file(dst)? == sha256_file(src)? {
        return Ok(());
    }
    let dir = dst.parent().expect("versioned path has a parent");
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let partial = dst.with_extension("partial");
    fs::copy(src, &partial).with_context(|| format!("copying {} to {}", src.display(), partial.display()))?;
    fs::rename(&partial, dst).with_context(|| format!("placing {}", dst.display()))
}

/// A file's SHA-256 in lower-case hex.
pub fn sha256_file(path: &Path) -> Result<String> {
    let mut f = fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f
            .read(&mut buf)
            .with_context(|| format!("reading {}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Fails, and deletes the file, unless its SHA-256 is `expected`.
pub fn check_sha256(path: &Path, expected: &str, what: &str) -> Result<()> {
    let actual = sha256_file(path)?;
    if !actual.eq_ignore_ascii_case(expected.trim()) {
        let _ = fs::remove_file(path);
        bail!("{what} has SHA-256 {actual}, expected {expected}; not installed");
    }
    Ok(())
}

/// GET `url`, saying who's asking in the User-Agent (`my-app/1.2.0`).
///
/// Connecting and the response's headers each have a short limit; the
/// body has an hour, so a slow link still finishes a large download and
/// only a stalled one gives up (ureq has no limit per read).
pub fn http_get(url: &str, agent: &str) -> Result<ureq::http::Response<ureq::Body>> {
    ureq::get(url)
        .header("User-Agent", agent)
        .config()
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_send_request(Some(Duration::from_secs(30)))
        .timeout_recv_response(Some(Duration::from_secs(60)))
        .timeout_recv_body(Some(Duration::from_secs(60 * 60)))
        .build()
        .call()
        .with_context(|| format!("GET {url}"))
}

/// Downloads `url` to `dest` (through a temporary name).
pub fn download(url: &str, dest: &Path, agent: &str) -> Result<()> {
    download_to(url, dest, agent, None)
}

/// [`download`], but `dest` is only replaced when the file's SHA-256 is
/// `sha256`; `what` names it in the error.
pub fn download_verified(url: &str, dest: &Path, agent: &str, sha256: &str, what: &str) -> Result<()> {
    download_to(url, dest, agent, Some((sha256, what)))
}

/// Writes to a name of its own beside `dest`, so two downloads at once don't
/// mix, checks it, then renames it into place. A failed one is deleted.
fn download_to(url: &str, dest: &Path, agent: &str, sha256: Option<(&str, &str)>) -> Result<()> {
    let name = dest.file_name().map_or("download".into(), |n| n.to_string_lossy());
    let partial = dest.with_file_name(format!("{name}.{}.download", unique_suffix()));
    let result = (|| {
        let mut resp = http_get(url, agent)?;
        let mut file = fs::File::create(&partial).with_context(|| format!("creating {}", partial.display()))?;
        let mut reader = resp.body_mut().with_config().limit(MAX_DOWNLOAD).reader();
        let mut buf = vec![0u8; 1 << 16];
        loop {
            let n = reader.read(&mut buf).with_context(|| format!("downloading {url}"))?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])
                .with_context(|| format!("writing {}", partial.display()))?;
        }
        file.sync_all()?;
        drop(file);
        if let Some((sha256, what)) = sha256 {
            check_sha256(&partial, sha256, what)?;
        }
        fs::rename(&partial, dest).with_context(|| format!("placing {}", dest.display()))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&partial);
    }
    result
}

fn unique_suffix() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("{nanos}-{}", std::process::id())
}

/// Lets `path` run on macOS and Linux (a downloaded file can't until it's
/// marked executable); nothing on Windows.
pub fn make_executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))
            .with_context(|| format!("making {} executable", path.display()))?;
    }
    let _ = path;
    Ok(())
}

/// The install scripts for an app: [`render`](Self::render) is
/// `install.ps1` for Windows, [`render_sh`](Self::render_sh) is `install.sh`
/// for macOS and Linux. Each downloads the latest release's build for the
/// machine, checks it against the release's `manifest.json`, puts `bin` on
/// the user's PATH and runs `<app> install`.
///
/// Keep the scripts in the repo and test that they're current, so a change
/// here shows up as a failing test rather than a stale installer:
///
/// ```
/// let script = abalone::install::InstallScript::new("My App", "my-app", "me/my-app");
/// assert!(script.render().contains("$Repo = 'me/my-app'"));
/// assert!(script.render_sh().contains("REPO='me/my-app'"));
/// ```
#[derive(Debug, Clone)]
pub struct InstallScript {
    /// The name people see: `Vantage`.
    pub name: String,
    /// The exe without `.exe`: `vantage`.
    pub app: String,
    /// `owner/repo` on GitHub.
    pub repo: String,
    /// More for the comment at the top, a line each.
    pub about: Vec<String>,
    /// PowerShell run after `<app> install` succeeds, indented 4 spaces.
    pub after: String,
    /// Shell run after `<app> install` succeeds (in `install.sh`).
    pub after_sh: String,
    /// The last thing it says.
    pub done: String,
}

impl InstallScript {
    pub fn new(name: &str, app: &str, repo: &str) -> Self {
        InstallScript {
            name: name.to_owned(),
            app: app.to_owned(),
            repo: repo.to_owned(),
            about: vec![],
            after: String::new(),
            after_sh: String::new(),
            done: format!("Run `{app}` to start. Open a new terminal first if it is not found."),
        }
    }

    /// Add `lines` to the comment at the top.
    pub fn about(mut self, lines: &str) -> Self {
        self.about.extend(lines.lines().map(str::to_owned));
        self
    }

    pub fn after(mut self, powershell: &str) -> Self {
        self.after = powershell.to_owned();
        self
    }

    pub fn after_sh(mut self, sh: &str) -> Self {
        self.after_sh = sh.to_owned();
        self
    }

    pub fn done(mut self, text: &str) -> Self {
        self.done = text.to_owned();
        self
    }

    /// `install.ps1`, for Windows.
    pub fn render(&self) -> String {
        self.fill(include_str!("../templates/install.ps1"), &self.after, ps_quote)
    }

    /// `install.sh`, for macOS and Linux.
    pub fn render_sh(&self) -> String {
        self.fill(include_str!("../templates/install.sh"), &self.after_sh, sh_quote)
    }

    /// The templates use `'{{NAME}}'` and the like as quoted strings, and
    /// bare `{{NAME}}` only in comments.
    fn fill(&self, template: &str, after: &str, quote: fn(&str) -> String) -> String {
        let after = if after.trim().is_empty() {
            "\n".to_owned()
        } else {
            format!("\n{}\n\n", after.trim_start_matches(['\r', '\n']).trim_end())
        };
        let about: String = if self.about.is_empty() {
            String::new()
        } else {
            std::iter::once("#".to_owned())
                .chain(self.about.iter().map(|l| format!("# {l}").trim_end().to_owned()))
                .map(|l| l + "\n")
                .collect()
        };
        // A line break would end a comment early.
        let comment = |s: &str| s.replace(['\r', '\n'], " ");
        // A checkout with CRLF endings still fills the template.
        template
            .replace("\r\n", "\n")
            .replace("{{ABOUT}}", &about)
            .replace("'{{NAME}}'", &quote(&self.name))
            .replace("'{{APP}}'", &quote(&self.app))
            .replace("'{{REPO}}'", &quote(&self.repo))
            .replace("'{{DONE}}'", &quote(&self.done))
            .replace("{{NAME}}", &comment(&self.name))
            .replace("{{APP}}", &comment(&self.app))
            .replace("{{ENV}}", &env_prefix(&self.app))
            .replace("{{REPO}}", &comment(&self.repo))
            .replace("{{AFTER}}\n", &after)
    }
}

/// `s` as a PowerShell string taken literally: `'Bob''s'`. PowerShell
/// also ends a quoted string at a curly single quote, so those double too.
fn ps_quote(s: &str) -> String {
    let mut out = String::from("'");
    for c in s.chars() {
        if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}') {
            out.push(c);
        }
        out.push(c);
    }
    out.push('\'');
    out
}

/// `s` as a shell string taken literally: `'Bob'\''s'`.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("abalone-install-{name}-{}", unique_suffix()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn versions_order_numerically() {
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("v1.0.0", "0.99.0"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0-beta", "0.1.0"));
        assert!(is_newer("0.1.1", "0.1"));
    }

    #[test]
    fn pre_releases_order_as_semver_does() {
        let order = [
            "1.0.0-alpha",
            "1.0.0-alpha.1",
            "1.0.0-alpha.beta",
            "1.0.0-beta",
            "1.0.0-beta.2",
            "1.0.0-beta.11",
            "1.0.0-rc.1",
            "1.0.0",
            "1.1.0-beta.1",
            "1.1.0-beta.2",
            "1.1.0",
        ];
        for pair in order.windows(2) {
            assert!(is_newer(pair[1], pair[0]), "{} > {}", pair[1], pair[0]);
            assert!(!is_newer(pair[0], pair[1]), "{} < {}", pair[0], pair[1]);
        }
        assert_eq!(compare_versions("v1.2.0+build.5", "1.2.0"), Ordering::Equal);
        assert_eq!(compare_versions("1.2", "1.2.0"), Ordering::Equal);
        assert!(is_newer("1.0.0-rc.1+b", "1.0.0-beta+c"));
    }

    #[test]
    fn prunes_all_but_the_newest_older_version() {
        let root = temp("prune");
        for v in ["1.0.0", "1.1.0-beta.1", "1.1.0-beta.2", "1.1.0", "1.2.0"] {
            fs::create_dir_all(root.join(v)).unwrap();
        }
        prune(&root, "1.2.0");
        let mut left: Vec<String> = fs::read_dir(&root)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["1.1.0", "1.2.0"]);
        fs::remove_dir_all(root).unwrap();
    }

    /// Serves `body` to one request on a local port; returns its URL.
    fn serve_once(body: &'static [u8]) -> String {
        use std::io::BufRead;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/file", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = std::io::BufReader::new(stream);
            let mut line = String::new();
            // The request, up to its blank line.
            while reader.read_line(&mut line).is_ok_and(|n| n > 2) {
                line.clear();
            }
            let mut stream = reader.into_inner();
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(body);
        });
        url
    }

    #[test]
    fn a_download_is_placed_only_when_its_hash_matches() {
        let root = temp("download");
        let dest = root.join("demo");
        let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        let err = download_verified(&serve_once(b"abc"), &dest, "test", "00", "demo").unwrap_err();
        assert!(format!("{err:#}").contains("SHA-256"), "{err:#}");
        assert!(!dest.exists());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0, "the partial file is deleted");
        download_verified(&serve_once(b"abc"), &dest, "test", abc, "demo").unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"abc");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn names_its_environment() {
        assert_eq!(home_env("vantage"), "VANTAGE_HOME");
        assert_eq!(env_prefix("my-app"), "MY_APP");
    }

    #[test]
    fn swaps_the_launcher_and_prunes_old_versions() {
        let root = temp("swap");
        let layout = Layout::at(&root, "demo");
        for v in ["0.1.0", "0.2.0", "0.3.0"] {
            let exe = layout.version_exe(v);
            fs::create_dir_all(exe.parent().unwrap()).unwrap();
            fs::write(&exe, v).unwrap();
        }
        place_launcher(&layout, &layout.version_exe("0.2.0")).unwrap();
        place_launcher(&layout, &layout.version_exe("0.3.0")).unwrap();
        assert_eq!(fs::read_to_string(layout.launcher()).unwrap(), "0.3.0");
        // The replaced launcher was moved aside, as Windows needs when it runs.
        let aside = |l: &Layout| {
            let prefix = format!("{}.old-", l.exe_name());
            fs::read_dir(l.bin())
                .unwrap()
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
                .count()
        };
        assert_eq!(aside(&layout), 1);

        cleanup(&layout, "0.3.0");
        assert_eq!(aside(&layout), 0);
        assert!(layout.version_exe("0.3.0").is_file());
        assert!(layout.version_exe("0.2.0").is_file());
        assert!(!layout.version_exe("0.1.0").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn installing_is_idempotent() {
        let root = temp("self");
        let layout = Layout::at(&root, "demo");
        let src = root.join("built.exe");
        fs::write(&src, b"demo").unwrap();
        install(&layout, "9.9.9", &src).unwrap();
        install(&layout, "9.9.9", &src).unwrap();
        assert_eq!(fs::read(layout.launcher()).unwrap(), b"demo");
        assert!(layout.contains(&layout.launcher()));
        assert!(!layout.contains(&src));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn hashes_files() {
        let root = temp("hash");
        let f = root.join("abc");
        fs::write(&f, b"abc").unwrap();
        let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!(sha256_file(&f).unwrap(), abc);
        assert!(check_sha256(&f, abc, "abc").is_ok());
        assert!(check_sha256(&f, "00", "abc").is_err());
        assert!(!f.exists(), "a file that fails the check is deleted");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_install_script_names_the_app() {
        let s = InstallScript::new("Demo App", "demo", "me/demo")
            .after("    Write-Host '  extra step'")
            .render();
        assert!(s.contains("$Repo = 'me/demo'"));
        assert!(s.contains("$env:DEMO_HOME"));
        assert!(s.contains("$Name = 'Demo App'"));
        assert!(s.contains("& $download install"));
        assert!(s.contains("    Write-Host '  extra step'\n\n    Write-Host ''"));
        assert!(!s.contains("{{ABOUT}}"));
        let about = InstallScript::new("Demo", "demo", "me/demo")
            .about("It also fetches a helper.")
            .render();
        assert!(about.contains("# that Demo updates itself.\n#\n# It also fetches a helper.\n#\n"));
        assert!(!s.contains("{{"), "every placeholder is filled");
        let plain = InstallScript::new("Demo", "demo", "me/demo").render();
        assert!(!plain.contains("{{"));
    }

    #[test]
    fn the_shell_script_names_the_app() {
        let s = InstallScript::new("Demo App", "demo", "me/demo")
            .after_sh("echo '  extra step'")
            .done("It's done.")
            .render_sh();
        assert!(s.starts_with("#!/bin/sh\n"));
        assert!(s.contains("REPO='me/demo'"));
        assert!(s.contains("${DEMO_HOME:-}"));
        assert!(s.contains("NAME='Demo App'"));
        assert!(s.contains("echo '  extra step'\n\necho"));
        assert!(s.contains(r"'It'\''s done.'"));
        assert!(!s.contains("{{"), "every placeholder is filled");
        assert!(!s.contains('\r'));
    }

    #[test]
    fn the_scripts_quote_what_they_are_given() {
        let tricky = "Bob's $App `x` 100% \u{2019}q";
        let script = InstallScript::new(tricky, "bob", "me/bob").done("Done, Bob's.");
        let ps = script.render();
        assert!(ps.contains("$Name = 'Bob''s $App `x` 100% \u{2019}\u{2019}q'"), "{ps}");
        assert!(ps.contains("Write-Host 'Done, Bob''s.'"));
        assert!(ps.contains("# Installs Bob's $App"), "comments take it as it is");
        let sh = script.render_sh();
        assert!(sh.contains(r"NAME='Bob'\''s $App `x` 100% ’q'"), "{sh}");
        assert!(sh.contains(r"printf '%s\n' 'Done, Bob'\''s.'"));
        // The name is only ever quoted or in a comment.
        for line in ps.lines().chain(sh.lines()) {
            if line.contains("Bob") && !line.trim_start().starts_with('#') {
                assert!(line.contains("'Bob'") || line.contains("'Done"), "{line}");
            }
        }
        let broken = InstallScript::new("Two\nLines", "demo", "me/demo").render_sh();
        assert!(broken.contains("# Installs Two Lines on macOS"));
    }

    /// `sh -n` reads the whole script without running it.
    #[cfg(unix)]
    #[test]
    fn the_shell_script_parses() {
        let root = temp("sh");
        let script = root.join("install.sh");
        fs::write(
            &script,
            InstallScript::new("Bob's $App `x` 100%", "demo", "me/demo")
                .after_sh("echo done")
                .render_sh(),
        )
        .unwrap();
        let status = std::process::Command::new("sh")
            .arg("-n")
            .arg(&script)
            .status()
            .unwrap();
        assert!(status.success());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn names_platforms_and_their_files() {
        let p = platform();
        assert!(
            p.contains('-') && !p.contains("x86_64") && !p.contains("aarch64"),
            "{p}"
        );
        assert_eq!(asset_name("demo", "windows-x64"), "demo-windows-x64.exe");
        assert_eq!(asset_name("demo", "macos-arm64"), "demo-macos-arm64");
        assert_eq!(fallback_platform("macos-arm64"), Some("macos-x64"));
        assert_eq!(fallback_platform("linux-arm64"), None);
    }
}
