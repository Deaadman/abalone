//! Updates from GitHub releases. Each release carries `manifest.json`:
//!
//! ```json
//! {"version": "0.2.0", "platforms": {"windows-x64": {"file": "my-app-windows-x64.exe", "sha256": "…"},
//!                                    "macos-arm64": {"file": "my-app-macos-arm64", "sha256": "…"}},
//!  "notes": ["What's new, one line each"]}
//! ```
//!
//! `notes` is optional; the release workflow template writes it from
//! CHANGELOG.md. Other keys are kept in [`Manifest::extra`] for the app.
//! An update saves the notes so the first launch of the new version can say
//! what's new ([`WhatsNew::take`]).
//!
//! An update downloads the new exe into `versions/<version>`, checks its
//! hash, and runs it as `<app> install --update`: the new version installs
//! itself (see [`crate::install`]). The running copy is untouched until it
//! exits.
//!
//! In the shell, [`Updater::background`] is the check to hand to
//! [`crate::Shell::update_check`]:
//!
//! ```no_run
//! # use std::time::Duration;
//! # struct App;
//! # impl abalone::App for App {
//! #     type Msg = (); type Panel = abalone::NoPanel;
//! #     fn commands(&self) -> Vec<abalone::CommandInfo> { vec![] }
//! #     fn run(&mut self, _: &str, _: &mut abalone::Ctx<Self>) {}
//! # }
//! use abalone::update::Updater;
//! let updater = Updater::new("My App", "my-app", "me/my-app", env!("CARGO_PKG_VERSION"), "data").unwrap();
//! abalone::Shell::new(App)
//!     .update_check(Duration::from_secs(6 * 60 * 60), "/update", updater.background(|| true))
//!     .run()
//!     .unwrap();
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::install::{self, Layout};
use crate::out::{self, Kind, Line};
use crate::status::UpdateNote;
use crate::theme::theme;

/// One update at a time per process (a background check and an update command).
static BUSY: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub version: String,
    pub platforms: BTreeMap<String, Asset>,
    #[serde(default)]
    pub notes: Vec<String>,
    /// Everything else in the manifest.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Asset {
    pub file: String,
    pub sha256: String,
}

impl Manifest {
    /// The build for `platform`, or one that runs there ([`install::fallback_platform`]).
    pub fn asset(&self, platform: &str) -> Result<&Asset> {
        self.platforms
            .get(platform)
            .or_else(|| install::fallback_platform(platform).and_then(|p| self.platforms.get(p)))
            .with_context(|| format!("release {} has no {platform} build", self.version))
    }
}

/// What an installed update brings, shown once on its first launch.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WhatsNew {
    pub version: String,
    pub notes: Vec<String>,
    /// What the app chose to keep from the manifest's other keys.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl WhatsNew {
    pub fn path(data_dir: &Path) -> PathBuf {
        data_dir.join("whats-new.json")
    }

    pub fn save(&self, data_dir: &Path) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(Self::path(data_dir), json);
        }
    }

    /// The notes for `version` (this build), removed so they show once.
    /// Notes for an older version are removed too; those for a newer one
    /// stay for it (an older copy may still be running).
    pub fn take(data_dir: &Path, version: &str) -> Option<Self> {
        let path = Self::path(data_dir);
        let w: Self = serde_json::from_str(&std::fs::read_to_string(&path).ok()?).ok()?;
        if !install::is_newer(&w.version, version) {
            let _ = std::fs::remove_file(&path);
        }
        (w.version == version).then_some(w)
    }
}

/// A "what's new" block: `✓ title · what's new`, a bullet for each note,
/// and where to read more.
pub fn news(title: &str, notes: &[String], link: &str) -> Vec<Line> {
    let t = theme();
    let mut heading = vec![(Kind::Paint(t.good, true), "✓ ".into()), (Kind::Bold, title.to_owned())];
    if !notes.is_empty() {
        heading.push((Kind::Dim, " · what's new".into()));
    }
    let mut lines = vec![heading];
    for n in notes {
        lines.push(vec![
            (Kind::Paint(t.accent, false), "  • ".into()),
            (Kind::Out, n.clone()),
        ]);
    }
    lines.push(vec![(Kind::Dim, format!("  {link} for details"))]);
    lines.push(vec![(Kind::Out, String::new())]);
    lines
}

/// What `update_command` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Updated {
    UpToDate,
    /// This version was installed; it runs from the next launch.
    Installed(String),
}

/// The last update check, for a status screen.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct State {
    /// Seconds since 1970.
    pub checked_at: u64,
    pub result: String,
}

