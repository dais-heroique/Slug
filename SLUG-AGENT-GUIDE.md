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
{ "scope": "focused" | "window" | "desktop",      // default: "window"
  "filter": "basket",                              // optional: substring on names
  "roles": ["button", "entry"],                    // optional: keep only these roles
  "interactive_only": true,                        // optional: drop static text/containers
  "limit": 50 }                                    // optional: cap (default 50)
```
- `focused` / `window` → the focused top-level window (small, fast — your default).
- `desktop` → every running app (use only to locate an app or its window).

**The fast path — filter server-side, don't pull the whole tree.** A real web
page is tens of thousands of characters; reading it all is what makes you slow.
When you're looking for *a specific control*, pass `filter` / `roles` /
`interactive_only`. The snapshot then returns a **compact flat list of only the
matching nodes**, each already carrying its `[ref=…]` **and** a centre `@x,y`:

```
- button "Add to Basket" [ref=b7] @812,540
- entry "Search" [ref=i1] [focused] @640,80
# … 3 more matched; raise 'limit' or refine 'filter'/'roles' …
```

That single line is everything you need to act — `slug_invoke b7 click`, or
`slug_click 812,540` if invoke fails. This replaces the old "snapshot the whole
tree, then grep it" loop: the grep now happens **inside** the server.

Common filters:
- a button by label → `{ "roles": ["button"], "filter": "send" }`
- all text fields → `{ "roles": ["entry", "combo_box", "entry_search", "entry_multiline"] }`
- just the actionable controls → `{ "interactive_only": true }`
- read a move list / labels → `{ "roles": ["static_text"], "limit": 200 }`

Omit all of them only when you genuinely need the *structure* (hierarchy) of the
window. `roles` values are the lower-case role names exactly as printed in a
snapshot (`button`, `entry`, `link`, `heading`, `static_text`, `combo_box`, …).

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

### `slug_launch` — open an app first
```json
{ "name": "Spotify", "uri": "spotify:playlist:37i9dQ…" }
```
Launch an app by name before driving it (Slug only controls running apps). `uri`
optionally opens a deep link / file with it. Works even without the a11y bus.
Pattern for "open Spotify and play my playlist": `slug_launch {name:"Spotify"}` →
`slug_wait_for` / `slug_snapshot focused` → `slug_invoke <playlist-ref> click`.

### `slug_click` — mouse click anywhere
```json
{ "x": 640, "y": 360 }
```
Synthetic left click at absolute screen coordinates — for clicking where there is
no accessible node (opaque apps, canvas). When a node IS accessible, prefer
`slug_invoke <ref> click` (more robust). macOS + Windows; Linux via xdotool/ydotool.

### `slug_scroll` — reveal off-screen content
```json
{ "x": 640, "y": 360, "dy": -3 }
```
Scroll at a point (negative `dy` scrolls down, positive up; optional `dx`). Use it
when a target you expect (a grid tile, a list row) isn't in the snapshot: scroll
over the relevant area, then re-`slug_snapshot`. This is the fix for "the item is
there but off-screen" (e.g. a Canva design type not yet visible). macOS + Windows.

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
(Implemented in-process on macOS + Windows. On Linux, Wayland blocks in-process
injection, so Slug shells out to **`xdotool`** (X11/XWayland — full support) or
**`ydotool`** (Wayland — text + click); if neither is installed it returns a clear
error telling you to install one or use the semantic path. `cmd` maps to `ctrl`
on Linux.)

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

## 4b. Field-tested rules (from real Mac runs — Safari, Gmail, …)

These come from driving real apps and override the defaults above when they
conflict:

1. **A snapshot can be huge — filter it server-side instead of reading it whole.**
   Pass `filter` / `roles` / `interactive_only` to `slug_snapshot` and you get back
   a tiny flat list of just the matching nodes (with `ref` + `@x,y`). This is the
   single biggest speed win — the grep now runs inside the server, so you never pay
   to transfer or read 80k characters:
   ```json
   { "scope": "focused", "roles": ["button"], "filter": "send" }     // a button
   { "scope": "focused", "roles": ["entry", "combo_box", "entry_search"] }  // fields
   { "scope": "focused", "roles": ["heading", "link"], "filter": "inbox" }  // titles/links
   ```
   (Only fall back to client-side `grep` on a saved file if you're in the raw
   curl/HTTP workflow and already dumped the full tree.)

2. **Don't trust `slug_wait_for` to land** — it times out more often than not on
   real apps. After an action, **just call `slug_snapshot {scope:"focused"}`
   right away** instead of waiting in a loop. Use `wait_for` only as a short,
   optional nicety, never as a gate you depend on.

3. **Open apps straight onto the right URL/state with `slug_launch … uri=`** to
   skip navigation clicks — e.g. Gmail compose
   `https://mail.google.com/mail/?view=cm&fs=1`, Amazon onto an already-encoded
   search URL, or a file / deep-link for a native app.

