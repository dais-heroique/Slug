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
  `disabled`, `readonly`, `required`, `invalid`.
  - `[disabled]` means the OS toolkit flagged the node inactive. `click`, `toggle`,
    `expand` will do nothing — but **`set_text` can still work** via the AX API even
    on a `[disabled]` entry (e.g. TextEdit in "Prevent Editing" mode). Try it; read
    the result text.
  - `[readonly]` on an entry means the user can't type in it, but `set_text` may
    still succeed programmatically. Same rule: try, read result.
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
{ "app": "Notes",                                  // optional: target an app by name
  "scope": "focused" | "window" | "desktop",      // default: "window"
  "filter": "send",                                // optional: substring on names
  "roles": ["button"],                             // optional: exact roles OR a group
  "interactive_only": true,                        // optional: drop static text/containers
  "limit": 1,                                      // optional: cap (default 50)
  "coords": false }                                // optional: add @x,y to every match
```
- **`app` (use this when you drive Slug from a terminal/another window).**
  `scope:"focused"` reads whatever the OS has frontmost — which keeps snapping
  *back to your own client window* between calls. Pass `app:"Safari"` to snapshot a
  specific app regardless of focus. Matched case-insensitively; overrides `scope`.
- `focused` / `window` → the OS-frontmost top-level window (small, fast — but it's
  whatever the OS focused; prefer `app` to be sure you read the right one).
- `desktop` → every running app across all monitors (use to locate an app/window).
- **macOS footgun: a stray click/key near the top of the screen (y < ~24) opens
  the system menu bar** (Apple menu, app menu, clock…), and `scope:"focused"`/
  `"window"` will then dutifully snapshot *that* instead of your app — you get a
  page of unrelated menu items and have wasted a round-trip closing it. `app:"…"`
  is immune to this since it always reads the named app's own tree regardless of
  what the OS currently has focused; prefer it whenever you're repeatedly
  snapshotting one app. If you do need a raw `slug_click`/`slug_key`, avoid y < 24.

**The fast path — filter server-side, don't pull the whole tree.** A real web
page is tens of thousands of characters; reading it all is what makes you slow.
When you're looking for *a specific control*, pass `filter` / `roles` /
`interactive_only`. The snapshot returns a **compact, lean flat list of only the
matching nodes** (just `role "name" [ref] [states]` — no coordinates on normal
controls, so it's as small as possible):

```
- button "Send" [ref=b7]
- button "Send later" [ref=b9]
# … 3 more matched; raise 'limit' or refine 'filter'/'roles' …
```

Then act — `slug_invoke b7 click`. Matches are **ranked: an exact name match comes
first**, so `limit: 1` returns the single control you meant.

Precise filters:
- a button by label → `{ "roles": ["button"], "filter": "send", "limit": 1 }`
- any text field → `{ "roles": ["field"] }`  (group: entries/combos/spinners)
- any actionable control → `{ "roles": ["clickable"] }`  (or `"interactive_only": true`)
- a link / heading → `{ "roles": ["link"] }` / `{ "roles": ["heading"], "filter": "inbox" }`
- read a move list / labels → `{ "roles": ["static_text"], "limit": 200 }`
- need to `slug_click` by coordinate → add `"coords": true` (opaque surfaces always
  print `@x,y`; normal controls only when you ask).

**Role groups** (besides exact role names like `button`, `entry`, `static_text`):
`clickable` = any actionable control, `field`/`input` = text entries/combos/spinners,
`text` = static text/labels/headings, `link`, `heading`, `price`/`money`/`currency` =
any label that *looks like* a currency amount, regardless of its actual role.

Omit all filters only when you genuinely need the *structure* (hierarchy) of the
window. Exact `roles` values are the lower-case role names exactly as printed in a
snapshot (`button`, `entry`, `link`, `heading`, `static_text`, `combo_box`, …).

**A bare `limit` (no `filter`/`roles`) still switches to the compact flat list** —
it does not pull the whole tree first and then cap it. `{ limit: 30 }` alone is
enough to keep a dense page small.

**Both the filtered and the unfiltered path are capped past ~20k characters** —
whichever you use, an oversized result is truncated with a note telling you to
narrow it (lower `limit`, or add `roles`/`filter`/`interactive_only`). Full
unfiltered dumps of a dense page (e.g. an e-commerce search results page) can run
to hundreds of KB, which overflows your own tool-result limit and forces a slow
file-dump-and-grep fallback — the cap exists specifically to prevent that. Don't
try to raise it with a `depth` or `max_chars` argument — neither exists.

**Prices don't reliably match a currency symbol** — Amazon prints "EUR 26.32",
not "$26.32", so `filter: "$"` finds nothing. Use `{ roles: ["price"] }` instead:
it matches currency-shaped text ($19.99, 26,32 €, EUR 26.32, …) by content, in
whatever role the site happens to render it (link, static text, span — doesn't
matter). Combine with a high `limit` to sweep an entire results page in **one
call**: `{ app: "Safari", roles: ["price"], limit: 100, coords: true }` returns
every price on the page with refs and click-fallback coordinates — pair it with
one `{ roles: ["link"], limit: 100 }` call for the product titles instead of
grepping a full-page dump or re-snapshotting per product.

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
| Any other named accessibility action | that name verbatim | as needed |

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
`slug_invoke <ref> click` (more robust). macOS + Windows.

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
{ "keys": "cmd+s", "mode": "chord" | "text", "activate": "TextEdit",
  "ref": "i1", "reasoning": "why" }
```
Synthetic OS keyboard input. This is how you act on apps that show up as opaque
(no accessible tree) — **no pixels, no tokens**, pure event injection.
- `mode: "chord"` (default) → a key combo: `cmd+s`, `shift+tab`, `return`,
  `escape`, `up`/`down`/`left`/`right`, `cmd+shift+z`, function keys `f1`…`f12`.