impl State {
    pub fn path(data_dir: &Path) -> PathBuf {
        data_dir.join("update.json")
    }

    pub fn load(data_dir: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(Self::path(data_dir)).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// Best effort: a failed write only loses the status line.
    pub fn record(data_dir: &Path, result: impl Into<String>) {
        let state = State {
            checked_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
            result: result.into(),
        };
        if std::fs::create_dir_all(data_dir).is_ok()
            && let Ok(json) = serde_json::to_string_pretty(&state)
        {
            let _ = std::fs::write(Self::path(data_dir), json);
        }
    }
}

type Keep = Arc<dyn Fn(&Manifest) -> Map<String, Value> + Send + Sync>;

/// Where updates come from and how they install.
#[derive(Clone)]
pub struct Updater {
    /// The name people see: `Vantage`.
    pub name: String,
    /// `owner/repo` on GitHub.
    pub repo: String,
    /// The manifest's platform key: this build's ([`install::platform`]) unless set.
    pub platform: String,
    /// This build's version.
    pub version: String,
    /// The start of the environment variables: `VANTAGE` reads `VANTAGE_DISABLE_AUTOUPDATER`.
    pub env_prefix: String,
    pub layout: Layout,
    pub data_dir: PathBuf,
    /// What the new exe runs with: `install --update`, then `--data-dir <dir>` when `data_dir_flag` is set.
    pub install_args: Vec<String>,
    pub data_dir_flag: Option<String>,
    keep: Option<Keep>,
}

impl Updater {
    /// For `app` (the exe's name), from GitHub's `repo`, installed in [`Layout::new`].
    pub fn new(name: &str, app: &str, repo: &str, version: &str, data_dir: impl Into<PathBuf>) -> Result<Self> {
        Ok(Updater {
            name: name.to_owned(),
            repo: repo.to_owned(),
            platform: install::platform(),
            version: version.to_owned(),
            env_prefix: install::env_prefix(app),
            layout: Layout::new(app)?,
            data_dir: data_dir.into(),
            install_args: vec!["install".into(), "--update".into()],
            data_dir_flag: None,
            keep: None,
        })
    }

    pub fn platform(mut self, platform: &str) -> Self {
        self.platform = platform.to_owned();
        self
    }

    pub fn layout(mut self, layout: Layout) -> Self {
        self.layout = layout;
        self
    }

    /// Pass the data dir to the new exe as `flag <dir>`.
    pub fn data_dir_flag(mut self, flag: &str) -> Self {
        self.data_dir_flag = Some(flag.to_owned());
        self
    }

    /// Which of the manifest's other keys an update saves for "what's new"
    /// (none by default). It runs in the old version, before the new one.
    pub fn keep_extra(mut self, keep: impl Fn(&Manifest) -> Map<String, Value> + Send + Sync + 'static) -> Self {
        self.keep = Some(Arc::new(keep));
        self
    }

    /// Set to anything but `0` to stop background checks.
    pub fn disable_env(&self) -> String {
        format!("{}_DISABLE_AUTOUPDATER", self.env_prefix)
    }

    /// A base URL serving `manifest.json` and the files it names, in place
    /// of GitHub (for testing an update end to end).
    pub fn url_env(&self) -> String {
        format!("{}_UPDATE_URL", self.env_prefix)
    }

    /// `my-app/1.2.0`, for requests.
    pub fn agent(&self) -> String {
        format!("{}/{}", self.layout.app, self.version)
    }

    /// Why background checks are off, if they are: the environment, or
    /// `setting` is false.
    pub fn auto_disabled(&self, setting: bool) -> Option<String> {
        let env = self.disable_env();
        if std::env::var_os(&env).is_some_and(|v| !v.is_empty() && v != "0") {
            return Some(format!("disabled by {env}"));
        }
        (!setting).then(|| "turned off in the settings".to_owned())
    }

    /// The URL override, if set.
    pub fn override_base(&self) -> Option<String> {
        std::env::var(self.url_env())
            .ok()
            .filter(|u| !u.is_empty())
            .map(|u| u.trim_end_matches('/').to_owned())
    }

    /// `https://github.com/<repo>/releases/tag`, for "what's new" links.
    pub fn releases(&self) -> String {
        format!("github.com/{}/releases/tag", self.repo)
    }

    /// A file on the repo's `main` branch (or under the URL override).
    pub fn main_file(&self, path: &str) -> String {
        match self.override_base() {
            Some(base) => format!("{base}/{path}"),
            None => format!("https://raw.githubusercontent.com/{}/main/{path}", self.repo),
        }
    }

