# Slug

> Created by **Thibault Dufour**.

**Slug** is a lightweight **desktop app** that lets an AI agent control the
applications on your computer **without ever looking at a screenshot**. Instead of
sending pictures of your screen to a model that guesses where to click, Slug reads
the operating system's built-in **accessibility layer** — the structured text
description of every button, field and menu (the same data screen readers use) —
and lets the agent act on those elements directly. The result is faster, cheaper
and more reliable than vision-based "computer use".

It is **not** an operating system: it installs on top of your existing macOS,
Windows or Linux like any other app (a `.dmg`, an `.exe` installer, or a tarball).
Under the hood it exposes the accessibility layer as an **MCP server**, so you can
connect Claude Code (or any MCP client) and have it read and drive native apps; it
also bundles a multi-provider agent loop (`slug-brain`), a built-in control
dashboard, and synthetic keyboard/mouse/scroll input. (The longer-term design
vision behind this approach is in [`docs/`](./docs).)

Slug runs on all three desktops; only the perception/action layer differs per OS:

| OS | Accessibility source | Permissions |
|----|----------------------|-------------|
| **Linux** | AT-SPI2 over D-Bus | enable toolkit accessibility |
| **Windows** | UI Automation (`IUIAutomation`) | none |
| **macOS** | Accessibility API (`AXUIElement`) | grant Accessibility permission |

