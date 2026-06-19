# Slug

**Slug** is a Linux OS design whose primary user is an AI agent: instead of
perceiving the screen through screenshots, the agent reads a mandatory, OS-wide
**semantic UI layer** — a typed, delta-compressed representation of every widget.
See [`docs/`](./docs) for the full design dossier.

This repository implements **Milestone 1** + **M1.5**: a *semantic bus* that
harvests the OS accessibility tree and exposes it as an **MCP server**, so you can
connect Claude Code (or any MCP client) and have it read and drive native apps —
no pixels required. As of **M1.5** the installable layer is **cross-platform**:

| OS | Accessibility source | Permissions |
|----|----------------------|-------------|
| **Linux** | AT-SPI2 over D-Bus | enable toolkit accessibility |
| **Windows** | UI Automation (`IUIAutomation`) | none |
| **macOS** | Accessibility API (`AXUIElement`) | grant Accessibility permission |

Only the platform perception/action layer (`slug-bridge`) differs per OS; the
semantic model (`slug-core`), the MCP server (`slug-mcp`), and the agent
(`slug-brain`) are identical everywhere. See [Platform backends](#platform-backends).

> Milestone 1 lives on the accessibility path, not the Wayland compositor. Two
> documented step-1 adaptations of the canonical spec apply (see
> [Milestone-1 adaptations](#milestone-1-adaptations)).

## Workspace layout

| Crate         | Role |
|---------------|------|
| `slug-core`   | The unified semantic document model — a faithful Rust mirror of [`docs/SEMANTIC-SCHEMA.md`](./docs/SEMANTIC-SCHEMA.md): `SlugNode`, `SlugRole`, `SlugState`, `SlugDelta`, stable refs, the document arena, and the Playwright-MCP-style YAML serializer. Depends only on `serde`. |
| `slug-bridge` | The cross-platform accessibility harvester. One `AccessibilityBackend` trait with three implementations — `backend_atspi` (Linux/AT-SPI2), `backend_uia` (Windows/UI Automation), `backend_ax` (macOS/AX) — selected per `cfg(target_os)`. Walks application trees, maps native roles/states to Slug, executes actions, and flags opaque (vision-fallback) apps. |
| `slug-mcp`    | The MCP server: JSON-RPC 2.0 over **stdio** and **streamable HTTP**, exposing four tools. This is the session layer that maps internal ULID refs to short agent-facing aliases (`b1`, `e5`). |
| `slug-cli`    | A `slug` binary for driving the bus by hand (snapshot, list apps, invoke actions, stream live events). |
| `slug-brain`  | A hybrid agentic loop (`slug-agent`) that drives the MCP tools. **Multi-provider**: Claude, OpenAI/OpenRouter/any OpenAI-compatible server, Gemini, or local Ollama — chosen in `slug.toml` or auto-selected from detected hardware. See [Providers](#providers-multi-provider-brain). |

```
agent ──MCP──► slug-mcp ──► slug-bridge ──AT-SPI2 / UIA / AX──► applications
                  │              │
                  └─ slug-core (SlugNode / SlugDocument / SlugDelta) ─┘
```

## Platform backends

`slug-bridge` defines one trait and selects an implementation at compile time:

```rust
trait AccessibilityBackend {
    fn enumerate_apps(&self) -> Result<Vec<AppHandle>>;
    fn snapshot_app(&self, app: &AppHandle) -> Result<Vec<SlugNode>>; // bounded walk
    fn invoke(&self, node_id: &BackendNodeId, action: &Action) -> Result<()>;
    fn subscribe_events(&self, sink: EventSink) -> Result<Subscription>;
    fn coverage(&self, app: &AppHandle) -> Coverage;
}
```

| Backend | `cfg` | Walk | Actions | Native node id (§4) |
|---------|-------|------|---------|---------------------|
| `backend_atspi` | `target_os = "linux"` | `AccessibleProxy.GetChildren` | `DoAction` / `SetTextContents` / `SetCurrentValue` / `GrabFocus` | `{bus_name}:{path}` |
| `backend_uia` | `target_os = "windows"` | `ControlViewWalker` | `Invoke` / `Value` / `Toggle` / `SelectionItem` / `ExpandCollapse` / `ScrollItem` / `SetFocus` patterns | stringified `RuntimeId` |
| `backend_ax` | `target_os = "macos"` | `kAXChildrenAttribute` | `AXUIElementPerformAction(kAXPress)` / `AXUIElementSetAttributeValue` | hash of `{pid}:{ax_tree_path}` |

Each backend hashes its native id into the schema's ULID via
`slug_core::derive_ref`, and `slug-mcp` maps that to short aliases (`b1`, `e5`) —
the agent's view is identical on every OS.

**Live events** (`SlugDelta`/`SlugEvent` streaming) are wired on Linux. On Windows
(`Add*EventHandler` COM sinks) and macOS (`AXObserver` notifications) the live
stream is a documented follow-up; **snapshot + invoke — the semantic-first core —
are implemented on all three**. The Windows and macOS backends are compile-verified
in [CI](.github/workflows/ci.yml) on `windows-latest` / `macos-latest`.

## Prerequisites

- Rust 1.77.2+ (`rustup`).
- A graphical desktop session, plus the per-OS setup below.

### Linux (AT-SPI2)

- The **AT-SPI2 accessibility bus** running and the system D-Bus available
  (`libdbus`). Present on most desktops.
- Accessibility enabled so toolkits expose their trees:
  ```sh
  gsettings set org.gnome.desktop.interface toolkit-accessibility true
  ```
  Firefox additionally needs `ACCESSIBILITY_ENABLED=1` (or an active screen
  reader) to publish its tree. `slug-bridge` also calls the AT-SPI
  `set_session_accessibility(true)` hint on connect.
- **Good first targets:** gnome-text-editor, Files (Nautilus), Firefox.

### Windows (UI Automation)

- **No special permissions** — UI Automation is available to any process.
  `slug-bridge` creates the `CUIAutomation` client on connect.
- **Good first targets:** Notepad, File Explorer, Microsoft Edge.

### macOS (Accessibility / AX)

- The app driving Slug (your terminal, or the packaged binary) must be granted
  **Accessibility** permission: **System Settings → Privacy & Security →
  Accessibility**, toggle it on, then restart the process.
- `slug-bridge` checks `AXIsProcessTrusted()` on connect and, if permission is
  missing, returns a typed error with these instructions (it never panics).
- **Good first targets:** TextEdit, Finder, Safari.

## Build

```sh
cargo build --workspace --release
# binaries: target/release/{slug-mcp, slug, slug-agent}  (.exe on Windows)
```

The same command builds on Linux, Windows, and macOS — the correct accessibility
backend is selected automatically per target. CI compiles all three on every push.

Run the tests (unit + MCP protocol integration tests; no desktop needed):

```sh
cargo test --workspace
```

## Run the MCP server

```sh
# stdio transport (what `claude mcp add` uses)
./target/release/slug-mcp --stdio

# streamable HTTP transport (POST JSON-RPC to /mcp)
./target/release/slug-mcp --http 127.0.0.1:7333
```

Logging goes to **stderr** (stdout is the JSON-RPC channel). Tune with
`RUST_LOG`, e.g. `RUST_LOG=slug_bridge=debug`.

### Tools

| Tool | Input | Result |
|------|-------|--------|
| `slug_snapshot` | `{ "scope": "focused" \| "window" \| "desktop" }` | The UI as a Playwright-style YAML tree; each node has a short `[ref=…]`. |
| `slug_invoke` | `{ "ref": "b1", "action": "click", "args"?: "…", "reasoning"?: "…" }` | Performs `activate`/`click`/`press`, `focus`, `set_text`, `set_value`, or any named AT-SPI action. |
| `slug_key` | `{ "keys": "cmd+s", "mode"?: "chord"\|"text", "ref"?: "i1", "reasoning"?: "…" }` | Synthetic keyboard input to the focused app — a key chord or literal text. Drives **any** app, including opaque ones (no accessibility tree), still **no pixels**. See [Controlling any app](#controlling-any-app-synthetic-input). |
| `slug_wait_for` | `{ "event_type"?: "focus_changed" \| …, "timeout_ms": 5000 }` | Blocks until a live UI event occurs or the timeout elapses. |
| `slug_list_apps` | `{}` | Lists running applications exposing an accessibility tree. |
| `slug_agent_start_task` | `{ "description": "…" }` | Starts the `slug-brain` agent on a task (see [Control dashboard](#mcp-native-control-dashboard)). |
| `slug_agent_status` | `{}` | Current task, status, active provider/tier/model, last 20 reasoning/action log lines. |
| `slug_agent_pause` / `slug_agent_resume` / `slug_agent_stop` | `{}` | Pause / resume / stop the running agent task. |

Tool **execution** failures are returned inside the result object
(`isError: true`), never as JSON-RPC protocol errors.

Example snapshot output:

```yaml
- window "Text Editor" [ref=e1]
  - button "Open" [ref=b1]
  - button "New Document" [ref=b2]
  - entry "Document name" [ref=i1] [focused]
    - text "untitled" [ref=e2]
```

## Why `slug_snapshot` is not a screenshot

The name *snapshot* means a **point-in-time read of the semantic document**, in the
database sense — not an image. A `slug_snapshot` returns text (YAML): for each
element its **role, name, state, and `ref`**. There is **no image, pixel buffer,
screen capture, or OCR anywhere in the pipeline** — the bridge reads the OS
accessibility tree (AT-SPI2 / UIA / AX) directly, and the agent acts on `ref`s.

This is the whole thesis: the agent perceives structured meaning, not pixels.
It's cheaper, deterministic, and legible — the [control dashboard](#mcp-native-control-dashboard)
renders the *exact same text* the agent reads, so a human supervising the agent
sees no screenshots either. (This note is mirrored in the `slug_snapshot` MCP
tool description so MCP clients see it too.)

## Controlling any app (synthetic input)

Most apps expose an accessibility tree, so the agent reads them with `slug_snapshot`
and acts with `slug_invoke` on a `ref`. Some apps expose **no** (or only a partial)
tree — games, some Electron/canvas apps — and show up as *opaque*. To drive those
too, `slug_key` injects **synthetic OS keyboard input** into the focused app:

```jsonc
// a key chord (shortcuts, navigation): cmd+s, shift+tab, return, escape, up …
{ "name": "slug_key", "arguments": { "keys": "cmd+s", "mode": "chord" } }
// literal text typed into whatever has focus
{ "name": "slug_key", "arguments": { "keys": "hello world", "mode": "text" } }
// optionally focus an accessible field first, then type
{ "name": "slug_key", "arguments": { "ref": "i1", "keys": "hello", "mode": "text" } }
```

This is still **no pixels and no model tokens**: it posts an OS input event, it does
**not** capture or analyse the screen. It is the lightweight alternative to a
screenshot+vision fallback — it works on any app the OS can route keystrokes to,
without the cost and storage of images.

Implemented on **macOS** (Quartz `CGEvent`; requires Accessibility — and on recent
macOS, Input Monitoring — permission). Linux/Windows synthetic input is a documented
follow-up; on those platforms `slug_key` returns a clear "not yet implemented" tool
error (never a crash). The semantic path (`slug_snapshot`/`slug_invoke`) is the
default and works everywhere.

## Connect Claude Code

Point Claude Code at the built binary over stdio. The command is identical on
every OS — only the binary path differs (`slug-mcp.exe` on Windows):

```sh
# Linux / macOS
claude mcp add slug -- /absolute/path/to/target/release/slug-mcp --stdio

# Windows (PowerShell)
claude mcp add slug -- C:\path\to\target\release\slug-mcp.exe --stdio
```

On macOS, grant the launching process (your terminal, or Claude Code) Accessibility
permission first (see [Prerequisites](#macos-accessibility--ax)); on Windows no
permission is needed; on Linux ensure toolkit accessibility is enabled.

Or, from a checkout, via cargo:

```sh
claude mcp add slug -- cargo run --release -p slug-mcp -- --stdio
```

Or over streamable HTTP (start `slug-mcp --http 127.0.0.1:7333` first):

```sh
claude mcp add --transport http slug http://127.0.0.1:7333/mcp
```

Equivalent `.mcp.json` entry:

```json
{
  "mcpServers": {
    "slug": {
      "command": "/absolute/path/to/target/release/slug-mcp",
      "args": ["--stdio"]
    }
  }
}
```

Then ask Claude to, e.g., *"snapshot the focused window and click the Open
button"* — it will call `slug_snapshot`, read the refs, and call `slug_invoke`.

## CLI

```sh
slug apps                                   # list accessible applications
slug snapshot --scope desktop               # print the YAML semantic tree
slug invoke b1 click                        # click the node shown as [ref=b1]
slug invoke i1 set_text "hello" --reasoning "fill the search box"
slug invoke s1 set_value 0.5                # set a slider/spinner value
slug live --scope window                    # snapshot, then stream live events
```

Ref aliases (`b1`, `e5`) are stable within an unchanged tree; `invoke` takes a
fresh desktop snapshot first to (re)build the alias table.

## Live tests (real desktop)

Live/runtime tests are gated behind the **`live-tests`** feature so default
`cargo test` and CI never try to run them without a desktop.

A cross-platform smoke test (`live_smoke`) connects the active backend, enumerates
apps, snapshots, and renders the agent-facing YAML — run it on any OS:

```sh
# Linux: gsettings set org.gnome.desktop.interface toolkit-accessibility true
# macOS: grant Accessibility permission to your terminal first
cargo test -p slug-bridge --features live-tests --test live_smoke -- --ignored --nocapture
```

A richer Linux end-to-end test drives gnome-text-editor on a live AT-SPI2 bus:

```sh
gsettings set org.gnome.desktop.interface toolkit-accessibility true
cargo test -p slug-bridge --features live-tests --test e2e_gnome_text_editor -- --ignored --nocapture
```

It launches the editor, harvests its tree, asserts it is non-trivial and
non-opaque, renders the agent-facing YAML, and focuses a button.

## Agent (slug-brain)

`slug-brain` turns the MCP tools into an autonomous **observe → reason → act →
verify** loop, and chooses its inference backend from the machine's hardware.

```sh
slug-agent --probe                 # "Can I run it?" hardware report
slug-agent --write-config          # print a default slug.toml
slug-agent "open the Open dialog in the text editor"
slug-agent --backend cloud "summarise the focused window"
```

Each step: the model reads the focused window via `slug_snapshot` (scope
`focused` by default to keep context small — oversized accessibility snapshots
are a well-known failure mode, so they're also truncated), acts via `slug_invoke`
with a `reasoning` slot, and is handed a **fresh post-action snapshot** to verify
expected vs. actual state before continuing.

### Local vs cloud decision

`slug-brain` detects total VRAM (NVIDIA via NVML, AMD via sysfs, Apple via
`system_profiler`), system RAM, and CPU cores, then maps VRAM to a capability
tier. With `selection = "auto"`, the cloud tier uses the Claude API and the local
tiers use Ollama; `local` / `cloud` force a backend.

| Tier | VRAM | Backend | Default model |
|------|------|---------|---------------|
| `TIER_CLOUD` | < 8 GB (or no GPU) | Claude API | `claude-sonnet-4-6` |
| `TIER_LOCAL_SMALL` | 8–11 GB | Ollama | `qwen3:8b` (Q4_K_M) |
| `TIER_LOCAL_STD` | 12–23 GB | Ollama | `qwen3:14b` (Q4_K_M) |
| `TIER_LOCAL_LARGE` | ≥ 24 GB | Ollama | `qwen3:32b` (Q4_K_M) |

This is the task's 4-tier scheme; it consolidates the A–G policy in
[`docs/HARDWARE-TIERING.md`](./docs/HARDWARE-TIERING.md) (Doc 5). The tier →
model/quant mapping and all caps are overridable in `slug.toml`. The cloud model
defaults to `claude-sonnet-4-6` (Doc 5's `cloud_model`); switch to
`claude-opus-4-8` by setting `cloud.model` in `slug.toml`.

### Providers (multi-provider brain)

All providers are driven with **identical tool schemas** behind one `LlmBackend`
trait. Select one in `slug.toml`; keys are read from env vars named in the config
and are **never stored in the file**:

```toml
[brain]
provider = "claude"   # auto | claude | openai | openrouter | gemini | ollama

[providers.claude]     # api_key_env defaults shown; model overridable
api_key_env = "ANTHROPIC_API_KEY"
model = "claude-sonnet-4-6"

[providers.openai]     # also drives any OpenAI-compatible server (vLLM, LM Studio, llama.cpp)
api_key_env = "OPENAI_API_KEY"
base_url = "https://api.openai.com/v1"
model = "gpt-4o"

[providers.openrouter]
api_key_env = "OPENROUTER_API_KEY"
base_url = "https://openrouter.ai/api/v1"
model = "openai/gpt-4o"

[providers.gemini]
api_key_env = "GEMINI_API_KEY"
model = "gemini-2.0-flash"

[providers.ollama]
base_url = "http://127.0.0.1:11434"
model = "qwen3:8b"
```

| Provider | Implementation | Endpoint |
|----------|----------------|----------|
| `claude` | `ClaudeBackend` | Anthropic Messages API (`tool_use` loop) |
| `openai` / `openrouter` / local | `OpenAiCompatibleBackend` | `POST {base_url}/chat/completions` with `tools`; parses `tool_calls` |
| `gemini` | `GeminiBackend` | `generateContent`; tools as `function_declarations`, parses `functionCall` parts |
| `ollama` | `OllamaBackend` | `/api/chat` function-calling |

With `provider = "auto"` the hardware tier decides (cloud → claude, local →
ollama). The `--probe` report recommends a provider too.

### Safety

- **Per-session caps** — token and (cloud) USD cost caps; when hit, the loop
  stops and escalates to a human instead of continuing.
- **Destructive-action confirmation** — `delete` / `send` / `purchase` /
  `submit` / … are pattern-matched (against the action, argument, and the model's
  stated reasoning) and gated behind a confirmation hook (`y/N` on the terminal;
  auto-deny with `--non-interactive`).
- **Structured action log** — every action is logged with its reasoning and
  result, with best-effort **undo** of the last action (e.g. restore prior text,
  re-toggle).

`slug-brain` ships unit tests for the tiering logic (mocked probes) and a scripted
backend that exercises the full loop, the caps, and the destructive gate without
a network or a bus.

## MCP-native control dashboard

A human can supervise and drive the agent through the **same MCP transport** — no
separate protocol. `slug-mcp` exposes agent-control tools (`slug_agent_start_task`,
`slug_agent_status`, `slug_agent_pause`, `slug_agent_resume`, `slug_agent_stop`),
and serves a tiny static dashboard at **`GET /dashboard`** (when run with
`--http`). It is a single self-contained HTML/JS file (no framework) laid out in
three columns — **agent control** (task box, start/pause/resume/stop, live
provider/tier/model badge, connection indicator), **semantic tree** (role-coloured,
clickable `ref` chips, state pills, scope selector), and **action log** (reasoning/
result lines, errors in red). It:

- polls `slug_agent_status` (≈1 s while a task runs, throttled to 3 s when idle, and
  paused entirely when the browser tab is hidden — low latency, low overhead);
- renders the live semantic tree from `slug_snapshot` **as text** — the exact
  hierarchy the agent reads;
- has a text box that calls `slug_agent_start_task`.

Its header states, verbatim: *“Slug never captures pixels. Everything below is the
same structured text the agent reads.”*

```sh
slug-mcp --http 127.0.0.1:7333      # then open http://127.0.0.1:7333/dashboard
```

To avoid a crate cycle (`slug-brain` depends on `slug-mcp`), the controller drives
`slug-agent --jsonl` as a child process and parses its JSON-lines event stream; set
`SLUG_AGENT_BIN` if it isn't installed next to `slug-mcp`.

### HTTP security

The `slug-mcp` HTTP server can read on-screen content and drive the desktop, so it
must never be reachable from a web page in your browser. It binds **loopback only**
(`127.0.0.1`) and additionally validates the request **`Origin` and `Host`** on
`POST /mcp`: any non-local value is rejected with `403`, blocking cross-site
(CSRF) and DNS-rebinding attacks. Local CLI clients (Claude Code, `curl`) send no
`Origin` and a local `Host`, so they pass unchanged.

## Install (macOS)

```sh
./slug-install/install.sh
```

Builds the Rust binaries (a few MB — **no models are downloaded**), writes a
starter `~/.slug/slug.toml` (defaulting to `ollama` if detected, else `claude`),
and registers a launchd agent that runs `slug-mcp --http` at login with the
dashboard. See [`slug-install/README.md`](./slug-install/README.md) (Windows is a
documented manual path for now).

## Milestone-1 adaptations

The canonical spec assumes a Wayland compositor. On the AT-SPI2 path we make two
documented deviations (and stub one security feature):

1. **Refs.** The stable `ref` is still a 128-bit, ULID-shaped, Crockford-Base32
   string (`docs/SEMANTIC-SCHEMA.md` §4.3), but at M1 it is **derived
   deterministically** from the AT-SPI identity `{unique_bus_name}:{accessible_path}`
   rather than minted by a compositor. The ULID is the internal identity; the
   agent only ever sees short **session aliases** (`b1`, `e5`) mapped 1:1 to
   ULIDs in `slug-mcp`. YAML snapshots and all MCP tools use aliases exclusively.
2. **Deltas.** `SlugDelta` / `SlugEvent` frames are produced from AT-SPI2 signals
   (`StateChanged`, `ChildrenChanged`, focus) instead of Wayland frame commits.
   The wire format is exactly the §5.2 format.
3. **Capability token** (§5.4) is **stubbed** — security is Milestone 5
   (see [`docs/RISK-REGISTER.md`](./docs/RISK-REGISTER.md)). The gate is wired
   through `SlugDocument::snapshot` so only the validation body changes later.

## License

MIT OR Apache-2.0.
