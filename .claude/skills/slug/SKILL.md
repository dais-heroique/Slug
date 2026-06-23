---
name: slug
description: >-
  Master reference for the Slug project — an OS-design where an AI agent reads a
  semantic accessibility layer (macOS AX / Windows UI Automation) instead of
  screenshots, exposed as an MCP server. Use this whenever working in the Slug
  workspace: understanding the architecture (slug-core / slug-bridge / slug-mcp /
  slug-cli / slug-brain), building/running/testing, the MCP tools (slug_snapshot,
  slug_invoke, slug_wait_for, slug_list_apps, slug_agent_*), the multi-provider
  brain and slug.toml config, the control dashboard, the macOS/Windows
  installer, granting accessibility permissions, or debugging "permission denied"
  / "not connected" errors. Read this before answering Slug questions or editing
  Slug code.
---

# Slug — complete operator & developer reference

**Slug** is an OS design whose primary user is an AI agent: instead of perceiving
the screen through screenshots, the agent reads a mandatory, OS-wide **semantic UI
layer** — a typed, delta-compressed representation of every widget. The repo
implements a **semantic bus** that harvests the OS accessibility tree and exposes
it as an **MCP server**, so any MCP client (Claude Code, etc.) can read and drive
native apps with **no pixels**.

> Core thesis, repeated everywhere in the codebase: **`slug_snapshot` is NOT a
> screenshot.** It is a point-in-time read of the semantic document (role, name,
> state, `ref` per element) as text/YAML — like a database snapshot. There is no
> image, pixel buffer, screen capture, or OCR anywhere in the pipeline.

When changing code, **never touch `slug-core`'s `SlugNode` / `SlugDelta` schema**
unless the task explicitly says so — it is the stable contract every backend and
the MCP server depend on.

---

## 1. Workspace layout (Rust, edition 2021, MSRV 1.77.2)

Crates live under `crates/`. The whole thing is one cargo workspace.

| Crate | Role |
|-------|------|
| `slug-core` | Unified semantic document model: `SlugNode`, `SlugRole`, `SlugState`, `SlugDelta`, stable refs (ULID-shaped, Crockford-Base32), the document arena, and a Playwright-MCP-style YAML serializer. Depends only on `serde`. **Schema is the contract — keep it stable.** |
| `slug-bridge` | Accessibility harvester. One `AccessibilityBackend` trait, a backend per shipped OS selected by `cfg(target_os)`: `backend_ax` (macOS/AXUIElement) and `backend_uia` (Windows/UI Automation). (An experimental `backend_atspi` for Linux/AT-SPI2 also exists in-tree but is not a shipped/supported target.) Walks app trees, maps native roles/states to Slug, executes actions, flags opaque apps. |
| `slug-mcp` | The MCP server: JSON-RPC 2.0 over **stdio** and **streamable HTTP**. Session layer mapping internal ULID refs → short agent-facing aliases (`b1`, `e5`). Hosts the control **dashboard** and the **AgentController**. |
| `slug-cli` | The `slug` binary for driving the bus by hand (apps, snapshot, invoke, live). |
| `slug-brain` | The hybrid agentic loop (`slug-agent`): observe → reason → act → verify. **Multi-provider** (Claude / OpenAI / OpenRouter / any OpenAI-compatible server / Gemini / Ollama). Depends on `slug-mcp`. |

Data flow:
```
agent ──MCP──► slug-mcp ──► slug-bridge ──UI Automation / AX──► applications
                  │              │
                  └─ slug-core (SlugNode / SlugDocument / SlugDelta) ─┘
```

Key architectural fact: **`slug-brain` depends on `slug-mcp`**, so `slug-mcp`
cannot link `slug-brain`. The dashboard's `AgentController` therefore drives
`slug-agent --jsonl` **as a subprocess** and parses its JSON-lines event stream.
This is intentional — do not try to "fix" it by linking the crates (it creates a
cycle).