4. **Find refs with the snapshot filter, not by eyeballing the tree** — see rule 1.
   `{ roles: [...], filter: "..." }` returns exactly the candidate nodes.

5. **Fill forms in a fixed order:** `slug_invoke set_text` on **every** field
   first, *then* `click` the submit/save button last. Don't submit between fields.

6. **`scope:"focused"` beats `window` when a modal/sheet is open** (e.g. the
   Gmail Compose window) — it's smaller and targets exactly the active surface.

7. **Refs are per-snapshot — never reuse old ones.** After any action, re-snapshot
   (or re-grep the latest saved file) and pull fresh refs. A ref from a previous
   snapshot is very likely stale.

8. **Off-screen elements have negative X — ignore them.** In carousels and
   horizontally-scrolled containers, nodes appear with coordinates like `@-500,479`.
   Anything with X < 0 is off-screen. Only `slug_click` nodes where X ≥ 0 (or scroll
   first to bring them on-screen).

9. **Canvas apps (chess.com, maps, editors) have no accessible nodes — use `slug_click`
   with coordinates.** For chess.com `/play/computer`, the grid maps as:
   - columns: a=352 b=452 c=552 d=652 e=752 f=852 g=952 h=1052
   - rows:    1=950  2=850  3=750  4=650  5=550  6=450  7=350  8=250
   - move e2→e4: `slug_click {x:752, y:850}` then `slug_click {x:752, y:650}`
   - read played moves with a **filtered** snapshot, not the whole page:
     `slug_snapshot { roles: ["static_text"], limit: 200 }`.

10. **If `slug_invoke` fails with error AX -25202** (action not supported), fall back to
    `slug_click` with the `@X,Y` coordinates printed in the snapshot for that node.
    (A filtered snapshot already prints `@x,y` on every line — use it directly.)

11. **Verify an action worked** with a *filtered* snapshot of the expected result —
    don't re-read the whole page:
    - Amazon add-to-cart: `slug_snapshot { filter: "items in basket" }` → read the counter.
    - Chess move played: `slug_snapshot { roles: ["static_text"], limit: 200 }` → read notation.
    - Form saved: `slug_snapshot { filter: "saved" }` or check the field's new state.

### App-specific fast paths

**Chess blitz (chess.com) — minimise per-move latency**
```
Per move (≈2 calls, no full snapshot):
1. slug_click {x: <from_x>, y: <from_y>}     # pick up your piece
2. slug_click {x: <to_x>,   y: <to_y>}       # drop it (coords from the grid in rule 9)
3. Only when you must read the engine's reply:
   slug_snapshot { roles: ["static_text"], limit: 200 }   # tiny — just the move list
Never snapshot the full board between your own moves; the board is a canvas with
no nodes anyway, so a full snapshot tells you nothing a filtered one doesn't.
```

**Amazon**
```
1. slug_launch Safari uri=https://www.amazon.fr/s?k=PRODUIT+ENCODE
2. slug_snapshot { roles: ["button"], filter: "basket" }   # flat list, each with ref + @x,y
3. Pick the button on the row you want (ignore any with X < 0 — off-screen)
4. slug_invoke ref=bXXX action=click  (repeat for each item)
```

**Gmail — compose and send/draft**
```
1. slug_launch Safari uri=https://mail.google.com/mail/?view=cm&fs=1   # opens compose directly
2. slug_snapshot { roles: ["entry", "combo_box", "entry_multiline"] }  # To / Subject / Body fields
3. slug_invoke set_text on To, Subject, Body  (in that order)
4. slug_snapshot { roles: ["button"], filter: "send" } → slug_invoke click
   (or { filter: "save" } / Esc to leave a draft)
```

---

## 5. Safety you must respect

- **Destructive actions** (`delete`, `send`, `purchase`, `submit`, `discard`, …)
  are gated **at the Slug server**, for every client — including you when you
  drive Slug directly over MCP. By default (`SLUG_DESTRUCTIVE=ask`) a destructive
  `slug_invoke` **blocks until a human approves it in the dashboard**; your tool
  call simply waits, then returns success if approved or an `isError` "denied…" if
  refused or timed out (~120 s). With `SLUG_DESTRUCTIVE=deny` they're refused
  outright; with `allow`, permitted. If a destructive action is denied, **stop and
  report** — don't try to route around the gate (e.g. don't replay it as a raw
  `slug_click`/`slug_key`).
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
