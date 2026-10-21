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

- `Theme::on_light`, `Theme::for_terminal`, `readable` and `for_light` give colours that read on a light background, and `light_background` and `set_light_background` say whether the terminal has one.

### Fixed

- On a light background the default theme darkens to read there, and matched letters in the command menu and the default logo's prompt use the terminal's own text colour instead of near white.
- Menu, release-notes and settings picker widths count terminal columns, so wide characters line up.
- Inline mode (`/tui default`) clears and redraws when the terminal's width changes, instead of leaving copies of the input box behind, and never prints rows wider than a terminal narrower than 20 columns.
- An open settings panel's Status tab is read again when the app refreshes, instead of only after a change in the panel.

## [1.0.0] - 2026-10-21

### Added

- The first release.

[Unreleased]: https://github.com/Deaadman/abalone/compare/1.0.0...HEAD
[1.0.0]: https://github.com/Deaadman/abalone/releases/tag/1.0.0