Design docs are in `docs/` (`SEMANTIC-SCHEMA.md`, `HARDWARE-TIERING.md`,
`RISK-REGISTER.md`, `WAYLAND-PROTOCOL.md`, `INDEX.md`).

---

## 2. Build, test, lint

```sh
cargo build --workspace --release      # binaries: target/release/{slug-mcp, slug, slug-agent} (.exe on Windows)
cargo test --workspace                 # unit + MCP protocol integration tests; no desktop needed
cargo clippy --workspace --all-targets # CI runs this with RUSTFLAGS="-D warnings" — keep it clean
```

The same build command works on macOS and Windows; the correct accessibility
backend is selected automatically per target. CI compiles all three OSes on every
push (`.github/workflows/ci.yml`).

**Cross-compile checks** (no desktop needed):
```sh
rustup target add x86_64-pc-windows-gnu aarch64-apple-darwin
cargo check --target x86_64-pc-windows-gnu -p slug-bridge
cargo check --target aarch64-apple-darwin  -p slug-bridge
```

Logging goes to **stderr** (stdout is the JSON-RPC channel for `--stdio`). Tune
with `RUST_LOG`, e.g. `RUST_LOG=slug_bridge=debug,slug_brain=info`.

---

## 3. The MCP server (`slug-mcp`)

Implements MCP directly (no SDK). Protocol version advertised: `2025-06-18`.
Methods: `initialize`, `tools/list`, `tools/call`, `ping`,
`notifications/initialized`, `notifications/cancelled`.

**Tool execution failures are returned inside the result object (`isError: true`),
never as JSON-RPC protocol errors.** Only malformed requests / unknown methods
produce protocol errors.

Run it:
```sh
./target/release/slug-mcp --stdio              # what `claude mcp add` uses
./target/release/slug-mcp --http 127.0.0.1:7333 # streamable HTTP: POST JSON-RPC to /mcp; dashboard at /dashboard
```

### Tools (9 total)

**Perception / action (always available):**

| Tool | Input | Result |
|------|-------|--------|
| `slug_snapshot` | `{ "scope": "focused" \| "window" \| "desktop", "filter"?: "send", "roles"?: ["button"], "interactive_only"?: true, "limit"?: 50 }` (default scope `window`) | UI as Playwright-style YAML tree; each node has a short `[ref=…]`. Not a screenshot. **With `filter`/`roles`/`interactive_only` it returns a compact FLAT list of just the matching nodes (each with `ref` + centre `@x,y`) — the server-side "grep" fast path that avoids shipping the whole 80k-char tree.** |
| `slug_invoke` | `{ "ref": "b1", "action": "click", "args"?: "…", "reasoning"?: "…" }` | Performs `activate`/`click`/`press`, `focus`, `set_text`, `set_value`, or any named accessibility action (`toggle`, `expand`, `select`, …). `ref` + `action` required. |
| `slug_launch` | `{ "name": "Spotify", "uri"?: "spotify:playlist:…" }` | **Launch** an app by name (+ optional URI/deep link). Slug otherwise only drives running apps. Works without the a11y bus. Cross-platform (open / start / xdg-open). |
| `slug_click` | `{ "x": 640, "y": 360, "reasoning"?: "…" }` | Synthetic left mouse click at absolute screen coords — click anywhere incl. opaque apps. macOS (CGEvent) + Windows (SendInput). |
| `slug_scroll` | `{ "x": 640, "y": 360, "dy": -3, "dx"?: 0 }` | Synthetic scroll at coords (negative dy = down) to reveal off-screen grid/list items. macOS + Windows. |
| `slug_key` | `{ "keys": "cmd+s", "mode"?: "chord"\|"text", "ref"?: "i1", "reasoning"?: "…" }` | Synthetic OS keyboard input to the focused app — key chord or literal text. Drives **any** app incl. opaque ones (no tree), still no pixels/tokens. macOS (CGEvent) + Windows (SendInput). Optional `ref` focuses a node first. |
| `slug_wait_for` | `{ "event_type"?: "node_created"\|"node_destroyed"\|"node_updated"\|"focus_changed"\|"any", "timeout_ms": 5000 }` | Blocks until a live UI event or timeout. |
| `slug_list_apps` | `{}` | Running apps exposing an accessibility tree. |