    /// GET `url` as this app.
    pub fn get_text(&self, url: &str) -> Result<String> {
        let mut resp = install::http_get(url, &self.agent())?;
        resp.body_mut()
            .read_to_string()
            .with_context(|| format!("reading {url}"))
    }

    /// The newest release's manifest.
    pub fn latest(&self) -> Result<Manifest> {
        let url = match self.override_base() {
            Some(base) => format!("{base}/manifest.json"),
            None => format!(
                "https://github.com/{}/releases/latest/download/manifest.json",
                self.repo
            ),
        };
        let text = self.get_text(&url)?;
        let manifest: Manifest = serde_json::from_str(&text).with_context(|| format!("parsing {url}"))?;
        if install::version_key(&manifest.version).iter().all(|&n| n == 0) {
            bail!("{url} names no usable version ({:?})", manifest.version);
        }
        Ok(manifest)
    }

    /// Where a release's file is downloaded from: the tag's own URL, so a
    /// release published mid-update cannot mix files from two versions.
    pub fn file_url(&self, manifest: &Manifest, file: &str) -> String {
        match self.override_base() {
            Some(base) => format!("{base}/{file}"),
            None => format!(
                "https://github.com/{}/releases/download/{}/{file}",
                self.repo, manifest.version
            ),
        }
    }

    /// Downloads, verifies and installs `manifest`'s version by running it
    /// with `install_args`. Returns the installed exe.
    pub fn apply(&self, manifest: &Manifest) -> Result<PathBuf> {
        let _busy = BUSY.lock().unwrap_or_else(|e| e.into_inner());
        let asset = manifest.asset(&self.platform)?;
        let exe = self.layout.version_exe(&manifest.version);
        let present = exe.is_file() && install::check_sha256(&exe, &asset.sha256, "").is_ok();
        if !present {
            let dir = exe.parent().expect("version_exe has a parent");
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
            let url = self.file_url(manifest, &asset.file);
            install::download_verified(&url, &exe, &self.agent(), &asset.sha256, &asset.file)?;
            install::make_executable(&exe)?;
        }
        let mut cmd = Command::new(&exe);
        cmd.args(&self.install_args)
            .env(install::home_env(&self.layout.app), &self.layout.root);
        if let Some(flag) = &self.data_dir_flag {
            cmd.arg(flag).arg(&self.data_dir);
        }
        let output = cmd.output().with_context(|| format!("running {}", exe.display()))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let last = stderr
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("no error output");
            bail!("{} {} failed to install itself: {last}", self.name, manifest.version);
        }
        WhatsNew {
            version: manifest.version.clone(),
            notes: manifest.notes.clone(),
            extra: self.keep.as_ref().map(|k| k(manifest)).unwrap_or_default(),
        }
        .save(&self.data_dir);
        Ok(exe)
    }

    /// The layout, when the running exe was installed into it.
    pub fn installed(&self) -> Option<&Layout> {
        self.layout.runs_this().then_some(&self.layout)
    }

    /// Check for a newer version: install it when `install_it` and this copy
    /// was installed, else only say one exists. Offline or failing checks
    /// stay quiet (they're recorded in [`State`]).
    pub fn check(&self, install_it: bool) -> Option<UpdateNote> {
        let layout = self.installed();
        if let Some(l) = layout {
            install::cleanup(l, &self.version);
        }
        let manifest = match self.latest() {
            Ok(m) => m,
            Err(e) => {
                State::record(&self.data_dir, format!("check failed: {e:#}"));
                return None;
            }
        };
        let v = manifest.version.clone();
        if !install::is_newer(&v, &self.version) {
            State::record(&self.data_dir, format!("up to date ({})", self.version));
            return None;
        }
        if layout.is_none() || !install_it {
            State::record(&self.data_dir, format!("{v} available"));
            return Some(UpdateNote::Available);
        }
        Some(match self.apply(&manifest) {
            Ok(_) => {
                State::record(&self.data_dir, format!("installed {v}"));
                UpdateNote::Installed
            }
            Err(e) => {
                State::record(&self.data_dir, format!("failed: {e:#}"));
                UpdateNote::Failed
            }
        })
    }

    /// The notes of the update that installed this build, the first time
    /// it runs: print them with [`out::row`], or show them in the shell's
    /// header from [`crate::App::launch`].
    pub fn whats_new(&self) -> Option<Vec<Line>> {
        let w = WhatsNew::take(&self.data_dir, &self.version)?;
        Some(self.news(&w))
    }

