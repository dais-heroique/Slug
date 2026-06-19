# SLUG — Operating Manual for the Connected Agent

> Read this once at the start of a Slug session. It contains everything you need
> to drive native apps through the Slug MCP server **fast**, without exploring or
> trial-and-error. Slug exposes the OS accessibility tree as text — **you never
> need screenshots, pixels, or OCR.**

---

## 0. The one rule that saves the most time

**Snapshot `focused` first, act on `ref`s, verify with the post-action snapshot.**
Don't snapshot `desktop` unless you genuinely need to find an app — it's large and
slow. Don't guess coordinates; there are none. Everything is a `ref`.

A correct loop is exactly four moves:

1. `slug_snapshot { scope: "focused" }` → read roles/names/refs.
2. Pick the `ref` you need.
3. `slug_invoke { ref, action, args?, reasoning }`.
4. `slug_snapshot { scope: "focused" }` again → confirm the state changed as expected.

If step 4 doesn't show the expected change, re-read and try a different ref/action
— **do not repeat the same call hoping for a different result.**

---

## 1. What a snapshot looks like

`slug_snapshot` returns a Playwright-style YAML tree. Each line is
`<role> "<name>" [ref=<alias>] [state] [state]...`. Indentation = hierarchy.

```yaml
- window "Text Editor" [ref=e1]
  - button "Open" [ref=b1]
  - button "New Document" [ref=b2]
  - entry "Document name" [ref=i1] [focused]
    - text "untitled" [ref=e2]
  - checkbox "Wrap lines" [ref=c1] [checked]
  - slider "Zoom" [ref=s1]
```

- **`ref` aliases** (`b1`, `e5`, `i1`, …) are short, stable within an *unchanged*
  tree. The letter is a hint (b=button-ish, e=element/window, i=input, c=check,
  s=slider) but **always trust the role, not the letter.**
- **States** you'll see in brackets: `focused`, `checked`, `selected`, `expanded`,
  `disabled`, `readonly`, `required`, `invalid`. A `disabled` node won't respond
  to actions — don't invoke it.
- After any action that changes the tree (opening a dialog, switching windows),
  **the old refs may be invalid.** Re-snapshot to rebuild them.

### Opaque apps
If the snapshot lists an app under `# opaque apps (no/flat accessibility tree —
vision fallback)`, that app exposes nothing useful. You cannot drive it through
Slug. Say so plainly instead of trying.

---

## 2. The complete tool reference

### `slug_snapshot`
```json
{ "scope": "focused" | "window" | "desktop" }   // default: "window"
```
- `focused` / `window` → the focused top-level window (small, fast — your default).
- `desktop` → every running app (use only to locate an app or its window).

### `slug_invoke`
```json
{ "ref": "b1", "action": "click", "args": "optional", "reasoning": "why" }
```
`ref` + `action` are required. **Always fill `reasoning`** — it's logged/audited and
costs you nothing.

**Action vocabulary:**

| Intent | `action` | `args` |
|--------|----------|--------|
| Press a button / menu item / link | `click` (aliases: `activate`, `press`) | — |
| Move keyboard focus to a node | `focus` | — |
| Replace the text of an entry/field | `set_text` | the new string |
| Set a slider/spinner/progress value | `set_value` | a number, e.g. `0.5` |
| Toggle a checkbox/switch | `toggle` | — |
| Expand/collapse a tree item/combo | `expand` | — |
| Select a list/menu option | `select` | — |
| Any other named AT-SPI/UIA action | that name verbatim | as needed |

Result text tells you what happened:
- `ok: <action> on <ref> succeeded` → done.
- `note: <action> on <ref> was dispatched but the toolkit reported no effect` →
  the call went through but nothing changed. Re-snapshot; you probably had a stale
  ref, a disabled node, or the wrong action for that role.

### `slug_key` — drive ANY app, including opaque ones
```json
{ "keys": "cmd+s", "mode": "chord" | "text", "ref": "i1", "reasoning": "why" }
```
Synthetic OS keyboard input to the **focused** app. This is how you act on apps
that show up as opaque (no accessible tree) — and it still uses **no pixels and no
tokens** (it injects an OS event, never a screenshot).
- `mode: "chord"` (default) → a key combo: `cmd+s`, `shift+tab`, `return`,
  `escape`, `up`/`down`/`left`/`right`, `cmd+shift+z`, function keys `f1`…`f12`.