**Agent control (only when a controller is attached, i.e. the `slug-mcp` daemon —
the in-process `slug-brain` path passes `None` and these return "agent control is
not available on this transport"):**

| Tool | Input | Result |
|------|-------|--------|
| `slug_agent_start_task` | `{ "description": "…" }` | Spawns `slug-agent --jsonl <desc>`; errors if one is already running. |
| `slug_agent_status` | `{}` | `status` (idle/running/paused/done/stopped), `paused`, `task`, `provider`, `tier`, `model`, and last 20 log lines. |
| `slug_agent_pause` | `{}` | Suspends the process (`kill -STOP`). |
| `slug_agent_resume` | `{}` | Resumes (`kill -CONT`). |
| `slug_agent_stop` | `{}` | Kills and clears the task. |

Example snapshot output:
```yaml
- window "Text Editor" [ref=e1]
  - button "Open" [ref=b1]
  - button "New Document" [ref=b2]
  - entry "Document name" [ref=i1] [focused]
    - text "untitled" [ref=e2]
```

### Connect Claude Code
```sh
# macOS
claude mcp add slug -- /absolute/path/to/target/release/slug-mcp --stdio
# Windows (PowerShell)
claude mcp add slug -- C:\path\to\target\release\slug-mcp.exe --stdio
# from a checkout via cargo
claude mcp add slug -- cargo run --release -p slug-mcp -- --stdio
# over HTTP (start `slug-mcp --http 127.0.0.1:7333` first)
claude mcp add --transport http slug http://127.0.0.1:7333/mcp
```
`.mcp.json` equivalent:
```json
{ "mcpServers": { "slug": { "command": "/abs/path/target/release/slug-mcp", "args": ["--stdio"] } } }
```

### Field-tested driving rules (real Mac runs — Safari, Gmail, Amazon, Chess.com)

Come from real app runs; these override idealized guidance where they conflict.
Full version in `SLUG-AGENT-GUIDE.md` §4b.

1. **Snapshots get huge — filter server-side, don't pull the whole tree.** Pass
   `filter` (substring), `roles` (e.g. `["button"]`, `["entry","combo_box"]`,
   `["static_text"]`) and/or `interactive_only:true` to `slug_snapshot`; you get a
   compact flat list of just the matches, each with `[ref=…]` AND `@x,y`. The grep
   now runs inside the server — this is the #1 speed win. (Client-side `grep` on a
   saved file is only for the raw curl workflow.)
2. **`slug_wait_for` times out often** — skip it; immediately
   `slug_snapshot {scope:"focused"}` after every action.
3. **`slug_launch … uri=`** straight onto the target state (Gmail compose
   `?view=cm&fs=1`, pre-encoded Amazon search, a file/deep-link).
4. **Fill forms in order:** `set_text` every field first, `click` submit last.
5. **`scope:"focused"` for modals/sheets** — smaller, exact.
6. **Refs are per-snapshot** — re-snapshot/re-grep after every action; never
   reuse an old ref.
7. **Off-screen = negative X.** Elements in carousels appear as `@-500,479` etc.
   Ignore anything with X < 0; scroll first if you need it.
8. **Canvas apps (chess.com, maps)** have no accessible nodes — use `slug_click`
   with screen coordinates. Chess.com grid: cols a–h = 352–1052 (step 100),
   rows 1–8 = 950–250 (step -100). Move e2→e4: click (752,850) then (752,650).
   Read moves with `slug_snapshot {roles:["static_text"], limit:200}` (tiny), never
   the full board — it's a canvas with no nodes anyway.
9. **AX -25202 fallback** — if `slug_invoke` fails with that code, use
   `slug_click {x, y}` at the `@x,y` coords (a filtered snapshot prints them on
   every line).
