# Slug

**Slug** is a Linux OS design whose primary user is an AI agent: instead of
perceiving the screen through screenshots, the agent reads a mandatory, OS-wide
**semantic UI layer** — a typed, delta-compressed representation of every widget.
See [`docs/`](./docs) for the full design dossier.

This repository implements **Milestone 1**: a *semantic bus* that harvests the
Linux **AT-SPI2** accessibility tree and exposes it as an **MCP server**, so you
can connect Claude Code (or any MCP client) and have it read and drive native
apps like **Firefox** and **gnome-text-editor** — no pixels required.

> Milestone 1 lives on the AT-SPI2 path, not the Wayland compositor. Two
> documented step-1 adaptations of the canonical spec apply (see
> [Milestone-1 adaptations](#milestone-1-adaptations)).

## Workspace layout

| Crate         | Role |
|---------------|------|
| `slug-core`   | The unified semantic document model — a faithful Rust mirror of [`docs/SEMANTIC-SCHEMA.md`](./docs/SEMANTIC-SCHEMA.md): `SlugNode`, `SlugRole`, `SlugState`, `SlugDelta`, stable refs, the document arena, and the Playwright-MCP-style YAML serializer. Depends only on `serde`. |
| `slug-bridge` | The AT-SPI2 harvester (via the [`atspi`](https://crates.io/crates/atspi) crate): connects to the a11y bus, walks application trees, maps AT-SPI roles/states to Slug, executes actions, streams live events, and flags opaque (vision-fallback) apps. |
| `slug-mcp`    | The MCP server: JSON-RPC 2.0 over **stdio** and **streamable HTTP**, exposing four tools. This is the session layer that maps internal ULID refs to short agent-facing aliases (`b1`, `e5`). |
| `slug-cli`    | A `slug` binary for driving the bus by hand (snapshot, list apps, invoke actions, stream live events). |
| `slug-brain`  | A hybrid agentic loop (`slug-agent`) that drives the MCP tools, switching between a local **Ollama** model and the **Anthropic Claude API** based on detected hardware. See [Agent: local vs cloud](#agent-slug-brain). |

```
agent ──MCP──► slug-mcp ──► slug-bridge ──AT-SPI2/D-Bus──► applications
                  │              │
                  └─ slug-core (SlugNode / SlugDocument / SlugDelta) ─┘
```

## Prerequisites

- Rust 1.77.2+ (`rustup`).
- A Linux session with the **AT-SPI2 accessibility bus** running and the system
  D-Bus available (`libdbus`). On most desktops this is already present.
- Accessibility enabled so toolkits expose their trees:
  ```sh
  gsettings set org.gnome.desktop.interface toolkit-accessibility true
  ```
  Firefox additionally needs `ACCESSIBILITY_ENABLED=1` (or an active screen
  reader) to publish its tree. `slug-bridge` also calls the AT-SPI
  `set_session_accessibility(true)` hint on connect.

## Build

```sh
cargo build --workspace --release
# binaries: target/release/slug-mcp  and  target/release/slug
```

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
| `slug_wait_for` | `{ "event_type"?: "focus_changed" \| …, "timeout_ms": 5000 }` | Blocks until a live UI event occurs or the timeout elapses. |
| `slug_list_apps` | `{}` | Lists running applications exposing an accessibility tree. |

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

## Connect Claude Code

Point Claude Code at the built binary over stdio:

```sh
claude mcp add slug -- /absolute/path/to/target/release/slug-mcp --stdio
```

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

## End-to-end test (real desktop)

A gated integration test drives gnome-text-editor on a live bus:

```sh
gsettings set org.gnome.desktop.interface toolkit-accessibility true
cargo test -p slug-bridge --test e2e_gnome_text_editor -- --ignored --nocapture
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

### Backends

Both backends are driven with **identical tool schemas** behind one
`LlmBackend` trait:

- **`ClaudeBackend`** — Anthropic Messages API over raw HTTP (no official Rust
  SDK). Implements the documented tool-use loop: send the `tools` array, and on
  `stop_reason = "tool_use"` execute the `tool_use` blocks and append
  `tool_result` blocks on the next turn.
- **`OllamaBackend`** — Ollama `/api/chat` function-calling, with the same tools
  wrapped in `{type:"function", …}` and tool results returned as `role:"tool"`.

Set the Claude key via the env var named in `slug.toml` (`api_key_env`, default
`ANTHROPIC_API_KEY`); run `ollama serve` and `ollama pull <model>` for local.

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
