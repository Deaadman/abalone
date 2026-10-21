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