10. **Verify with a filtered snapshot, not the full tree:** Amazon →
    `{filter:"items in basket"}`; chess → `{roles:["static_text"],limit:200}`;
    form saved → `{filter:"saved"}` or the field's new state.

**Fast paths:**
- Chess blitz: per move just `slug_click from` then `slug_click to`; only read the
  reply with `slug_snapshot {roles:["static_text"],limit:200}`. No full snapshots.
- Amazon: `slug_launch … uri=amazon.fr/s?k=PRODUIT` → `slug_snapshot {roles:["button"],filter:"basket"}` → invoke the row's ref.
- Gmail compose: launch `?view=cm&fs=1` → `slug_snapshot {roles:["entry","combo_box"]}` → set_text To/Subject/Body → `{roles:["button"],filter:"send"}` → click.

---

## 4. The CLI (`slug`)

Reuses the same session layer as the MCP server, so refs/snapshots behave
identically to what the agent sees.

```sh
slug apps                                   # list accessible applications
slug snapshot --scope desktop               # print the YAML tree (default scope: desktop)
slug invoke b1 click                        # click the node shown as [ref=b1]
slug invoke i1 set_text "hello" --reasoning "fill the search box"
slug invoke s1 set_value 0.5                # set a slider/spinner value
slug live --scope window                    # snapshot, then stream live events until Ctrl-C
```

Ref aliases (`b1`, `e5`) are stable within an unchanged tree; `invoke` takes a
fresh **desktop** snapshot first to (re)build the alias table, then acts.

---

## 5. The agent (`slug-brain` / `slug-agent`)

Turns the MCP tools into an autonomous observe → reason → act → verify loop, and
chooses its inference backend from hardware (or explicit config).

```sh
slug-agent --probe                          # "Can I run it?" hardware report + recommended provider
slug-agent --write-config                   # print a default slug.toml to stdout
slug-agent "open the Open dialog in the text editor"
slug-agent --backend cloud "summarise the focused window"   # force auto|local|cloud
slug-agent --config /path/to/slug.toml "..."
slug-agent --non-interactive "..."          # auto-deny destructive confirmations
slug-agent --jsonl "..."                    # stream status/step/final as JSON-lines (used by the dashboard controller); implies --non-interactive
```

Per step: model reads the focused window via `slug_snapshot` (scope `focused` by
default to keep context small — oversized snapshots are truncated), acts via
`slug_invoke` with a `reasoning` slot, and is handed a **fresh post-action
snapshot** to verify expected vs. actual before continuing.

### JSONL event kinds (`--jsonl`)
- `{"kind":"status","provider","tier","model"}`
- `{"kind":"task","description"}`
- `{"kind":"step","step","reasoning","tool","args","result","is_error"}`
- `{"kind":"final","answer","step"}`

### Hardware tiers (local vs cloud)
Detects total VRAM (NVIDIA/NVML, AMD/sysfs, Apple/`system_profiler`), RAM, CPU
cores, maps VRAM → tier. With `selection = "auto"`: cloud → Claude, local → Ollama.

| Tier | VRAM | Backend | Default model |
|------|------|---------|---------------|
| `TIER_CLOUD` | < 8 GB (or no GPU) | Claude API | `claude-sonnet-4-6` |
| `TIER_LOCAL_SMALL` | 8–11 GB | Ollama | `qwen3:8b` (Q4_K_M) |
| `TIER_LOCAL_STD` | 12–23 GB | Ollama | `qwen3:14b` (Q4_K_M) |
| `TIER_LOCAL_LARGE` | ≥ 24 GB | Ollama | `qwen3:32b` (Q4_K_M) |