- `mode: "text"` → type the string literally into whatever has focus.
- `ref` (optional) → focus that accessible node first, then send the input.

Pattern for an opaque app: bring it to the front / focus its field if you can,
then `slug_key`. For shortcuts in any app: just `slug_key {keys:"cmd+s"}`.
(Implemented on macOS + Windows; on Linux it returns a clear error — Wayland
blocks synthetic input by design, so use the semantic path there.)

### `slug_wait_for`
```json
{ "event_type": "focus_changed", "timeout_ms": 5000 }
```
`event_type` ∈ `node_created | node_destroyed | node_updated | focus_changed | any`
(omit ⇒ any). Use **after** an action that triggers async UI (a dialog opening, a
page loading) instead of re-snapshotting in a busy loop. Returns the event, or
`timeout: no matching event within <ms>ms`.

### `slug_list_apps`
```json
{}
```
Lists running apps exposing an accessibility tree. Use to discover what's open
before a `desktop` snapshot.

---

## 3. Efficient patterns (copy these)

**Click a named button in the current window**
1. `slug_snapshot {scope:"focused"}` → find `button "Open" [ref=b1]`.
2. `slug_invoke {ref:"b1", action:"click", reasoning:"open the file dialog"}`.
3. `slug_wait_for {event_type:"node_created", timeout_ms:3000}` (dialog appears).
4. `slug_snapshot {scope:"focused"}` → now operate on the dialog.

**Fill a form field**
1. snapshot → find `entry "Email" [ref=i2]`.
2. `slug_invoke {ref:"i2", action:"set_text", args:"me@example.com", reasoning:"enter email"}`.
3. snapshot → confirm the `text` child now reads the value.

**Find and act in an app that isn't focused**
1. `slug_list_apps` → confirm it's running.
2. `slug_snapshot {scope:"desktop"}` → locate its window + the ref you want.
3. Optionally `focus` the window first, then act.

**Toggle a setting**
1. snapshot → find `checkbox "Wrap lines" [ref=c1] [checked]`.
2. `slug_invoke {ref:"c1", action:"toggle", reasoning:"turn off wrap"}`.
3. snapshot → state should flip to no `[checked]`.

---

## 4. Things that waste time — avoid them

- ❌ Snapshotting `desktop` every step. Use `focused`.
- ❌ Re-running the identical `slug_invoke` after a `note: … no effect`. Re-read first.
- ❌ Acting on a `ref` from a snapshot taken **before** the tree changed. Refs go
  stale when windows/dialogs open or close — re-snapshot.
- ❌ Inventing coordinates, key chords, or pixel positions. Slug has none of that.
- ❌ Acting on `disabled` nodes.
- ❌ Long prose between tool calls. Read, act, verify; keep reasoning to one line.

---

## 5. Safety you must respect

- **Destructive actions** (`delete`, `send`, `purchase`, `submit`, `discard`, …)
  are gated. In interactive mode a human is asked `y/N`; with `--non-interactive`
  they are **auto-denied**. If a destructive action is denied, **stop and report**
  — don't try to route around the gate.
- **Per-session caps** (tokens, USD cost, max steps) can halt the loop and escalate
  to a human. If you're escalated, summarize what you did and what's left.
- Every action is logged with its `reasoning`. Write reasoning that would make
  sense to a human auditor reading the log later.

---

## 6. Quick environment facts

- **Transports:** stdio (Claude Code) or HTTP (`/mcp`); dashboard at
  `http://127.0.0.1:7333/dashboard` when running HTTP.
- **Permissions:** Linux needs toolkit-accessibility on; Windows needs none; macOS
  needs Accessibility granted to the *specific binary* running Slug (the launchd
  daemon `~/.slug/bin/slug-mcp` for the dashboard, or your terminal for stdio).
- If a snapshot errors with *"permission denied / AXIsProcessTrusted returned
  false / not connected"*, it's an OS permission problem, **not** something you can
  fix with more tool calls — report it and the fix (grant Accessibility, restart).

---

### TL;DR
Snapshot `focused` → act on a `ref` → verify with another snapshot. Trust roles
and states, keep refs fresh, fill `reasoning`, respect the destructive gate, and
never reach for pixels. That's the whole job.
