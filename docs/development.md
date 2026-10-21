# Working on Abalone

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

CI runs these on Windows, macOS and Linux, with warnings as errors. It also runs `cargo audit`, which fails if a crate in `Cargo.lock` has a [security advisory](https://rustsec.org).

Once a week, [Dependabot](../.github/dependabot.yml) opens a pull request for crates and actions with a newer version, with the minor and patch updates together. Merge it when CI passes.

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
[patch.crates-io]
abalone = { path = "../abalone" }
```

## Releasing

A release goes to crates.io, then to GitHub with its notes, when the [release workflow](../.github/workflows/release.yml) sees a version tag pushed. A manual run of it only checks.

1. In `CHANGELOG.md`, rename `## [Unreleased]` to `## [0.2.0] - YYYY-MM-DD`, add a fresh `## [Unreleased]` above it, and update the links at the bottom.
2. Set `version` in `Cargo.toml` to the same version, and commit.
3. Tag it with no `v` and push the tag: `git tag 0.2.0 && git push origin 0.2.0`.

The release workflow runs all of CI first. Then it checks that the tag matches `Cargo.toml` and that `CHANGELOG.md` has a section for it, publishes Abalone to crates.io, and makes a GitHub release with that section as its description. A version on crates.io can't be deleted or published again, only yanked, so check before you tag.

The workflow gets its crates.io token by [trusted publishing](https://crates.io/docs/trusted-publishing), which only works once Abalone is on crates.io. For the first release:

1. Make an API token at [crates.io/settings/tokens](https://crates.io/settings/tokens) with the `publish-new` and `publish-update` scopes, and add it to this repository as an Actions secret named `CARGO_REGISTRY_TOKEN`.
2. Release as above. The workflow uses the secret.
3. On crates.io, under Abalone's Settings > Trusted Publishing, add GitHub with owner `Deaadman`, repository `abalone` and workflow `release.yml`.
4. Delete the secret on GitHub and the token on crates.io. Later releases need neither.

Follow [Semantic Versioning](https://semver.org): a change that breaks apps' code bumps the first number (1.4 to 2.0), a new feature the middle one (1.4 to 1.5), and a fix the last.
