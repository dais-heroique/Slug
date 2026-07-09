# Changelog

All notable changes to Slug are documented here.  
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

---

## [Unreleased]

### Added
- `slug-ui` — retained-mode, declarative UI-as-data toolkit whose AccessKit semantic tree is derived **automatically and completely** from the same widget tree that renders. Opaque widgets are structurally impossible.
- `slug-notes` — notes-editor demo built with `slug-ui`; an agent drives it through the bus with zero vision (agent-drive integration test included).
- `slug-ui-ffi` — C ABI for `slug-ui` (cbindgen-generated header, mirrors AccessKit's binding strategy).
- `slug-ui-py` — PyO3 Python binding for `slug-ui`.
- Bus export over a local Unix socket (macOS/Linux) and Windows named pipe, with length-prefixed JSON framing and a canonical Cap'n Proto schema at `crates/slug-ui/schema/slug_ui.capnp`.
- Cross-platform `slug-ui` bus: `#[cfg(unix)]` uses `tokio::net::UnixListener`; `#[cfg(windows)]` uses `tokio::net::windows::named_pipe`.
- New eye-mark brand assets (`assets/logo.png`, `favicon-32.png`, `favicon-180.png`, `header-96.png`) — purple-on-black pixel-art eye.
- Dashboard fully rethemed to match the violet eye logo palette (`--accent:#8b5cf6`, `--grad1:#7c3aed`, `--grad2:#c084fc`).
- `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, `SECURITY.md`, `CHANGELOG.md`.

### Fixed
- Windows CI build: `use std::io::Write` scoped inside `#[cfg(unix)]` block in `dashboard_api.rs`.
- `slug_snapshot` unfiltered output capped at ~20 k characters with a clear truncation notice; `limit` alone narrows the tree without requiring `interactive_only:false`.
- Price / money / currency role group added to `slug_snapshot` so `roles:["price"]` matches any amount regardless of currency symbol format.

### Changed
- `slug-brain` upgraded to multi-provider support (Claude, OpenAI-compatible, local via any OpenAI-API endpoint). Active provider persisted in `slug.toml`; key stored in `~/.slug/secrets.env`.
- Dashboard MCP-server registry tab removed (out of scope for the control panel).
- Dashboard `slug_status` heartbeat now works correctly across stdio and HTTP transports.

---

## [0.1.0] — initial release

- `slug-core`: `SlugNode` / `SlugDelta` semantic schema, snapshot diffing, role taxonomy.
- `slug-bridge`: macOS AX and Windows UIA backends with live-test feature flag.
- `slug-mcp`: JSON-RPC 2.0 MCP server over stdio and streamable HTTP; tools: `slug_snapshot`, `slug_invoke`, `slug_wait_for`, `slug_list_apps`, `slug_launch`, `slug_sequence`, `slug_key`, `slug_agent_*`, `slug_status`.
- `slug-cli`: `run` and `test` subcommands.
- Control dashboard (single self-contained HTML, served at `GET /dashboard`) with agent control, semantic tree viewer, provider switcher, and approval gate.
- macOS app bundle (`slug-install/make-macos-app.sh`) and Windows installer (`slug-install/windows/slug.iss`).
