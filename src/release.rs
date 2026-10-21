//! The release side of [`crate::update`]: what publishes the
//! `manifest.json` the updater reads. Copy these into the app's repo and
//! keep them current with a test, the way [`crate::install::InstallScript`]
//! does for `install.ps1`:
//!
//! - [`NOTES_SCRIPT`] as `scripts/release-notes.ps1`: CHANGELOG.md's section
//!   for a version as the release description and the in-app notes;
//! - [`Workflow`] as `.github/workflows/release.yml`: on a tag, builds,
//!   writes the manifest and publishes the release.

/// `scripts/release-notes.ps1`.
pub const NOTES_SCRIPT: &str = include_str!("../templates/release-notes.ps1");

/// `.github/workflows/release.yml` for an app: a build on each platform,
/// then one release with every file and the manifest.
#[derive(Debug, Clone)]
pub struct Workflow {
    /// The exe without `.exe`.
    pub app: String,
    /// The Cargo package that builds it.
    pub package: String,
    /// What to build, from [`PLATFORMS`].
    pub platforms: Vec<String>,
}

/// The platforms the workflow can build: name, GitHub runner, Rust target.
pub const PLATFORMS: [(&str, &str, &str); 6] = [
    ("windows-x64", "windows-latest", "x86_64-pc-windows-msvc"),
    ("windows-arm64", "windows-11-arm", "aarch64-pc-windows-msvc"),
    ("linux-x64", "ubuntu-22.04", "x86_64-unknown-linux-gnu"),
    ("linux-arm64", "ubuntu-22.04-arm", "aarch64-unknown-linux-gnu"),
    ("macos-x64", "macos-latest", "x86_64-apple-darwin"),
    ("macos-arm64", "macos-latest", "aarch64-apple-darwin"),
];

impl Workflow {
    /// Builds for Windows, Linux and macOS on x64 and Arm, Windows on Arm
    /// aside (it runs the x64 build).
    pub fn new(app: &str, package: &str) -> Self {
        Workflow {
            app: app.to_owned(),
            package: package.to_owned(),
            platforms: ["windows-x64", "linux-x64", "linux-arm64", "macos-x64", "macos-arm64"]
                .map(String::from)
                .to_vec(),
        }
    }

    /// Only these, by their names in [`PLATFORMS`]: `&["windows-x64"]` for a Windows-only app.
    pub fn platforms(mut self, platforms: &[&str]) -> Self {
        self.platforms = platforms.iter().map(|p| (*p).to_owned()).collect();
        self
    }

    /// Panics on a platform [`PLATFORMS`] doesn't list.
    pub fn render(&self) -> String {
        let matrix: String = self
            .platforms
            .iter()
            .map(|p| {
                let (name, runner, target) = PLATFORMS
                    .iter()
                    .find(|(name, ..)| name == p)
                    .unwrap_or_else(|| panic!("no platform {p:?} in abalone::release::PLATFORMS"));
                format!("          - platform: {name}\n            runner: {runner}\n            target: {target}\n")
            })
            .collect();
        include_str!("../templates/release.yml")
            .replace("\r\n", "\n")
            .replace("{{MATRIX}}\n", &matrix)
            .replace("{{APP}}", &self.app)
            .replace("{{PACKAGE}}", &self.package)
    }
}

/// `a` and `b` are the same text, whatever their line endings.
pub fn same_text(a: &str, b: &str) -> bool {
    a.replace("\r\n", "\n") == b.replace("\r\n", "\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_workflow_names_the_app() {
        let w = Workflow::new("demo", "demo-cli").render();
        assert!(w.contains("          - platform: macos-arm64\n            runner: macos-latest\n"));
        assert!(w.contains("cargo build --release --locked -p demo-cli"));
        let windows = Workflow::new("demo", "demo-cli").platforms(&["windows-x64"]).render();
        assert!(windows.contains("platform: windows-x64") && !windows.contains("platform: linux"));
        // GitHub's own `${{ … }}` stays; only the template's placeholders go.
        for p in ["{{APP}}", "{{PACKAGE}}", "{{MATRIX}}"] {
            assert!(!w.contains(p), "{p}");
        }
        assert!(w.contains("${{ github.token }}"));
    }
}
