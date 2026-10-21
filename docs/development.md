# Working on shellkit

## Building and testing

You need [Rust](https://rustup.rs). Then:

```sh
cargo test --all-features
cargo run --example custom
```

`cargo run --example custom -- --inline` starts in the terminal's own scrollback.

Before you push, run what CI runs:

```sh
cargo fmt --check
cargo clippy --all-features --all-targets
cargo clippy --no-default-features --all-targets
cargo test --all-features
cargo test --no-default-features
```

CI runs these on Windows, macOS and Linux, with warnings as errors.

## Layout

| Where | What's there |
| --- | --- |
| `src/shell.rs` | The shell: the `App` trait, `Shell`, drawing full screen and inline |
| `src/skin.rs`, `src/status.rs`, `src/widgets.rs` | How each part is drawn |
| `src/help.rs`, `src/settings.rs`, `src/notes.rs` | The built-in panels |
| `src/editor.rs` | The input box and the command menu |
| `src/log.rs`, `src/files.rs`, `src/image.rs` | Logs, files and pictures |
| `src/install.rs`, `src/update.rs`, `src/release.rs` | Installing, updating and the release templates |
| `templates/` | The install scripts, release notes script and release workflow that apps copy |
| `examples/` | `minimal` and `custom`, which the docs' pictures are drawn from |

## The docs' pictures

The pictures in `docs/images` are drawn from the examples' real screens. After changing something they show, draw them again:

```sh
cargo test --all-features --test docs_pictures -- --ignored
```

Look over the changed pictures before committing. CI fails if they're out of date.

## Using a local copy in an app

To try a change in an app before it's released, point the app at your copy in its `Cargo.toml`:

```toml
[patch."https://github.com/Deaadman/shellkit"]
shellkit = { path = "../shellkit" }
```

## Releasing

Apps depend on shellkit by git tag, so a release is a tag and its notes.

1. In `CHANGELOG.md`, rename `## [Unreleased]` to `## [0.2.0] - YYYY-MM-DD`, add a fresh `## [Unreleased]` above it, and update the links at the bottom.
2. Set `version` in `Cargo.toml` to the same version, and commit.
3. Tag it with no `v` and push the tag: `git tag 0.2.0 && git push origin 0.2.0`.

The release workflow runs all of CI first. Then it checks that the tag matches `Cargo.toml`, and publishes the release with that version's section of `CHANGELOG.md` as its description.

Follow [Semantic Versioning](https://semver.org): while the version starts with `0.`, a change that breaks apps' code bumps the middle number (0.1 to 0.2), and anything else the last.