- `mode: "text"` → type the string literally into the focused app.
- `activate` → **always pass this when Slug is driven from a terminal.**
  Without it the chord/text lands in whatever window is frontmost — your client —
  and the target app is untouched. The tool returns `ok` either way (the OS doesn't
  report where the event went), so a silent miss looks like a success.
- `ref` (optional) → focus that accessible node first, then send input.

> ⚠️ **ALWAYS pass `activate` for keyboard input.** Field testing showed that
> `cmd+a` / `cmd+c` / `cmd+s` without `activate` consistently land in the
> **terminal (or Claude Code window)** not in the target app. The result says `ok`.
> Nothing happens in the app. This is the #1 source of "slug_key seems broken".
>
> ```json
> // ❌ Wrong — chord lands in the terminal
> { "keys": "cmd+s" }
>
> // ✅ Right — chord goes to TextEdit
> { "keys": "cmd+s", "activate": "TextEdit", "reasoning": "save file" }
> ```
>
> For multiple keystrokes, use `slug_sequence` — one atomic call, focus can't
> be stolen between steps:
> ```json
> { "steps": [{"activate":"TextEdit"}, {"key":"cmd+a"}, {"key":"cmd+c"}] }
> ```
> Don't verify a chord worked by reading the clipboard — it may hold *your*
> client's clipboard. Re-snapshot the target app instead.

### `slug_activate` / `slug_sequence` — beat focus theft
```json
{ "app": "Safari", "settle_ms": 150 }                       // slug_activate
{ "steps": [ {"activate":"Safari"}, {"wait_ms":200},
             {"text":"crane"}, {"key":"return"} ] }          // slug_sequence
```
`slug_activate` just brings an app to the front. `slug_sequence` runs an ordered
list of steps with **no return to the client in between**, so focus can't be
stolen. Step kinds: `{activate:"App"}`, `{focus:"ref"}`, `{click:"ref"}`,
`{click_xy:[x,y]}`, `{key:"return"}`, `{text:"hello"}`, `{wait_ms:200}`.
Example — play a Wordle guess in one call: `[{activate:"Safari"},{wait_ms:200},
{text:"crane"},{key:"return"}]`.

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

### `slug_status`
```json
{}
```
One-shot health report printed directly in chat — the dashboard's content as
text, for clients (like Claude Code over stdio) that can't open a browser:
app version, AI brain (provider/model/ready), which transport an MCP client
is connected over, accessibility-bus reachability, pending destructive-action
approvals, and the built-in agent's current task if one is running. Call this
instead of `GET /dashboard` when you just need a status check without
leaving the conversation.

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
- ❌ Giving up on `set_text` just because a node shows `[disabled]`. Try it —
  `set_text` works via AX API even on nodes flagged disabled (e.g. TextEdit in
  Prevent Editing mode). Only `click`/`toggle` are truly blocked on disabled nodes.
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

12. **On macOS, a stray `slug_click`/`slug_key` near the top of the screen
    (y < ~24) can pop the system menu bar** (Apple menu, app menu, clock, …).
    If a `scope:"focused"`/`"window"` snapshot suddenly comes back full of
    unrelated menu items instead of your app, that's almost certainly what
    happened — close the menu (`slug_key {keys:"escape"}`) and re-snapshot.
    `app:"…"` targeting sidesteps this entirely since it reads the named app's
    tree regardless of what the OS currently has focused; prefer it over
    `scope` for any multi-step flow in one app.

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
Always pass `app: "Safari"` on every call in a multi-step Amazon flow (not just
`scope: "focused"`) — it's immune to OS-focus drift, including the system menu
bar popping open after a stray click near screen-top.