### Safety
Destructive-action detection lives in `slug-core` (`is_destructive`, shared so
there's no crate cycle); the keyword list covers delete/send/buy/submit/…
- **Agent loop (`slug-brain/src/safety.rs`)** — per-session caps (token + cloud
  USD cost; when hit the loop stops and escalates, exit code 1); destructive
  actions gated behind a `y/N` `ConfirmHook` (auto-deny with `--non-interactive`);
  a structured action log with best-effort **undo** of the last action.
- **MCP server (`slug-mcp/src/approval.rs`)** — enforced for **external** clients
  (which never hit the agent's hook). When a controller is attached, a destructive
  `slug_invoke` is gated by `SLUG_DESTRUCTIVE`: `ask` (default) blocks until a
  human approves/denies in the **dashboard** (`GET /approvals`, `POST /approve`;
  120 s timeout → denied), `deny` refuses outright, `allow` permits. The launchd
  plist sets `ask`. This closes the hole where any MCP client driving Slug
  directly could run irreversible actions unsupervised.

---

## 6. Multi-provider brain & `slug.toml`

All providers use **identical tool schemas** behind one `LlmBackend` trait.
`ClaudeBackend` and `OllamaBackend` are the originals; `OpenAiCompatibleBackend`
(one impl for OpenAI, OpenRouter, and any OpenAI-compatible local server) and
`GeminiBackend` were added in M2.5. **API keys are read from env vars named in the
config and are NEVER stored in the file.**

```toml
[brain]
provider = "claude"   # auto | claude | openai | openrouter | gemini | ollama

[providers.claude]
api_key_env = "ANTHROPIC_API_KEY"
model = "claude-sonnet-4-6"          # switch to claude-opus-4-8 for the strongest

[providers.openai]                   # also drives vLLM / LM Studio / llama.cpp (OpenAI-compatible)
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

[providers.ollama]                   # keyless
base_url = "http://127.0.0.1:11434"
model = "qwen3:8b"

# Safety / caps (defaults shown)
[caps]
max_tokens_per_session = 200000      # 0 = unlimited
max_cost_usd = 1.0                    # cloud only; 0 = unlimited
max_steps = 25

[safety]
confirm_destructive = true
```

| Provider | Impl | Endpoint |
|----------|------|----------|
| `claude` | `ClaudeBackend` | Anthropic Messages API (`tool_use` loop) |
| `openai`/`openrouter`/local | `OpenAiCompatibleBackend` | `POST {base_url}/chat/completions` with `tools`; parses `tool_calls` (arguments are a JSON **string**) |
| `gemini` | `GeminiBackend` | `generateContent`; tools as `function_declarations`, parses `functionCall` parts (no call ids → synthesizes `gemini-{i}-{name}`) |
| `ollama` | `OllamaBackend` | `/api/chat` function-calling |

Config gotcha: a **partial** `[providers.X]` block resets sibling fields to their
serde defaults (empty). `Config::resolved_provider(p)` fills empty
`api_key_env`/`base_url` from `provider_defaults(p)` — always go through it when
constructing a backend, never read the raw block.

With `provider = "auto"`, the hardware tier decides (cloud → claude, local →
ollama); `slug-agent --probe` also recommends a provider.

---

## 7. Control dashboard

A human supervises/drives the agent through the **same MCP transport** — no
separate protocol. Served by axum at **`GET /dashboard`** when run with `--http`.
Plain HTML/JS SPA (`crates/slug-mcp/src/dashboard.html`, `include_str!`'d), no
framework. It:
- polls `slug_agent_status` every second (task, provider/tier/model, last 20 lines);
- renders the live semantic tree from `slug_snapshot` **as text**;
- has a text box that calls `slug_agent_start_task`.

Header text, **verbatim**: *“Slug never captures pixels. Everything below is the
same structured text the agent reads.”*

```sh
slug-mcp --http 127.0.0.1:7333    # then open http://127.0.0.1:7333/dashboard
```

The controller locates `slug-agent` via `$SLUG_AGENT_BIN` → sibling of the current
exe → PATH. Set `$SLUG_CONFIG` to point runs at a specific `slug.toml`.

---

## 8. Install & run on each OS

### Quick local dev (any OS, from a checkout)
```sh
cargo build --workspace --release
SLUG_AGENT_BIN=$PWD/target/release/slug-agent ./target/release/slug-mcp --http 127.0.0.1:7333
# open http://127.0.0.1:7333/dashboard
```

### macOS installer (verified) — `slug-install/install.sh`
```sh
./slug-install/install.sh            # build + install ~/.slug + load launchd agent
./slug-install/install.sh uninstall  # unload + remove ~/.slug
```
What it does: builds the 3 binaries (**a few MB; never downloads/bundles an Ollama
model**) → `~/.slug/bin`; writes `~/.slug/slug.toml` (ollama if `ollama list`
succeeds, else claude with `ANTHROPIC_API_KEY` placeholder; backs up any existing
file); registers launchd agent `~/Library/LaunchAgents/org.slug.daemon.plist`
running `slug-mcp --http 127.0.0.1:7333` at login (`RunAtLoad`/`KeepAlive`),
logging to `~/Library/Logs/slug/`, with env `SLUG_AGENT_BIN`/`SLUG_CONFIG`/`RUST_LOG`.

Restart the daemon after a code change / permission grant:
```sh
launchctl kickstart -k gui/$(id -u)/org.slug.daemon
```

### Windows
One-click `SlugSetup.exe` installer, or `install.ps1` (PowerShell + Task
Scheduler) — see `slug-install/README.md`. Dashboard at `http://127.0.0.1:7333/dashboard`.

---

## 9. Permissions per OS (and the #1 gotcha)

| OS | Accessibility source | Permission |
|----|----------------------|------------|
| macOS | AXUIElement | grant **Accessibility** to the binary that calls `AXIsProcessTrusted()` |
| Windows | UI Automation | none |

**macOS gotcha (most common support issue):** TCC ties Accessibility permission to
the *specific process/binary* that calls the AX API. There are typically **two
distinct `slug-mcp` processes**:
1. The one launched **from a terminal** (`slug-mcp --stdio`, used by Claude Code) →
   permission must be granted to the **Terminal app**.
2. The **launchd daemon** (`~/.slug/bin/slug-mcp`, serves the dashboard at :7333) →
   permission must be granted to **`~/.slug/bin/slug-mcp` itself**, NOT the terminal.

So if the **dashboard** shows `AXIsProcessTrusted() returned false` /
"accessibility permission denied", granting the Terminal does nothing — you must
add the daemon binary. In **System Settings → Privacy & Security → Accessibility**,
Unix CLI binaries appear greyed-out in the `+` file picker; add them by either:
- **drag-and-drop** the binary from Finder (`⌘⇧G` → `~/.slug/bin`) into the list, or
- click `+`, then `⌘⇧G`, type `~/.slug/bin`, and select `slug-mcp`.

Then ensure its toggle is **on** and restart the daemon
(`launchctl kickstart -k gui/$(id -u)/org.slug.daemon`).

`slug-bridge` checks `AXIsProcessTrusted()` on connect and returns a typed error
with these instructions — it never panics.

---

## 10. Live tests (real desktop)

Gated behind the `live-tests` feature so default `cargo test` / CI never run them
without a desktop.

```sh
# Smoke test: connect backend, enumerate, snapshot, render YAML
cargo test -p slug-bridge --features live-tests --test live_smoke -- --ignored --nocapture
```

Good first targets: macOS — TextEdit, Finder, Safari; Windows — Notepad, File
Explorer, Edge.

---

## 11. Conventions when contributing

- Develop on the milestone feature branch; commit with clear messages.
- Commit message footer:
  ```
  Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
  Claude-Session: <session url>
  ```
- PR body footer: the 🤖 Generated line + session URL.
- **Never** put a model id in commits/PRs/code/docs.
- Push with `git push -u origin <branch>`, retry on network errors with
  exponential backoff (2s/4s/8s/16s).
- Keep all existing tests passing; clippy clean under `-D warnings`.
- Design notes: refs are derived deterministically from the native accessibility
  identity (AX element path on macOS, UIA `RuntimeId` on Windows); deltas/events use
  a stable wire format independent of the producing OS.
