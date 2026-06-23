# Slug Validation Test — Run this prompt when connected to Slug via MCP

You are Claude Code, connected to the **Slug MCP server** running on this Mac.
Your job is to run a systematic validation of every Slug capability documented in
the agent guide. Work through the checklist below top-to-bottom. For each test:

1. Call the tool as described.
2. Check the result against the success criterion.
3. Print a one-line result: `✅ PASS — <test name>` or `❌ FAIL — <test name>: <reason>`.
4. Move to the next test immediately (do not stop for confirmation).

At the end print a summary table of all results.

---

## PHASE 0 — Verify Slug is reachable

**Test P0-1 · slug_help**
Call `slug_help {}`.
Success: response has `isError: false` and the text contains both `slug_snapshot`
and `slug_invoke`.

---

## PHASE 1 — Discovery tools

**Test 1-1 · tools/list completeness**
List all tools via MCP. Verify that every tool from this list is present:
`slug_snapshot`, `slug_invoke`, `slug_launch`, `slug_click`, `slug_scroll`,
`slug_key`, `slug_activate`, `slug_sequence`, `slug_wait_for`, `slug_list_apps`,
`slug_help`.
Success: all 11 names found.

**Test 1-2 · slug_list_apps**
Call `slug_list_apps {}`.
Success: `isError: false` and the response lists at least one running app (Finder
is always present on macOS).

**Test 1-3 · snapshot schema — filter params discoverable**
Look at the `slug_snapshot` input schema from tools/list.
Success: `filter`, `roles`, `interactive_only`, `limit`, `app` are all listed as
properties.

**Test 1-4 · snapshot disclaimer**
Check that the `slug_snapshot` tool description contains the string "NOT a
screenshot".
Success: present.

---

## PHASE 2 — Snapshot basics (no focus dependency)

> For every snapshot test below, use `app:"Finder"` (always running) so focus
> does not matter.

**Test 2-1 · app-targeted snapshot**
Call `slug_snapshot { "app": "Finder" }`.
Success: `isError: false`, YAML contains at least one node, no mention of
"focused".

**Test 2-2 · desktop scope**
Call `slug_snapshot { "scope": "desktop" }`.
Success: `isError: false`, YAML contains at least 2 different window or
application-level nodes.

**Test 2-3 · filter by role group "clickable"**
Call `slug_snapshot { "app": "Finder", "roles": ["clickable"], "limit": 10 }`.
Success: `isError: false`, every line in the result is a button/menu-item/link or
similar actionable role (none are `static_text` or `group`).

**Test 2-4 · filter by text**
Call `slug_snapshot { "app": "Finder", "filter": "File", "limit": 5 }`.
Success: `isError: false`, the result contains at most 5 nodes and each node name
contains "File" (case-insensitive).

**Test 2-5 · interactive_only drops containers**
Call `slug_snapshot { "app": "Finder", "interactive_only": true, "limit": 20 }`.
Success: `isError: false`, no node has role `group`, `split_group`, or `scroll_area`.

**Test 2-6 · limit is respected**
Call `slug_snapshot { "app": "Finder", "limit": 3 }`.
Success: at most 3 nodes are returned (the result may contain a "more matched"
comment, that is fine).

**Test 2-7 · coords: true prints @x,y**
Call `slug_snapshot { "app": "Finder", "roles": ["button"], "limit": 3, "coords": true }`.
Success: at least one line in the result contains `@` followed by digits.

**Test 2-8 · slug_snapshot with unknown app returns isError**
Call `slug_snapshot { "app": "ZZZNoSuchApp9999" }`.
Success: `isError: true`, not a protocol error (top-level `error` field is absent).

---

## PHASE 3 — Role groups

**Test 3-1 · role group "field"**
Call `slug_snapshot { "app": "Finder", "roles": ["field"], "limit": 20 }`.
Success: `isError: false` (empty result is OK — Finder may have no fields — but
must not error).

**Test 3-2 · role group "text"**
Call `slug_snapshot { "app": "Finder", "roles": ["text"], "limit": 10 }`.
Success: `isError: false`, every returned node is a label-style role (`static_text`,
`text`, or heading variant). No buttons.

**Test 3-3 · role group "heading"**
Open **TextEdit**: `slug_launch { "name": "TextEdit" }`.
Then call `slug_snapshot { "app": "TextEdit", "roles": ["button"], "limit": 10 }`.
Success for launch: `isError: false`.
Success for snapshot: `isError: false`, result contains refs.

---

## PHASE 4 — Actions

> Open **TextEdit** (if not already open from phase 3). We use it because it has
> standard accessible controls.