**Amazon — compare N products by price/rating in ~2 calls instead of dozens**
```
1. slug_launch Safari uri=https://www.amazon.fr/s?k=PRODUIT+ENCODE
2. slug_snapshot { app: "Safari", roles: ["link"], filter: "PRODUIT", limit: 50 }
   # product titles — each line is a candidate product
3. slug_snapshot { app: "Safari", roles: ["price"], limit: 50, coords: true }
   # every price on the page in one call — matches "$19.99", "26,32 €", "EUR 26.32"
   # all at once, no need to guess a currency symbol
4. Correlate titles ↔ prices by their order/position in the two flat lists (both
   reflect on-page reading order), then slug_invoke click on the chosen product.
```
This replaces reading the whole page and grepping for `$`/`€` by hand — that
literal-symbol approach misses results in non-US currency formats and, on a
dense results page, risks tripping the ~20k-char truncation cap before you even
reach the prices.

**Gmail — compose and send/draft**
```
1. slug_launch Safari uri=https://mail.google.com/mail/?view=cm&fs=1   # opens compose directly
2. slug_snapshot { roles: ["entry", "combo_box", "entry_multiline"] }  # To / Subject / Body fields
3. slug_invoke set_text on To, Subject, Body  (in that order)
4. slug_snapshot { roles: ["button"], filter: "send" } → slug_invoke click
   (or { filter: "save" } / Esc to leave a draft)
```

**Canva (web) — navigate and click on opaque canvas**
```
1. slug_launch Safari uri=https://www.canva.com
2. slug_snapshot { app: "Safari", roles: ["button", "link"], filter: "design" }
   → find CTA ("Start designing for free", "Create a design", etc.)
3. slug_invoke ref=bXXX action=click   # if accessible node
   OR slug_click { x: <x>, y: <y> }   # if canvas zone — read @x,y from coords:true snapshot
4. After page nav, re-snapshot for the next step
Note: Canva requires login for templates. slug_snapshot { roles: ["button", "link"] }
to find the login button and fill credentials with set_text on the entry fields.
```

### Known OS limitations (don't fight these — work around them)
Some macOS apps expose little or no accessibility tree. These are OS facts, not
Slug bugs; the workaround is always the same — drive them with synthetic input
(`slug_key` / `slug_click` / `slug_sequence`) instead of reading nodes.

| App / kind | What you'll see | How to drive it |
|---|---|---|
| **Spotify** | `generic "Spotify"`, no children/coords | Keyboard only: `slug_key {keys:"space", activate:"Spotify"}` (play/pause), `cmd+right` (next). Or `slug_launch Spotify uri=spotify:…` to jump to content. |
| **Notes body** | a `WKWebView` — you can **write** into it but the text isn't read back via AX | Type with `slug_sequence`; verify by other means (don't expect to read the body back). |
| **Chess.app** (native) | no AX tree, no usable AppleScript | Not drivable. Use **chess.com in Safari** instead (canvas board → click by coords, read the move list via `roles:["static_text"]`). |
| **Electron/Chromium apps** (e.g. ChatGPT Atlas, Claude desktop) | labelled `generic` but actually rich | Not opaque — snapshot with `app:"<name>"`; the content is in `entry`/`entry_multiline` nodes. Don't trust the `generic` top label, look inside. |

Rule of thumb: a top-level `generic` with **no children** is genuinely opaque
(keyboard/coords only); a `generic` **with** children is just an unlabelled
container — keep reading into it.

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
- **Permissions (macOS):** Two separate binaries, each needs its own grant in
  **System Settings → Privacy & Security → Accessibility**:
  - `~/.slug/bin/slug-mcp` — the launchd daemon (dashboard + HTTP clients)
  - The binary launched via stdio by Claude Code (shown in `ps aux | grep slug-mcp`)
  After granting, restart the daemon:
  `launchctl kickstart -k gui/$(id -u)/org.slug.daemon && curl http://127.0.0.1:7333/healthz`
  Expected: `ok`. If you still see "permission denied", the wrong binary was added.
- If a snapshot errors with *"permission denied / not connected"*, it's an OS
  permission problem — **not** something fixable with more tool calls. Report it
  and the fix above.

---

### TL;DR
Snapshot `focused` → act on a `ref` → verify with another snapshot. Trust roles
and states, keep refs fresh, fill `reasoning`, respect the destructive gate, and
never reach for pixels. That's the whole job.
