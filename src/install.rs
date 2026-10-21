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
    if launcher.exists() {
        let aside = layout.bin().join(format!("{name}.old-{}", unique_suffix()));
        fs::rename(&launcher, &aside).with_context(|| format!("moving the old {} aside", launcher.display()))?;
    }
    fs::rename(&staged, &launcher).with_context(|| format!("placing {}", launcher.display()))
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
    let mut others: Vec<(Vec<u64>, PathBuf)> = entries
        .flatten()
        .filter(|e| e.path().is_dir() && e.file_name() != keep)
        .map(|e| (version_key(&e.file_name().to_string_lossy()), e.path()))
        .collect();
    others.sort();
    let n = others.len().saturating_sub(KEEP_OLDER);
    for (_, path) in others.into_iter().take(n) {
        let _ = fs::remove_dir_all(path);
    }
}

/// `v1.2.3` or `1.2.3-beta` as numbers for ordering; non-numeric parts are 0.
pub fn version_key(v: &str) -> Vec<u64> {
    v.trim_start_matches('v')
        .split(['-', '+'])
        .next()
        .unwrap_or("")
        .split('.')
        .map(|p| p.parse().unwrap_or(0))
        .collect()
}

/// True when `candidate` is a higher version than `current`.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    version_key(candidate) > version_key(current)
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
pub fn http_get(url: &str, agent: &str) -> Result<ureq::http::Response<ureq::Body>> {
    ureq::get(url)
        .header("User-Agent", agent)
        .config()
        .timeout_global(Some(Duration::from_secs(300)))
        .build()
        .call()
        .with_context(|| format!("GET {url}"))
}

/// Downloads `url` to `dest` (through a temporary name).
pub fn download(url: &str, dest: &Path, agent: &str) -> Result<()> {
    let mut resp = http_get(url, agent)?;
    let partial = dest.with_extension("download");
    let mut file = fs::File::create(&partial).with_context(|| format!("creating {}", partial.display()))?;
    let mut reader = resp.body_mut().with_config().limit(MAX_DOWNLOAD).reader();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = reader.read(&mut buf).with_context(|| format!("downloading {url}"))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
    }
    file.sync_all()?;
    drop(file);
    fs::rename(&partial, dest).with_context(|| format!("placing {}", dest.display()))
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
        self.fill(include_str!("../templates/install.ps1"), &self.after)
            .replace("{{DONE}}", &self.done.replace('\'', "''"))
    }

    /// `install.sh`, for macOS and Linux.
    pub fn render_sh(&self) -> String {
        self.fill(include_str!("../templates/install.sh"), &self.after_sh)
            .replace("{{DONE}}", &self.done.replace('\'', r"'\''"))
    }

    fn fill(&self, template: &str, after: &str) -> String {
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
        // A checkout with CRLF endings still fills the template.
        template
            .replace("\r\n", "\n")
            .replace("{{ABOUT}}", &about)
            .replace("{{NAME}}", &self.name)
            .replace("{{APP}}", &self.app)
            .replace("{{ENV}}", &env_prefix(&self.app))
            .replace("{{REPO}}", &self.repo)
            .replace("{{AFTER}}\n", &after)
    }
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
        assert!(s.contains("Installing Demo App"));
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
        assert!(s.contains("Installing Demo App"));
        assert!(s.contains("echo '  extra step'\n\necho"));
        assert!(s.contains(r"'It'\''s done.'"));
        assert!(!s.contains("{{"), "every placeholder is filled");
        assert!(!s.contains('\r'));
    }

    /// `sh -n` reads the whole script without running it.
    #[cfg(unix)]
    #[test]
    fn the_shell_script_parses() {
        let root = temp("sh");
        let script = root.join("install.sh");
        fs::write(
            &script,
            InstallScript::new("Demo", "demo", "me/demo")
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