    /// [`news`] for an update, linking to its release.
    pub fn news(&self, w: &WhatsNew) -> Vec<Line> {
        news(
            &format!("{} updated to v{}", self.name, w.version),
            &w.notes,
            &format!("{}/{}", self.releases(), w.version),
        )
    }

    /// `<app> update`: installs the newest release, or says this one is.
    /// Works whether or not this copy was installed (the update installs
    /// into [`Updater::layout`] either way).
    pub fn update_command(&self) -> Result<Updated> {
        if let Some(l) = self.installed() {
            install::cleanup(l, &self.version);
        }
        let manifest = self.latest().context("checking for updates failed")?;
        if !install::is_newer(&manifest.version, &self.version) {
            State::record(&self.data_dir, format!("up to date ({})", self.version));
            crate::outln!("{} is up to date (v{})", self.name, self.version);
            return Ok(Updated::UpToDate);
        }
        match self.apply(&manifest) {
            Ok(_) => {
                State::record(&self.data_dir, format!("installed {}", manifest.version));
                out::row(vec![
                    (Kind::Paint(theme().good, true), "✓ ".into()),
                    (Kind::Bold, "New version installed".into()),
                ]);
                out::emit(
                    Kind::Dim,
                    format!(
                        "  Restart {} to use it: /exit, then run {} again. What's new shows when it opens.",
                        self.name, self.layout.app
                    ),
                );
                Ok(Updated::Installed(manifest.version))
            }
            Err(e) => {
                State::record(&self.data_dir, format!("failed: {e:#}"));
                Err(e)
            }
        }
    }

    /// The check for [`crate::Shell::update_check`]: installs updates while
    /// `auto` says so (and the environment doesn't turn it off), and stops
    /// checking once one is installed.
    pub fn background(
        self,
        auto: impl Fn() -> bool + Send + Sync + 'static,
    ) -> impl Fn(Option<UpdateNote>) -> Option<UpdateNote> + Send + Sync + 'static {
        move |note| {
            if note == Some(UpdateNote::Installed) {
                return note;
            }
            let install_it = self.auto_disabled(auto()).is_none();
            self.check(install_it)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_manifest_and_keeps_other_keys() {
        let m: Manifest = serde_json::from_str(
            r#"{
                "version": "0.2.0",
                "platforms": {"windows-x64": {"file": "demo-windows-x64.exe", "sha256": "ab"}},
                "notes": ["One.", "Two."],
                "helper": {"tag": "v0.3.0"}
            }"#,
        )
        .unwrap();
        assert_eq!(m.asset("windows-x64").unwrap().file, "demo-windows-x64.exe");
        assert!(m.asset("linux-x64").is_err());
        assert_eq!(
            m.asset("windows-arm64").unwrap().file,
            "demo-windows-x64.exe",
            "Arm Windows falls back to x64"
        );
        assert_eq!(m.notes, ["One.", "Two."]);
        assert_eq!(m.extra["helper"]["tag"], "v0.3.0");
    }

    #[test]
    fn whats_new_shows_once() {
        let dir = std::env::temp_dir().join(format!("abalone-whats-new-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut extra = Map::new();
        extra.insert("helper".into(), serde_json::json!({"tag": "v1"}));
        WhatsNew {
            version: "0.2.0".into(),
            notes: vec!["New.".into()],
            extra,
        }
        .save(&dir);
        assert!(WhatsNew::take(&dir, "0.1.0").is_none(), "an older build leaves them");
        let w = WhatsNew::take(&dir, "0.2.0").unwrap();
        assert_eq!(w.notes, ["New."]);
        assert_eq!(w.extra["helper"]["tag"], "v1");
        assert!(WhatsNew::take(&dir, "0.2.0").is_none());
        // A newer build than the notes are for removes them unseen.
        WhatsNew {
            version: "0.2.0".into(),
            ..WhatsNew::default()
        }
        .save(&dir);
        assert!(WhatsNew::take(&dir, "0.3.0").is_none());
        assert!(!WhatsNew::path(&dir).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn names_its_urls_and_environment() {
        let u = Updater::new("Demo", "demo", "me/demo", "0.1.0", "C:/data")
            .unwrap()
            .layout(Layout::at("C:/nowhere", "demo"));
        assert_eq!(u.disable_env(), "DEMO_DISABLE_AUTOUPDATER");
        assert_eq!(u.agent(), "demo/0.1.0");
        if u.override_base().is_none() {
            assert_eq!(
                u.main_file("x.json"),
                "https://raw.githubusercontent.com/me/demo/main/x.json"
            );
        }
        assert!(u.installed().is_none());
    }
}
