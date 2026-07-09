# Contributing to Slug

Thank you for your interest in Slug. Contributions of all kinds are welcome — bug reports, feature ideas, documentation fixes, and code changes.

## Before you start

- Check the [open issues](https://github.com/dais-heroique/Slug/issues) to avoid duplicating work.
- For significant changes, open an issue first to discuss the approach before writing code.
- Read `SLUG-AGENT-GUIDE.md` to understand how Slug drives native apps at the accessibility layer — it gives essential context for any agent-side contribution.

## Setting up the workspace

```bash
# Rust stable (MSRV 1.77.2)
rustup update stable

# Clone and build (excludes the Python binding to avoid needing a Python toolchain)
git clone https://github.com/dais-heroique/Slug
cd Slug
cargo build --workspace --exclude slug-ui-py
```

On macOS you will need to grant the terminal Accessibility permission once (`System Settings → Privacy & Security → Accessibility`) before live tests can run.

## Running the tests

```bash
# Unit + integration tests (no live desktop required)
cargo test --workspace

# With live-test feature (needs a real macOS/Windows session)
cargo test --workspace --features live-tests
```

CI runs `cargo build`, `cargo clippy -D warnings`, and `cargo test` on macOS, Windows, and Linux. All three must pass before a PR can merge.

## Code style

- **Rust edition 2021.** Run `cargo fmt` before committing.
- **Clippy with `-D warnings`.** CI treats every warning as an error.
- **Platform guards.** Anything OS-specific must be behind `#[cfg(unix)]` / `#[cfg(windows)]` / `#[cfg(target_os = "macos")]`. Imports used only inside a cfg block must live inside that block.
- **No API keys in source.** Keys belong in env vars or `~/.slug/secrets.env`, never in `slug.toml` or committed code.
- **No model identifiers in committed text.** Keep AI model IDs out of commit messages, comments, and docs.

## Submitting a pull request

1. Fork and create a feature branch (`git checkout -b feat/your-change`).
2. Write focused commits with descriptive messages. One logical change per commit.
3. Ensure `cargo test --workspace` and `cargo clippy --workspace -- -D warnings` pass locally.
4. Open a PR against `main`. Fill in the PR description: what changed, why, and how to test it.
5. A maintainer will review within a few days. CI must be green before merge.

## Reporting bugs

Open a GitHub issue with:
- Slug version (`slug-mcp --version` or from the dashboard header).
- OS and version (macOS 14.x / Windows 11, etc.).
- Steps to reproduce.
- What you expected vs what actually happened.
- Relevant log output (the dashboard's log pane or `RUST_LOG=debug slug-cli`).

## Questions

Open a discussion or an issue tagged `question`. We prefer public discussion over private messages so answers benefit everyone.