**Test 4-1 · slug_invoke set_text**
1. `slug_snapshot { "app": "TextEdit", "roles": ["entry", "field"], "limit": 5 }`.
   If no entry found, `slug_snapshot { "app": "TextEdit" }` to see the full tree.
2. Pick any text entry or the document body ref.
3. `slug_invoke { "ref": "<ref>", "action": "set_text", "args": "slug-test-123", "reasoning": "validation test" }`.
Success: `isError: false`, result text contains "ok" or "dispatched".

**Test 4-2 · slug_invoke click (button)**
1. `slug_snapshot { "app": "Finder", "roles": ["button"], "filter": "Close", "limit": 1 }`.
   (Finder's toolbar usually has a close/minimize area — if empty, use any button.)
2. If a ref is found, note it but **do NOT actually click Close** (would close
   the window). Instead call `slug_invoke { "ref": "<ref>", "action": "focus", "reasoning": "focus test only" }`.
Success: `isError: false`.

**Test 4-3 · slug_invoke on disabled node returns graceful error**
1. `slug_snapshot { "app": "Finder", "interactive_only": true, "limit": 30 }`.
2. If any node has `[disabled]` state, attempt `slug_invoke { "ref": "<disabled-ref>", "action": "click", "reasoning": "testing disabled guard" }`.
3. If no disabled nodes found, mark this test SKIP.
Success (if run): `isError: true` or result text contains "no effect".

**Test 4-4 · slug_click with coordinates**
1. `slug_snapshot { "app": "Finder", "roles": ["button"], "limit": 3, "coords": true }`.
2. Read an `@x,y` pair from a non-destructive button (e.g. a toolbar button).
3. `slug_click { "x": <x>, "y": <y> }`.
Success: `isError: false`.

**Test 4-5 · slug_click missing y — clean isError**
Call `slug_click { "x": 100 }`.
Success: `isError: true`, text contains `'y'` or "required".

**Test 4-6 · slug_key chord without focus theft**
Call `slug_key { "keys": "cmd+z", "activate": "TextEdit", "reasoning": "undo in TextEdit" }`.
Success: `isError: false` (the key is sent to TextEdit, not to the terminal).

**Test 4-7 · slug_key text mode**
Call `slug_key { "keys": "Hello from Slug", "mode": "text", "activate": "TextEdit", "reasoning": "type test string" }`.
Success: `isError: false`.

**Test 4-8 · slug_key with no keys — clean isError**
Call `slug_key {}`.
Success: `isError: true`.

---

## PHASE 5 — slug_sequence (atomic, no focus theft)

**Test 5-1 · empty steps list — clean isError**
Call `slug_sequence { "steps": [] }`.
Success: `isError: true`, text contains "empty".

**Test 5-2 · wait-only step runs without bus**
Call `slug_sequence { "steps": [ { "wait_ms": 1 } ] }`.
Success: `isError: false`, text contains "ran 1 steps".

**Test 5-3 · activate + type in one atomic call**
Call `slug_sequence { "steps": [ { "activate": "TextEdit" }, { "wait_ms": 200 }, { "text": "atomic-slug" }, { "key": "return" } ] }`.
Success: `isError: false`, text contains "ran 4 steps".

**Test 5-4 · sequence description mentions "atomic" and "focus"**
From the tools/list response, check `slug_sequence`'s description.
Success: text contains both "atomic" and "focus".

---

## PHASE 6 — slug_activate

**Test 6-1 · bring Finder to front**
Call `slug_activate { "app": "Finder" }`.
Success: `isError: false`.

**Test 6-2 · unknown app returns isError**
Call `slug_activate { "app": "ZZZNoSuchApp9999" }`.
Success: `isError: true` (not a protocol error).

---

## PHASE 7 — slug_launch

**Test 7-1 · launch without name or URI — clean isError**
Call `slug_launch {}`.
Success: `isError: true`, text contains "provide".

**Test 7-2 · launch TextEdit (idempotent — already open)**
Call `slug_launch { "name": "TextEdit" }`.
Success: `isError: false` (already running is OK, tool should not error).

**Test 7-3 · launch with a file URI**
Call `slug_launch { "name": "TextEdit", "uri": "/etc/hosts" }`.
Success: `isError: false`. (Opens the file; you can close it after.)

---

## PHASE 8 — slug_scroll

**Test 8-1 · scroll in Finder**
1. `slug_snapshot { "app": "Finder", "roles": ["scroll_area"], "limit": 3, "coords": true }`.
2. Pick a scroll area and read its `@x,y`.
3. `slug_scroll { "x": <x>, "y": <y>, "dy": -3 }`.
Success: `isError: false`.

**Test 8-2 · negative dy scrolls down**
Repeat with `dy: -5`. Success: `isError: false`.

---

## PHASE 9 — slug_wait_for

**Test 9-1 · wait_for with very short timeout expires gracefully**
Call `slug_wait_for { "event_type": "node_created", "timeout_ms": 200 }`.
Success: `isError: false`, result text contains "timeout" or the event if
coincidentally one fired.

---

## PHASE 10 — Known OS limitations (verify correct behaviour, not a bug)

**Test 10-1 · Spotify (if installed) appears as generic/opaque**
Call `slug_snapshot { "app": "Spotify", "limit": 5 }`.
If Spotify is not installed: mark SKIP.
If installed: Success is `isError: false` and either (a) result shows `generic
"Spotify"` with no child nodes, or (b) result contains "opaque". The key is that
it does NOT crash or return a protocol error.

**Test 10-2 · opaque app is drivable via slug_key**
If Spotify was found in test 10-1:
Call `slug_key { "keys": "space", "activate": "Spotify", "reasoning": "play/pause test" }`.
Success: `isError: false` (key is injected even with no accessible tree).

**Test 10-3 · Notes app — write-only via sequence**
`slug_launch { "name": "Notes" }`.
Then `slug_sequence { "steps": [ { "activate": "Notes" }, { "wait_ms": 300 }, { "text": "slug-validation-note" } ] }`.
Success: `isError: false`, ran N steps. (Body is WKWebView — you can write but
not read back via AX; this validates the write path works.)

**Test 10-4 · Chess.app is not drivable (no AX tree)**
`slug_launch { "name": "Chess" }`.
Then `slug_snapshot { "app": "Chess", "limit": 10 }`.
If Chess.app is not installed: mark SKIP.
Success: `isError: false` but the result is either empty or `generic` with no
children — Slug should not return a protocol error.

---

## PHASE 11 — Error contracts (must never be protocol errors)

All tool-call errors must be **isError tool results**, never JSON-RPC protocol
errors (top-level `error` field present). Confirm from tests above:

- `slug_list_apps` without bus → isError ✓ (tested in P0 or implicitly in 1-2)
- `slug_launch {}` → isError ✓ (test 7-1)
- `slug_click { x: 10 }` → isError ✓ (test 4-5)
- `slug_key {}` → isError ✓ (test 4-8)
- `slug_sequence { steps: [] }` → isError ✓ (test 5-1)
- Unknown tool → protocol error `-32602` ✓ (this is the *intended* case)

**Test 11-1 · unknown tool is a -32602 protocol error**
Call `tools/call` with `name: "slug_nonexistent_tool"`.
Success: top-level `error.code` equals `-32602`.

---

## PHASE 12 — Security gate

> This test requires `SLUG_DESTRUCTIVE=deny` on the server. Check if the env var
> is set; if not, you can still try the call — with the default `ask` mode it will
> block waiting for human approval. In that case mark SKIP rather than waiting.

**Test 12-1 · destructive action denied in deny mode**
Call `slug_invoke { "ref": "b1", "action": "click", "reasoning": "delete the account" }`.
(The word "delete" in reasoning triggers the destructive gate.)
If `SLUG_DESTRUCTIVE=deny`: Success is `isError: true`, text contains "denied".
If `SLUG_DESTRUCTIVE=ask`: mark SKIP (would block for human).
If `SLUG_DESTRUCTIVE=allow`: mark SKIP.

---

## PHASE 13 — Multi-monitor / off-screen coords

**Test 13-1 · off-screen nodes have negative X**
Call `slug_snapshot { "scope": "desktop", "coords": true, "limit": 50 }`.
Scan the result for any `@-` (negative X coordinate).
Success: if you have two monitors, at least some nodes appear with negative X.
If single monitor: this test may return no negative X — mark PASS with note
"single monitor setup, no negative X expected".

---

## PHASE 14 — Regression: snapshot description

**Test 14-1 · slug_snapshot description says NOT a screenshot**
Already covered in test 1-4.

**Test 14-2 · slug_sequence advertises atomicity**
Already covered in test 5-4.

---

## END — Summary

Print a markdown table:

| # | Test | Result |
|---|------|--------|
| P0-1 | slug_help | ✅/❌/SKIP |
| 1-1 | tools/list completeness | … |
| … | … | … |

Then print:
- Total PASS count
- Total FAIL count  
- Total SKIP count
- Any FAIL details with the actual error text received

If any test FAILs, briefly describe what Slug returned vs. what was expected, so
the developer can reproduce and fix it.
