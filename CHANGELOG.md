# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/2.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!--
A version's section is its GitHub release description, so write each entry
for someone using Abalone in their app: one plain sentence about what
changed for them. Changes that break their code go first, under Changed or
Removed, and say what to do instead.

- Add entries under "## [Unreleased]" as you go, grouped under Added,
  Changed, Deprecated, Removed, Fixed or Security.
- To release, rename it to "## [0.2.0] - YYYY-MM-DD" (no v), add a fresh
  "## [Unreleased]" above it, and update the links at the bottom.

See docs/development.md, "Releasing". CI checks this file's format.
-->

## [Unreleased]

### Added

- `text_width`, `char_width` and `fit_width`, to measure text in terminal columns; `row_width`, `pad` and `wrap` now count wide characters as two columns.
- `install::compare_versions` and `install::download_verified`.

### Fixed

- A space typed into an empty input box no longer crashes the shell, and nor does `0` in the help and settings panels.
- Enter or Tab with the cursor inside a word replaces the whole word, so it no longer runs the wrong command.
- A command typed in full runs even when a longer name starting the same way is listed first, and the suggestion shows when it's typed in capitals.
- Characters typed with AltGr on Windows (such as `@`, `\` and `{`) reach the input box and settings fields.
- Pasted lines become one line with spaces between them, rather than running together.
- An app's own `quit` command runs, rather than quitting the shell.
- On a short terminal the input box stays on screen when the command menu opens, and every panel fits.
- The status bar's "more lines" count is right, and no longer shows when there is nothing to scroll.
- Chinese, Japanese and emoji text no longer runs past the edge of the screen, and the cursor sits where it should beside them.
- The end of a long sent line no longer goes missing at the right edge.
- Colours and other terminal escape codes in command output are taken out, rather than moving the cursor or breaking the layout.
- A crash's message is printed after the terminal is restored, rather than vanishing with the full-screen view.
- The settings Status tab scrolls on its own, an open choice list stays in view, and a long label keeps a space before its value.
- Settings are saved through a temporary file, so a crash mid-save no longer leaves a broken file.
- Logs stop growing at 10 MB, pruning no longer deletes another app's logs, and the user name is only taken out of whole words.
- `hex` rejects text like `#+f+f+f`.
- Pre-release versions sort before their release, so `1.1.0` updates a `1.1.0-beta`, and the release workflow marks pre-releases as such.
- Updates check a download's hash before putting it in place, two copies of an app updating at once no longer spoil each other's download, and a slow connection no longer times out.
- A failed launcher swap on Windows puts the old launcher back.
- An update's "what's new" notes are no longer taken by an older copy of the app.
- The install scripts work with app names containing quotes, `$` or `%`; `install.ps1` keeps the user's PATH entries unexpanded and leaves the session's settings as they were; `install.sh` says when a download fails and refuses to install without a hash check.

### Security

- The release workflows give a write token only to the job that publishes the release.

## [1.0.0] - 2026-10-21

### Added

- A full-screen shell: a header and logo, an input box with a command menu, a status bar and a footer, each of which you can change.
- `/help`, `/settings` and `/release-notes` panels, and panels of your own.
- `/tui`, to draw full screen or in the terminal's own scrollback.
- A log file per run for bug reports, and a clear message after a crash.
- The data folder, settings in a TOML file, and pictures drawn in the terminal.
- Install scripts for Windows, macOS and Linux, updates from GitHub releases, and the release workflow that publishes them.

[Unreleased]: https://github.com/Deaadman/abalone/compare/1.0.0...HEAD
[1.0.0]: https://github.com/Deaadman/abalone/releases/tag/1.0.0