Only the platform perception/action layer (`slug-bridge`) differs per OS; the
semantic model (`slug-core`), the MCP server (`slug-mcp`), and the agent
(`slug-brain`) are identical everywhere. See [Platform backends](#platform-backends).

> Slug works through the OS accessibility APIs (AT-SPI2 / UI Automation / AX), not
> a custom compositor. Two documented adaptations of the original design spec apply
> (see [Design adaptations](#milestone-1-adaptations)).

## Workspace layout

| Crate         | Role |
|---------------|------|
| `slug-core`   | The unified semantic document model — a faithful Rust mirror of [`docs/SEMANTIC-SCHEMA.md`](./docs/SEMANTIC-SCHEMA.md): `SlugNode`, `SlugRole`, `SlugState`, `SlugDelta`, stable refs, the document arena, and the Playwright-MCP-style YAML serializer. Depends only on `serde`. |
| `slug-bridge` | The cross-platform accessibility harvester. One `AccessibilityBackend` trait with three implementations — `backend_atspi` (Linux/AT-SPI2), `backend_uia` (Windows/UI Automation), `backend_ax` (macOS/AX) — selected per `cfg(target_os)`. Walks application trees, maps native roles/states to Slug, executes actions, and flags opaque (vision-fallback) apps. |
| `slug-mcp`    | The MCP server: JSON-RPC 2.0 over **stdio** and **streamable HTTP**, exposing the perception/action and agent-control tools (see [Tools](#tools)). This is the session layer that maps internal ULID refs to short agent-facing aliases (`b1`, `e5`), and it enforces the destructive-action [approval gate](#safety) for external clients. |
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

## Install the app

Download the file for your system from the repository's **Releases** page and
install it like any normal app. Full details (provider setup, troubleshooting) are
in **[INSTALL.md](./INSTALL.md)**.

### macOS — `.dmg`
1. Download **`Slug-<ver>-macos-arm64.dmg`** (Apple Silicon) or
   **`…-macos-x86_64.dmg`** (Intel).
2. Open the `.dmg` and **drag `Slug.app` onto the Applications folder**.
3. First launch: right-click **Slug → Open → Open** (the build is unsigned unless
   you supply signing certs — see [INSTALL.md](./INSTALL.md)). Or clear the
   quarantine once in Terminal: `xattr -dr com.apple.quarantine /Applications/Slug.app`.
4. **Double-click Slug** — it starts the background service and opens the dashboard
   at <http://127.0.0.1:7333/dashboard>.
5. Grant permissions once, then relaunch Slug: **System Settings → Privacy &
   Security → Accessibility** (add Slug, toggle on) and, for synthetic
   typing/clicking, **Input Monitoring** too. (Slug never records the screen, so
   Screen Recording is not needed.)

### Windows — `.exe`
1. Download **`SlugSetup-<ver>-windows-x86_64.exe`**.
2. Run it (per-user, **no admin needed**). It installs Slug, starts the background
   service, registers it to run at sign-in, and offers to open the dashboard.
3. Open <http://127.0.0.1:7333/dashboard>. No extra OS permission is required.
   Uninstall from **Apps & features** like any program.

### Linux — tarball
```sh
tar -xzf slug-*-linux-x86_64.tar.gz && cd slug-*-linux-x86_64
gsettings set org.gnome.desktop.interface toolkit-accessibility true   # expose trees
SLUG_AGENT_BIN="$PWD/slug-agent" ./slug-mcp --http 127.0.0.1:7333       # then open the dashboard
```
For synthetic input also `sudo apt install xdotool` (or `ydotool` on Wayland).

> **Connecting an AI** to drive Slug (e.g. Claude Code over MCP) is optional and
> covered in [Connect Claude Code](#connect-claude-code) and [INSTALL.md](./INSTALL.md).

To build the app yourself instead of downloading it:

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

### Build the installers (`.dmg` / `.exe`)

The release artifacts are produced by CI ([`.github/workflows/release.yml`](.github/workflows/release.yml))
— push a tag like `v0.1.0`, or run the workflow manually (**Actions → Release →
Run workflow**) to get the `.dmg`, `SlugSetup.exe`, and tarballs as downloads. To
build them by hand:

```sh
# macOS: app bundle + drag-to-install disk image (run on macOS)
cargo build --release -p slug-mcp -p slug-cli -p slug-brain
bash slug-install/make-macos-app.sh target/release dist     # -> dist/Slug.app
bash slug-install/make-macos-dmg.sh dist/Slug.app dist/Slug.dmg
# (optional signing/notarization: set SIGN_IDENTITY and AC_PROFILE first)

# Windows: one-click installer (run on Windows, needs Inno Setup's ISCC.exe)
cargo build --release -p slug-mcp -p slug-cli -p slug-brain
mkdir slug-install\windows\payload
copy target\release\slug-mcp.exe slug-install\windows\payload\
copy target\release\slug.exe slug-install\windows\payload\
copy target\release\slug-agent.exe slug-install\windows\payload\
iscc /DMyAppVersion=0.1.0 slug-install\windows\slug.iss      # -> dist\SlugSetup.exe
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
| `slug_snapshot` | `{ "scope": "focused" \| "window" \| "desktop", "filter"?: "…", "roles"?: ["button"], "interactive_only"?: bool, "limit"?: 50 }` | The UI as a Playwright-style YAML tree; each node has a short `[ref=…]`. With `filter`/`roles`/`interactive_only` it returns a compact **flat list** of just the matching nodes (each with `ref` + centre `@x,y`) — a server-side "grep" so you don't ship the whole tree. |
| `slug_invoke` | `{ "ref": "b1", "action": "click", "args"?: "…", "reasoning"?: "…" }` | Performs `activate`/`click`/`press`, `focus`, `set_text`, `set_value`, or any named AT-SPI action. |
| `slug_launch` | `{ "name": "Spotify", "uri"?: "spotify:playlist:…" }` | **Launch** an app by name (and optionally open a URI / deep link). Slug otherwise only drives already-running apps. See [Controlling any app](#controlling-any-app-launch-keyboard-mouse). |
| `slug_key` | `{ "keys": "cmd+s", "mode"?: "chord"\|"text", "ref"?: "i1", "reasoning"?: "…" }` | Synthetic keyboard input to the focused app — a key chord or literal text. Drives **any** app, including opaque ones (no accessibility tree), still **no pixels**. |
| `slug_click` | `{ "x": 640, "y": 360, "reasoning"?: "…" }` | Synthetic left mouse click at absolute screen coordinates — click **anywhere**, including opaque apps. No pixels. |
| `slug_scroll` | `{ "x": 640, "y": 360, "dy": -3, "dx"?: 0 }` | Synthetic scroll at coordinates (negative `dy` = down) to reveal off-screen content (grids, lists). No pixels. |
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

**How much cheaper, in tokens?** A reproducible comparison for *playing* chess.com
— counting the whole loop (looking, clicking, reading the reply) —
[`docs/BENCH-vision-vs-slug.md`](./docs/BENCH-vision-vs-slug.md), measured with this
repo's own serializer:

- A vision agent takes **3 screenshots per move** (look + one verify per click) ≈
  **4,296 tokens/move**; Slug plays the move as two near-free clicks and reads the
  reply as text ≈ **416 tokens/move** at the start.
- Over a 40-move game: **vision ≈ 171,840 input tokens vs Slug ≈ 29,459 → ~6×
  fewer** (up to **10× fewer** in the opening).
- **One screenshot (1,365 tokens) costs more than reading the entire 40-move game
  as text (828 tokens).**

(Honest footnote: a *naïve* Slug that re-snapshots the whole page each move would be
worse than vision — which is why Slug filters by default.) Reproduce with
`cargo test -p slug-core --test snapshot_vs_vision -- --nocapture report`.

## Controlling any app (launch, keyboard, mouse)

A full task like *"open Spotify and play my playlist"* uses three capabilities:

1. **Launch** the app — `slug_launch { "name": "Spotify" }` (Slug otherwise only
   drives already-running apps). You can also open a deep link:
   `slug_launch { "name": "Spotify", "uri": "spotify:playlist:37i9dQ…" }`.
2. **Click inside it** — for apps with an accessibility tree (Spotify, Safari,
   Finder, most native and Electron apps) the agent reads `slug_snapshot` and
   clicks elements with `slug_invoke { ref, action: "click" }`. That *is* clicking
   inside the app, on real controls — not just opening it.
3. **Type / shortcuts / click anywhere** — `slug_key` for keyboard, `slug_click`
   for a mouse click at coordinates.

So the flow is: `slug_launch Spotify` → `slug_snapshot focused` → find the playlist
→ `slug_invoke <ref> click` (or `slug_key`/`slug_click`). Each step verifies with a
fresh snapshot.

**Clicking opaque surfaces (canvas/graphics).** On macOS, `slug_invoke … click`
auto-falls back to a synthetic mouse click at the node's centre when the element
exposes geometry but no accessibility press action — so "click" works on canvas
nodes too, with no extra calls. And for those opaque surfaces the snapshot prints
a centre coordinate hint (`@x,y` after the node) so the agent can `slug_click x,y`
a specific spot. Normal controls omit coordinates (clicked by ref — keeps the
snapshot small).

Most apps expose an accessibility tree, so the agent reads them with `slug_snapshot`
and acts with `slug_invoke` on a `ref`. Some apps expose **no** (or only a partial)
tree — games, some canvas apps — and show up as *opaque*. To drive those too,
`slug_key` injects **synthetic OS keyboard input**, and `slug_click` a **synthetic
mouse click** at coordinates, into the focused app:

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

Implemented in-process on **macOS** (Quartz `CGEvent`; needs Accessibility — and on
recent macOS, Input Monitoring — permission) and **Windows** (`SendInput`; no
special permission). On **Linux**, Wayland blocks in-process injection by design,
so Slug shells out to a system input tool when one is installed: **`xdotool`**
(X11/XWayland — full support: keys, text, click, scroll) or **`ydotool`** (Wayland
— text + click). If neither is present, `slug_key`/`slug_click`/`slug_scroll`
return a clear, explained tool error (never a crash); the semantic path
(`slug_snapshot`/`slug_invoke`) needs no injection and remains fully functional on
Linux. The macOS and Windows synthetic paths are compile-verified on their target
triples and need a real-hardware smoke test, as with the rest of the per-OS
backends.

### Snapshot latency

`focused`/`window` snapshots deep-walk only the **frontmost application** (via the
backend's `focused_app`), not the whole desktop, and the session keeps a short
(250 ms) snapshot cache that is invalidated on every action — so the dashboard's
polling and the agent's read-act-verify loop stay responsive without re-harvesting
the entire accessibility tree each time. `desktop` scope still harvests everything.

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
- **Server-side approval for external clients** — the same detection is enforced
  at the MCP server, so a client driving Slug directly (e.g. Claude Code) is gated
  too, not just the in-process agent. Controlled by `SLUG_DESTRUCTIVE`
  (`ask` | `deny` | `allow`, default `ask`): in `ask` mode a destructive
  `slug_invoke` **blocks until a human approves/denies it in the dashboard**
  (`GET /approvals`, `POST /approve`; ~120 s timeout → denied). The shared
  detection lives in `slug-core::is_destructive`.
- **Structured action log** — every action is logged with its reasoning and
  result, with best-effort **undo** of the last action (e.g. restore prior text,
  re-toggle).

`slug-brain` ships unit tests for the tiering logic (mocked probes) and a scripted
backend that exercises the full loop, the caps, and the destructive gate without
a network or a bus.

## MCP-native control dashboard

A human supervises and drives everything from a **built-in dark dashboard** —
served by `slug-mcp` over the same transport, opened as its **own app window**
(Chrome/Edge `--app`) by the packaged app so it feels native, not like a browser
tab. It is one self-contained HTML/JS file (no framework). Top bar shows, at a
glance, **which brain is connected** — *Claude* (cloud) or a *local model* (Ollama)
with the model name — whether an **MCP client is connected**, and the **MCP server
count**. Below:

- **Agent** — brain card, metric tiles (steps / elapsed / tokens / cost), the task
  box, and start / pause / resume / stop. Full control of the agent.
- **Semantic view** — the live tree from `slug_snapshot` rendered **as text**
  (role-coloured, `ref` chips, state pills, `@x,y`), with a filter box and scope
  selector.
- **Tabs** — **Activity** (reasoning/result log, errors in red), **MCP** (every
  connected MCP server with live reachability, and a form to **add your own custom
  MCP servers** — persisted to `~/.slug/mcp_servers.json`), and **Apps** (running
  accessible apps).
- A red **approvals banner** appears on top when a destructive action awaits your
  approval, with Approve / Deny buttons.

Backing HTTP endpoints (all loopback-gated): `GET /info` (app/brain/client),
`GET /mcp-servers` + `POST`/`DELETE` (manage MCP servers), `GET /approvals` +
`POST /approve`, and the agent-control tools over `POST /mcp`. The center panel
states, verbatim: *“Slug never captures pixels … the same structured text the agent
reads.”*

```sh
slug-mcp --http 127.0.0.1:7333      # the packaged app opens it for you, as a window
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

## Install from source (developer / launchd service)

If you built from source instead of downloading the app, this installs Slug as a
background service without the `.app`/`.exe` packaging:

```sh
./slug-install/install.sh           # macOS: ~/.slug + launchd agent at login
./slug-install/install.sh uninstall
```

Builds the Rust binaries (a few MB — **no models are downloaded**), writes a
starter `~/.slug/slug.toml` (defaulting to `ollama` if detected, else `claude`),
and registers a launchd agent that runs `slug-mcp --http` at login with the
dashboard (and sets `SLUG_DESTRUCTIVE=ask`). On Windows use
`powershell -ExecutionPolicy Bypass -File .\slug-install\install.ps1`; on Linux run
the daemon directly or via a systemd `--user` unit. See [INSTALL.md](./INSTALL.md)
and [`slug-install/README.md`](./slug-install/README.md).

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

## Author

**Slug** is created and maintained by **Thibault Dufour**.

## License

MIT OR Apache-2.0.
