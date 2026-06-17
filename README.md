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
