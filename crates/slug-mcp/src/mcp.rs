//! Minimal Model Context Protocol implementation: JSON-RPC 2.0 framing, the
//! `initialize` / `tools/list` / `tools/call` methods, and the four Slug tools.
//!
//! We implement the protocol directly (rather than depending on an SDK) per the
//! task brief's fallback option. Tool *execution* errors are returned inside the
//! tool result object (`isError: true`), never as JSON-RPC protocol errors — only
//! malformed requests / unknown methods produce protocol errors.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::{debug, warn};

use crate::agent::AgentController;
use crate::session::{Scope, Session, SnapshotFilter};

/// The MCP protocol revision we advertise.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// JSON-RPC 2.0 request.
#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    #[allow(dead_code)]
    pub jsonrpc: String,
    /// Absent for notifications.
    #[serde(default)]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

/// JSON-RPC 2.0 response.
#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: &'static str,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
}

impl JsonRpcResponse {
    fn ok(id: Value, result: Value) -> Self {
        JsonRpcResponse { jsonrpc: "2.0", id, result: Some(result), error: None }
    }
    fn err(id: Value, code: i64, message: impl Into<String>) -> Self {
        JsonRpcResponse {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(JsonRpcError { code, message: message.into() }),
        }
    }
}

/// Handle a single JSON-RPC message (no agent controller — used by `slug-brain`'s
/// in-process tool dispatch). Returns `None` for notifications.
pub async fn handle(session: &Arc<Session>, req: JsonRpcRequest) -> Option<JsonRpcResponse> {
    handle_with_control(session, None, req).await
}

/// Handle a JSON-RPC message, optionally with an [`AgentController`] so the
/// agent-control tools are available (the `slug-mcp` daemon passes one; the
/// in-process `slug-brain` path passes `None`).
pub async fn handle_with_control(
    session: &Arc<Session>,
    control: Option<Arc<AgentController>>,
    req: JsonRpcRequest,
) -> Option<JsonRpcResponse> {
    debug!(method = %req.method, "rpc request");
    let is_notification = req.id.is_none();
    let id = req.id.clone().unwrap_or(Value::Null);

    let resp = match req.method.as_str() {
        "initialize" => JsonRpcResponse::ok(id, initialize_result()),
        "notifications/initialized" | "notifications/cancelled" => return None,
        "ping" => JsonRpcResponse::ok(id, json!({})),
        "tools/list" => JsonRpcResponse::ok(id, json!({ "tools": tool_definitions() })),
        "tools/call" => match handle_tool_call(session, control.as_ref(), req.params).await {
            Ok(result) => JsonRpcResponse::ok(id, result),
            Err(code_msg) => JsonRpcResponse::err(id, code_msg.0, code_msg.1),
        },
        other => {
            warn!(method = %other, "method not found");
            JsonRpcResponse::err(id, -32601, format!("method not found: {other}"))
        }
    };

    if is_notification {
        None
    } else {
        Some(resp)
    }
}

fn initialize_result() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": "slug-mcp", "version": env!("CARGO_PKG_VERSION") },
        "instructions": "Slug exposes the OS accessibility tree as a semantic \
            document (text, never a screenshot). Logical workflow: (1) slug_launch \
            to open an app by name (use uri= to jump straight to the right page/state \
            and skip clicks) if it isn't running; (2) slug_snapshot (scope 'focused' \
            is smallest/fastest) to read the UI — but pages can be tens of thousands \
            of characters, so to FIND a control pass filter='text' and/or \
            roles=['button'|'entry'|'link'|...] and/or interactive_only=true: this \
            returns a compact flat list of just the matching nodes, each with its \
            [ref=...] AND a centre @x,y. Use that instead of reading the whole tree; \
            (3) act: slug_invoke <ref> click/set_text/set_value/focus for accessible \
            controls, slug_key for keyboard chords or text, slug_click x,y for a mouse \
            click anywhere (use the @x,y from a filtered snapshot — this is also the \
            fallback when slug_invoke fails on canvas/opaque apps like chess.com or \
            maps); (4) verify with another filtered slug_snapshot. slug_wait_for is \
            unreliable on real apps — after an action just snapshot again rather than \
            waiting. Prefer slug_invoke on a ref over coordinates when a node exists; \
            refs are per-snapshot, never reuse old ones. Call slug_help any time for a \
            compact cheat-sheet of all commands."
    })
}

/// JSON-Schema tool definitions (`tools/list`).
pub fn tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "slug_snapshot",
            "description": "Read the current UI as a Playwright-style YAML tree. \
                NOT a screenshot: a slug_snapshot is a point-in-time read of the \
                semantic document (role, name, state, ref per element) as text/YAML — \
                analogous to a database snapshot. No image, pixel, or OCR is involved \
                anywhere in the pipeline. Each interactive node carries a short \
                [ref=...] used by slug_invoke. Apps with no accessible tree are \
                reported as opaque (vision needed). SPEED: a full web page can be \
                tens of thousands of characters — to find one control fast, pass \
                'filter' (text), 'roles' (e.g. [\"button\",\"entry\"]) and/or \
                'interactive_only', which return a compact FLAT list of just the \
                matching nodes, each with its ref AND centre @x,y (for slug_click). \
                Prefer that over reading the whole tree — an unfiltered dump of a dense page \
                (e.g. an e-commerce search results page) is truncated past ~20k characters, \
                since returning everything tends to overflow a calling client's own result \
                limit. There is no 'depth' or 'max_chars' parameter — control output size with \
                'filter'/'roles'/'interactive_only'/'limit' instead.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "app": {
                        "type": "string",
                        "description": "Snapshot THIS app by name (e.g. 'Notes', 'Spotify'), \
                            regardless of OS focus. USE THIS when you drive Slug from another \
                            window (e.g. a terminal): 'focused' reflects the OS-frontmost window, \
                            which is often your own client, not the app you mean. Matched \
                            case-insensitively. Overrides 'scope'."
                    },
                    "scope": {
                        "type": "string",
                        "enum": ["focused", "window", "desktop"],
                        "description": "focused/window = the OS-frontmost top-level window (fast, \
                            but it is whatever the OS focused — may be your controlling client; \
                            prefer the 'app' param to target a specific app); desktop = every \
                            running application across ALL monitors. Coordinates are global screen \
                            space, so @x,y from any scope works on a multi-monitor setup (a window \
                            on a second screen may have large or negative x — normal, pass as-is).",
                        "default": "window"
                    },
                    "filter": {
                        "type": "string",
                        "description": "Case-insensitive substring matched against each \
                            node's name/label. Returns a compact flat list of matches \
                            (with ref + @x,y) instead of the whole tree — the fast way \
                            to locate a button/field/link by its text."
                    },
                    "roles": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Keep only these roles — exact lower-case role names \
                            (e.g. [\"button\"], [\"entry\"], [\"static_text\"], [\"link\"]) \
                            or a friendly GROUP: \"clickable\" (any actionable control), \
                            \"field\"/\"input\" (text entries, combos, spinners), \"text\" \
                            (static text/labels/headings), \"link\", \"heading\". \
                            Searching for a button? pass roles:[\"button\"] and you get ONLY \
                            buttons. Combine with filter."
                    },
                    "interactive_only": {
                        "type": "boolean",
                        "description": "Keep only directly actionable controls (buttons, \
                            fields, links, checkboxes, …) — drops static text/containers."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "default": 50,
                        "description": "Max nodes returned (default 50). Passing 'limit' alone, \
                            with no filter/roles/interactive_only, still switches to the compact \
                            flat list capped at this many nodes — it does not return the full \
                            tree. Set limit:1 with a filter to get just the single best match — \
                            results are ranked so an exact name match comes first."
                    },
                    "coords": {
                        "type": "boolean",
                        "default": false,
                        "description": "Include each match's centre @x,y. Off by default to \
                            stay lean (you click normal controls by ref); turn on when you \
                            need a slug_click fallback. Opaque surfaces (canvas/image) always \
                            include @x,y."
                    }
                },
                "additionalProperties": false
            }
        }),
        json!({
            "name": "slug_invoke",
            "description": "Perform an action on a node by its ref (from slug_snapshot). \
                Actions: activate/click/press, focus, set_text, set_value, or any named \
                AT-SPI action (toggle, expand, select, ...).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "ref": { "type": "string", "description": "Node ref alias, e.g. b1 or e5." },
                    "action": {
                        "type": "string",
                        "description": "activate | click | press | focus | set_text | set_value | toggle | expand | ..."
                    },
                    "args": {
                        "type": "string",
                        "description": "Argument for the action: the text for set_text, the number for set_value."
                    },
                    "reasoning": {
                        "type": "string",
                        "description": "Why you are taking this action (logged for auditing)."
                    }
                },
                "required": ["ref", "action"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "slug_launch",
            "description": "Launch an application by name (e.g. 'Spotify'), optionally \
                opening a URI / deep link with it (e.g. uri 'spotify:playlist:…'). Use \
                this to START an app before driving it — Slug otherwise only controls \
                already-running apps. Works without the accessibility bus.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Application name, e.g. Spotify, Safari, Finder." },
                    "uri": { "type": "string", "description": "Optional URI / deep link / file path to open with it." }
                },
                "additionalProperties": false
            }
        }),
        json!({
            "name": "slug_click",
            "description": "Synthetic left mouse click at absolute GLOBAL screen coordinates \
                (x, y) — these span all monitors, so a point on a second screen (possibly with \
                large or negative x) just works. Lets the agent click ANYWHERE, including inside \
                opaque apps, when it has a position (e.g. @x,y from a filtered snapshot). No pixels \
                are captured. macOS + Windows.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "x": { "type": "number", "description": "Absolute screen X." },
                    "y": { "type": "number", "description": "Absolute screen Y." },
                    "activate": { "type": "string", "description": "Optional app name to bring to the front FIRST (same call), so the click lands in it and not in the client's window." },
                    "reasoning": { "type": "string", "description": "Why (logged)." }
                },
                "required": ["x", "y"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "slug_scroll",
            "description": "Synthetic scroll at screen coordinates (x, y) by dy wheel \
                lines (negative dy scrolls DOWN, positive UP) and optional dx. Reveals \
                off-screen content — e.g. a grid item or list entry not yet visible. No \
                pixels. macOS + Windows implemented; Linux OS-constrained.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "x": { "type": "number", "description": "Screen X to scroll over." },
                    "y": { "type": "number", "description": "Screen Y to scroll over." },
                    "dy": { "type": "number", "description": "Vertical lines; negative = down." },
                    "dx": { "type": "number", "description": "Horizontal lines (optional)." },
                    "reasoning": { "type": "string", "description": "Why (logged)." }
                },
                "required": ["x", "y", "dy"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "slug_key",
            "description": "Send synthetic keyboard input to the focused app — a key \
                chord (mode=chord, e.g. 'cmd+s', 'return', 'shift+tab', 'down') or \
                literal text (mode=text). Drives ANY app, including opaque ones with \
                no accessibility tree, capturing NO pixels (it injects an OS input \
                event). IMPORTANT: if you are driving Slug from another window (e.g. a \
                terminal), keyboard focus returns to THAT window between tool calls, so \
                keys can land there instead of the target app. Pass 'activate' with the \
                target app name to bring it to the front in this SAME call first — or \
                use slug_sequence to do activate+type+enter atomically.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "keys": { "type": "string", "description": "Chord like 'cmd+shift+z', a key name like 'return'/'tab'/'escape'/'up', or literal text when mode=text." },
                    "mode": { "type": "string", "enum": ["chord", "text"], "default": "chord",
                              "description": "chord = key combo; text = type the string literally." },
                    "ref": { "type": "string", "description": "Optional node ref to focus before sending input." },
                    "activate": { "type": "string", "description": "Optional app name to bring to the front FIRST (same call), so the keys land in it and not in the controlling client's window." },
                    "settle_ms": { "type": "integer", "minimum": 0, "default": 120, "description": "Delay after activating before sending keys, to let the window-server settle." },
                    "reasoning": { "type": "string", "description": "Why (logged for auditing)." }
                },
                "required": ["keys"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "slug_activate",
            "description": "Bring an application to the foreground (give it keyboard \
                focus) so the NEXT synthetic input lands in it. Use this when Slug is \
                driven from another window (e.g. a terminal) that keeps stealing focus. \
                For typing right after, prefer slug_sequence (atomic).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "app": { "type": "string", "description": "Application name to activate, e.g. 'Safari'." },
                    "settle_ms": { "type": "integer", "minimum": 0, "default": 120, "description": "How long to wait after activating, in ms." }
                },
                "required": ["app"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "slug_sequence",
            "description": "Run several actions in ONE atomic call, with NO return to \
                the client in between — so keyboard focus can't be stolen mid-sequence. \
                This is THE way to type into another app when you drive Slug from a \
                terminal: e.g. [{activate:'Safari'},{wait_ms:200},{text:'crane'},{key:'return'}]. \
                Steps run in order; each step is one of: \
                {activate:'App'} (foreground an app), {focus:'ref'} (focus a node), \
                {click:'ref'} (invoke a node) or {click_xy:[x,y]} (click a point), \
                {key:'return'} (chord/keyname), {text:'hello'} (type literally), \
                {wait_ms:200} (pause).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "steps": {
                        "type": "array",
                        "minItems": 1,
                        "description": "Ordered list of step objects (see description).",
                        "items": { "type": "object" }
                    },
                    "settle_ms": { "type": "integer", "minimum": 0, "default": 150, "description": "Default pause inserted right after each {activate} step." },
                    "reasoning": { "type": "string", "description": "Why (logged for auditing)." }
                },
                "required": ["steps"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "slug_wait_for",
            "description": "Block until a UI event occurs or the timeout elapses. \
                Event types: node_created, node_destroyed, node_updated, focus_changed.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "event_type": {
                        "type": "string",
                        "description": "Event to wait for; omit to wait for any event.",
                        "enum": ["node_created", "node_destroyed", "node_updated", "focus_changed", "any"]
                    },
                    "timeout_ms": {
                        "type": "integer",
                        "minimum": 0,
                        "default": 5000,
                        "description": "Maximum time to wait, in milliseconds."
                    }
                },
                "additionalProperties": false
            }
        }),
        json!({
            "name": "slug_list_apps",
            "description": "List the running applications currently exposing an accessibility tree.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        }),
        json!({
            "name": "slug_help",
            "description": "Return a short cheat-sheet of how to drive Slug efficiently \
                (workflow, fast filtered search, acting, gotchas). Call once if unsure — it \
                is cheap and self-contained.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        }),
        json!({
            "name": "slug_status",
            "description": "One-shot health/status report, printed directly in this chat: \
                app version, configured AI brain (provider/model/ready), which transport an \
                MCP client is connected over, accessibility-bus connectivity, pending \
                destructive-action approvals, and the built-in agent's current task if one is \
                running. This is the dashboard's content as text — call it when you want a \
                status check without leaving the chat or opening a browser.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        }),
        json!({
            "name": "slug_agent_start_task",
            "description": "Start the slug-brain agent on a natural-language task. Drives \
                the UI autonomously via the same tools (observe→reason→act→verify).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "description": { "type": "string", "description": "What the agent should do." }
                },
                "required": ["description"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "slug_agent_status",
            "description": "Current agent task, status (idle/running/paused/done/stopped), \
                active provider/tier/model, and the last 20 reasoning/action log lines.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        }),
        json!({
            "name": "slug_agent_pause",
            "description": "Pause the running agent task (suspends the process).",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        }),
        json!({
            "name": "slug_agent_resume",
            "description": "Resume a paused agent task.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        }),
        json!({
            "name": "slug_agent_stop",
            "description": "Stop and clear the running agent task.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        }),
    ]
}

/// Dispatch a `tools/call`. The `Err` variant is a *protocol* error (bad params);
/// tool execution failures are returned as `Ok` with `isError: true`.
async fn handle_tool_call(
    session: &Arc<Session>,
    control: Option<&Arc<AgentController>>,
    params: Value,
) -> std::result::Result<Value, (i64, String)> {
    let name = params.get("name").and_then(Value::as_str).ok_or((-32602, "missing tool name".into()))?;
    let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));

    // Security gate: destructive actions from an external client (i.e. when a
    // controller is attached — the daemon path) are enforced here, since such
    // clients never go through the agent's own confirmation hook. The in-process
    // agent path passes `control == None` and keeps using its own gate.
    if let Some(ctrl) = control {
        if let Some(summary) = crate::approval::destructive_summary(name, &args) {
            use crate::approval::{Decision, PolicyMode, DEFAULT_APPROVAL_TIMEOUT};
            match PolicyMode::from_env() {
                PolicyMode::Allow => {}
                PolicyMode::Deny => {
                    return Ok(tool_text(
                        format!("denied: destructive action blocked by policy (SLUG_DESTRUCTIVE=deny): {summary}"),
                        true,
                    ));
                }
                PolicyMode::Ask => {
                    match ctrl.approvals().request(name, &summary, DEFAULT_APPROVAL_TIMEOUT).await {
                        Decision::Approved => {}
                        Decision::Denied => {
                            return Ok(tool_text(
                                format!("denied: a human declined this action in the dashboard: {summary}"),
                                true,
                            ));
                        }
                        Decision::TimedOut => {
                            return Ok(tool_text(
                                format!("denied: no human approval within timeout — open the Slug dashboard to approve destructive actions: {summary}"),
                                true,
                            ));
                        }
                    }
                }
            }
        }
    }

    let result = match name {
        "slug_snapshot" => tool_snapshot(session, &args).await,
        "slug_invoke" => tool_invoke(session, &args).await,
        "slug_launch" => tool_launch(session, &args).await,
        "slug_click" => tool_click(session, &args).await,
        "slug_scroll" => tool_scroll(session, &args).await,
        "slug_key" => tool_key(session, &args).await,
        "slug_activate" => tool_activate(session, &args).await,
        "slug_sequence" => tool_sequence(session, &args).await,
        "slug_wait_for" => tool_wait_for(session, &args).await,
        "slug_list_apps" => tool_list_apps(session).await,
        "slug_help" => Ok(help_text()),
        "slug_status" => tool_status(session, control).await,
        name if name.starts_with("slug_agent_") => match control {
            Some(ctrl) => agent_tool(ctrl, name, &args).await,
            None => Err("agent control is not available on this transport".into()),
        },
        other => return Err((-32602, format!("unknown tool: {other}"))),
    };

    Ok(match result {
        Ok(text) => tool_text(text, false),
        Err(msg) => tool_text(msg, true),
    })
}

/// Dispatch an agent-control tool to the [`AgentController`].
async fn agent_tool(
    ctrl: &Arc<AgentController>,
    name: &str,
    args: &Value,
) -> std::result::Result<String, String> {
    match name {
        "slug_agent_start_task" => {
            let desc = args.get("description").and_then(Value::as_str).ok_or("missing 'description'")?;
            ctrl.start_task(desc).await
        }
        "slug_agent_status" => {
            let status = ctrl.status().await;
            Ok(serde_json::to_string_pretty(&status).unwrap_or_else(|_| "{}".into()))
        }
        "slug_agent_pause" => ctrl.pause().await,
        "slug_agent_resume" => ctrl.resume().await,
        "slug_agent_stop" => ctrl.stop().await,
        other => Err(format!("unknown agent tool: {other}")),
    }
}

/// A compact, self-contained cheat-sheet returned by `slug_help` — so any
/// connected agent can learn the commands cheaply (on demand), regardless of
/// transport or whether the client surfaces the `initialize` instructions.
fn help_text() -> String {
    "SLUG — drive native apps by reading the OS accessibility tree as text (never \
screenshots).\n\
WORKFLOW: slug_launch (open app) → slug_snapshot (read) → slug_invoke (act on a \
ref) → slug_snapshot again to verify.\n\
TARGET THE RIGHT APP: if you drive Slug from another window (e.g. a terminal), \
scope:\"focused\" reads the OS-frontmost window (often your own client), so pass \
app:\"Notes\" to snapshot a specific app regardless of focus.\n\
FIND FAST (saves tokens — don't pull the whole tree): \
slug_snapshot {app:\"Safari\", roles:[…], filter:\"text\", limit:1}. \
roles take exact names (button, entry, static_text, link…) OR groups \
(clickable, field, text, link, heading). Returns only matches as `role \"name\" \
[ref]`; exact-name match ranks first, so limit:1 gives the one you meant. Add \
coords:true to also get @x,y.\n\
ACT: slug_invoke {ref, action:\"click\"|\"set_text\"|\"set_value\"|\"focus\"|\"toggle\"|…, \
args, reasoning}. Forms: set_text every field, then click submit last.\n\
NO ACCESSIBLE TREE (canvas/games): slug_click {x,y}, slug_scroll {x,y,dy} (dy<0 = \
down), slug_key {keys:\"cmd+s\"} or {keys:\"hello\", mode:\"text\"}. Get x,y from a \
snapshot (coords:true); never invent them.\n\
FOCUS GOTCHA: if you drive Slug from another window (e.g. a terminal), focus \
returns there between calls, so keys can miss the target app. FIX: send the whole \
move in ONE call with slug_sequence, e.g. \
[{activate:\"Safari\"},{wait_ms:200},{text:\"crane\"},{key:\"return\"}] — nothing can \
steal focus mid-sequence. Or pass activate:\"App\" to slug_key/slug_click to \
foreground it first in the same call. slug_activate {app} just brings it to front.\n\
OTHER: slug_launch {name, uri?} (uri jumps straight to a page/state); \
slug_list_apps. slug_wait_for often times out — prefer snapshotting again. \
slug_status prints a one-shot health report (brain, connection, pending approvals, \
running agent task) right here in the chat — no dashboard needed.\n\
RULES: refs change whenever the UI changes — re-snapshot, never reuse old refs. \
Destructive actions (delete/send/buy/submit) may pause for human approval in the \
dashboard. Prefer slug_invoke on a ref over raw coordinates."
        .to_string()
}

/// Build a `tools/call` result object with a single text content block.
fn tool_text(text: String, is_error: bool) -> Value {
    json!({
        "content": [ { "type": "text", "text": text } ],
        "isError": is_error
    })
}

// --- Individual tools. Inner Err(String) becomes an isError tool result. ---

async fn tool_snapshot(session: &Arc<Session>, args: &Value) -> std::result::Result<String, String> {
    let scope_str = args.get("scope").and_then(Value::as_str).unwrap_or("window");
    let scope = Scope::parse(scope_str).unwrap_or(Scope::Window);

    let filter = SnapshotFilter {
        query: args
            .get("filter")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        roles: args
            .get("roles")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(|s| s.trim().to_ascii_lowercase())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default(),
        interactive_only: args.get("interactive_only").and_then(Value::as_bool).unwrap_or(false),
        limit: args
            .get("limit")
            .and_then(Value::as_u64)
            .map(|n| n.max(1) as usize),
        coords: args.get("coords").and_then(Value::as_bool).unwrap_or(false),
    };

    let active = filter.is_active();
    // Target a specific app by name when given — focus-independent, so it reads the
    // right app even though the controlling client holds OS focus.
    let app = args.get("app").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
    let out = match app {
        Some(app) => session.snapshot_app(app, &filter).await.map_err(|e| e.to_string())?,
        None => session.snapshot_filtered(scope, &filter).await.map_err(|e| e.to_string())?,
    };
    let mut text = out.yaml;
    if !out.opaque.is_empty() {
        text.push_str("\n# opaque apps (no/flat accessibility tree — vision fallback):\n");
        for c in &out.opaque {
            text.push_str(&format!("#   - {} ({:?})\n", c.app_id, c.opaque.unwrap()));
        }
    }
    // If an unfiltered snapshot came back large, nudge the agent to filter next
    // time — this keeps any client token-efficient without it reading the docs.
    // Past SNAPSHOT_HARD_CAP we don't just nudge, we truncate: dense pages (e.g.
    // an e-commerce search/product page) can render hundreds of KB of YAML for
    // the full tree, which blows straight past a calling client's own
    // tool-result size limit — that overflow has been observed in practice to
    // force a slow file-dump-and-grep fallback that takes minutes instead of
    // seconds. Returning a truncated tree with clear next steps is strictly
    // better than returning everything and letting the caller's transport choke.
    if !active && text.len() > SNAPSHOT_HARD_CAP_CHARS {
        truncate_at_char_boundary(&mut text, SNAPSHOT_HARD_CAP_CHARS);
        text.push_str(&format!(
            "\n# … truncated at {SNAPSHOT_HARD_CAP_CHARS} chars — the full tree is much larger \
             and would likely overflow your own result limits. This page is too dense for a full \
             dump; narrow it instead: slug_snapshot {{roles:[\"button\"|\"link\"|\"text\"], \
             filter:\"text\", limit:20}} or {{interactive_only:true, limit:50}}. For text content \
             like prices/ratings that won't match a literal symbol (e.g. \"EUR 26.32\" has no \
             \"$\"), prefer roles:[\"text\"] with a generous limit over guessing a filter string.\n"
        ));
    } else if !active && text.len() > SNAPSHOT_TIP_THRESHOLD_CHARS {
        text.push_str(
            "\n# tip: large result — to save tokens, narrow it: \
             slug_snapshot {roles:[\"button\"|\"field\"|…], filter:\"text\", limit:1}\n",
        );
    }
    Ok(text)
}

/// Soft threshold: past this, append an advisory tip but still return the full
/// unfiltered tree.
const SNAPSHOT_TIP_THRESHOLD_CHARS: usize = 8_000;

/// Hard cap on an unfiltered (full-tree) snapshot's rendered length. Past this,
/// truncate rather than return everything — see the comment at the call site.
const SNAPSHOT_HARD_CAP_CHARS: usize = 20_000;

const _: () = assert!(SNAPSHOT_HARD_CAP_CHARS > SNAPSHOT_TIP_THRESHOLD_CHARS);

/// Truncate `s` to at most `max` bytes, backing off to the nearest UTF-8 char
/// boundary so we never split a multi-byte character.
fn truncate_at_char_boundary(s: &mut String, max: usize) {
    let mut cut = max.min(s.len());
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    s.truncate(cut);
}

async fn tool_invoke(session: &Arc<Session>, args: &Value) -> std::result::Result<String, String> {
    let r = args.get("ref").and_then(Value::as_str).ok_or("missing 'ref'")?;
    let action = args.get("action").and_then(Value::as_str).ok_or("missing 'action'")?;
    let inner = args.get("args").and_then(Value::as_str);
    let reasoning = args.get("reasoning").and_then(Value::as_str);

    let ok = session.invoke(r, action, inner, reasoning).await.map_err(|e| e.to_string())?;
    Ok(if ok {
        format!("ok: {action} on {r} succeeded")
    } else {
        format!("note: {action} on {r} was dispatched but the toolkit reported no effect")
    })
}

async fn tool_launch(session: &Arc<Session>, args: &Value) -> std::result::Result<String, String> {
    let name = args.get("name").and_then(Value::as_str).unwrap_or("");
    let uri = args.get("uri").and_then(Value::as_str);
    if name.is_empty() && uri.is_none() {
        return Err("provide 'name' or 'uri'".into());
    }
    session.launch(name, uri).await.map_err(|e| e.to_string())?;
    let suffix = uri.map(|u| format!(" ({u})")).unwrap_or_default();
    Ok(format!("ok: launched {name}{suffix}"))
}

async fn tool_click(session: &Arc<Session>, args: &Value) -> std::result::Result<String, String> {
    let x = args.get("x").and_then(Value::as_f64).ok_or("missing numeric 'x'")?;
    let y = args.get("y").and_then(Value::as_f64).ok_or("missing numeric 'y'")?;
    let reasoning = args.get("reasoning").and_then(Value::as_str);
    maybe_activate(session, args.get("activate").and_then(Value::as_str), args).await?;
    session.synth("click_at", Some(&format!("{x},{y}")), None, reasoning).await.map_err(|e| e.to_string())?;
    Ok(format!("ok: clicked at {x},{y}"))
}

/// If an `activate` app name is present, foreground it and pause `settle_ms`
/// (default 120ms) before the caller sends input — so the input lands there and
/// not in the window the controlling client lives in.
async fn maybe_activate(
    session: &Arc<Session>,
    app: Option<&str>,
    args: &Value,
) -> std::result::Result<(), String> {
    if let Some(app) = app.map(str::trim).filter(|s| !s.is_empty()) {
        session.activate(app).await.map_err(|e| e.to_string())?;
        let ms = args.get("settle_ms").and_then(Value::as_u64).unwrap_or(120);
        if ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
        }
    }
    Ok(())
}

async fn tool_activate(session: &Arc<Session>, args: &Value) -> std::result::Result<String, String> {
    let app = args.get("app").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
        .ok_or("missing 'app'")?;
    maybe_activate(session, Some(app), args).await?;
    Ok(format!("ok: activated {app}"))
}

/// Run a list of steps atomically in one call (no client round-trip between
/// them), so keyboard focus can't be stolen mid-sequence. See the tool schema for
/// the step grammar.
async fn tool_sequence(session: &Arc<Session>, args: &Value) -> std::result::Result<String, String> {
    let steps = args.get("steps").and_then(Value::as_array).ok_or("missing 'steps' array")?;
    if steps.is_empty() {
        return Err("'steps' is empty".into());
    }
    let reasoning = args.get("reasoning").and_then(Value::as_str);
    let settle_ms = args.get("settle_ms").and_then(Value::as_u64).unwrap_or(150);
    let mut log: Vec<String> = Vec::new();

    for (i, step) in steps.iter().enumerate() {
        let obj = step.as_object().ok_or_else(|| format!("step {} is not an object", i + 1))?;
        let n = i + 1;
        // Each step object carries exactly one action key.
        if let Some(app) = obj.get("activate").and_then(Value::as_str) {
            session.activate(app).await.map_err(|e| format!("step {n} activate: {e}"))?;
            if settle_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(settle_ms)).await;
            }
            log.push(format!("activate {app}"));
        } else if let Some(r) = obj.get("focus").and_then(Value::as_str) {
            session.invoke(r, "focus", None, reasoning).await.map_err(|e| format!("step {n} focus: {e}"))?;
            log.push(format!("focus {r}"));
        } else if let Some(r) = obj.get("click").and_then(Value::as_str) {
            session.invoke(r, "click", None, reasoning).await.map_err(|e| format!("step {n} click: {e}"))?;
            log.push(format!("click {r}"));
        } else if let Some(xy) = obj.get("click_xy").and_then(Value::as_array) {
            let (x, y) = xy_pair(xy).ok_or_else(|| format!("step {n} click_xy: expected [x,y]"))?;
            session.synth("click_at", Some(&format!("{x},{y}")), None, reasoning).await
                .map_err(|e| format!("step {n} click_xy: {e}"))?;
            log.push(format!("click_xy {x},{y}"));
        } else if let Some(keys) = obj.get("key").and_then(Value::as_str) {
            session.synth("key", Some(keys), None, reasoning).await.map_err(|e| format!("step {n} key: {e}"))?;
            log.push(format!("key {keys}"));
        } else if let Some(text) = obj.get("text").and_then(Value::as_str) {
            session.synth("type_text", Some(text), None, reasoning).await.map_err(|e| format!("step {n} text: {e}"))?;
            log.push(format!("text \"{text}\""));
        } else if let Some(ms) = obj.get("wait_ms").and_then(Value::as_u64) {
            tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
            log.push(format!("wait {ms}ms"));
        } else {
            return Err(format!(
                "step {n}: unknown step (use activate/focus/click/click_xy/key/text/wait_ms)"
            ));
        }
    }
    Ok(format!("ok: ran {} steps → {}", steps.len(), log.join(" · ")))
}

/// Parse a JSON `[x, y]` number pair.
fn xy_pair(arr: &[Value]) -> Option<(f64, f64)> {
    match arr {
        [x, y, ..] => Some((x.as_f64()?, y.as_f64()?)),
        _ => None,
    }
}

async fn tool_scroll(session: &Arc<Session>, args: &Value) -> std::result::Result<String, String> {
    let x = args.get("x").and_then(Value::as_f64).ok_or("missing numeric 'x'")?;
    let y = args.get("y").and_then(Value::as_f64).ok_or("missing numeric 'y'")?;
    let dy = args.get("dy").and_then(Value::as_f64).ok_or("missing numeric 'dy'")?;
    let dx = args.get("dx").and_then(Value::as_f64).unwrap_or(0.0);
    let reasoning = args.get("reasoning").and_then(Value::as_str);
    session
        .synth("scroll", Some(&format!("{x},{y},{dx},{dy}")), None, reasoning)
        .await
        .map_err(|e| e.to_string())?;
    Ok(format!("ok: scrolled at {x},{y} by dx={dx} dy={dy}"))
}

async fn tool_key(session: &Arc<Session>, args: &Value) -> std::result::Result<String, String> {
    let keys = args.get("keys").and_then(Value::as_str).ok_or("missing 'keys'")?;
    let mode = args.get("mode").and_then(Value::as_str).unwrap_or("chord");
    let focus = args.get("ref").and_then(Value::as_str);
    let reasoning = args.get("reasoning").and_then(Value::as_str);
    let verb = if mode == "text" { "type_text" } else { "key" };

    let activate = args.get("activate").and_then(Value::as_str);
    maybe_activate(session, activate, args).await?;
    session.synth(verb, Some(keys), focus, reasoning).await.map_err(|e| e.to_string())?;
    match activate {
        Some(app) => Ok(format!("ok: sent {mode} '{keys}' to {app}")),
        // No target app: the OS routes synthetic input to whatever is frontmost,
        // which — when Slug is driven from a terminal/another window — is the
        // CLIENT, not the app you meant. Say so, so a chord that "did nothing"
        // (e.g. cmd+a landing in the terminal) isn't mistaken for a bug.
        None => Ok(format!(
            "ok: posted {mode} '{keys}' to whatever app is frontmost. \
             If you're driving Slug from another window and this seemed to do nothing, \
             the keys likely landed in your client — pass activate:\"<App>\" (same call) \
             or use slug_sequence so they reach the target app."
        )),
    }
}

async fn tool_wait_for(session: &Arc<Session>, args: &Value) -> std::result::Result<String, String> {
    let event_type = args
        .get("event_type")
        .and_then(Value::as_str)
        .filter(|t| *t != "any");
    let timeout_ms = args.get("timeout_ms").and_then(Value::as_u64).unwrap_or(5000);

    match session.wait_for(event_type, timeout_ms).await.map_err(|e| e.to_string())? {
        Some(ev) => {
            let json = serde_json::to_string_pretty(&ev).unwrap_or_else(|_| "{}".into());
            Ok(format!("event: {}\n{json}", ev.type_name()))
        }
        None => Ok(format!("timeout: no matching event within {timeout_ms}ms")),
    }
}

async fn tool_list_apps(session: &Arc<Session>) -> std::result::Result<String, String> {
    let apps = session.list_apps().await.map_err(|e| e.to_string())?;
    if apps.is_empty() {
        return Ok("(no accessible applications found)".into());
    }
    let mut out = String::new();
    for a in apps {
        out.push_str(&format!("- {} [{}]\n", if a.app_id.is_empty() { "<unnamed>" } else { &a.app_id }, a.bus_name));
    }
    Ok(out)
}

/// `slug_status` — the dashboard's content as text, printed straight into the
/// chat. Built for clients (like Claude Code over stdio) that can't open a
/// browser to see `GET /dashboard`: app version, brain, which transport is
/// connected, bus reachability, pending approvals, and any running agent task.
async fn tool_status(
    session: &Arc<Session>,
    control: Option<&Arc<AgentController>>,
) -> std::result::Result<String, String> {
    let mut out = String::new();
    out.push_str(&format!("Slug v{}\n", env!("CARGO_PKG_VERSION")));

    let brain = crate::dashboard_api::brain_detail();
    out.push_str(&format!(
        "Brain: {} / {} [{}] — {}\n",
        brain.get("provider").and_then(Value::as_str).unwrap_or("?"),
        brain.get("model").and_then(Value::as_str).unwrap_or("?"),
        brain.get("location").and_then(Value::as_str).unwrap_or("?"),
        if brain.get("ready").and_then(Value::as_bool).unwrap_or(false) { "ready" } else { "not ready" },
    ));

    let client = crate::dashboard_api::client_status(60);
    out.push_str(&match client.get("connected").and_then(Value::as_bool).unwrap_or(false) {
        true => format!(
            "MCP client: connected via {} ({}s ago)\n",
            client.get("transport").and_then(Value::as_str).unwrap_or("?"),
            client.get("last_seen_s").and_then(Value::as_u64).unwrap_or(0),
        ),
        false => "MCP client: no recent client seen\n".to_string(),
    });

    match session.list_apps().await {
        Ok(apps) => out.push_str(&format!("Accessibility bus: connected ({} app(s) visible)\n", apps.len())),
        Err(e) => out.push_str(&format!("Accessibility bus: NOT connected ({e})\n")),
    }

    match control {
        Some(ctrl) => {
            let pending = ctrl.approvals().list().await;
            let pending_n = pending.get("pending").and_then(Value::as_array).map(|a| a.len()).unwrap_or(0);
            out.push_str(&format!("Pending approvals: {pending_n}\n"));

            let status = ctrl.status().await;
            let agent_status = status.get("status").and_then(Value::as_str).unwrap_or("idle");
            if agent_status == "idle" {
                out.push_str("Agent task: none running\n");
            } else {
                out.push_str(&format!(
                    "Agent task: {} (status={}, paused={}, steps={}, elapsed={}s)\n  {}\n",
                    status.get("task").and_then(Value::as_str).unwrap_or("?"),
                    agent_status,
                    status.get("paused").and_then(Value::as_bool).unwrap_or(false),
                    status.get("steps").and_then(Value::as_u64).unwrap_or(0),
                    status.get("elapsed_s").and_then(Value::as_u64).unwrap_or(0),
                    status.get("log").and_then(Value::as_array).and_then(|l| l.last()).and_then(Value::as_str).unwrap_or(""),
                ));
            }
        }
        None => out.push_str("Agent control: not available on this transport\n"),
    }

    Ok(out)
}

#[cfg(test)]
mod snapshot_cap_tests {
    use super::*;

    #[test]
    fn truncate_caps_length_and_respects_char_boundaries() {
        let mut s = "a".repeat(50);
        truncate_at_char_boundary(&mut s, 20);
        assert_eq!(s.len(), 20);

        // A multi-byte char (3 bytes) sitting right at the cut point must not be
        // split — the cut should back off to the char before it.
        let mut s: String = std::iter::repeat('a').take(19).chain(std::iter::once('€')).collect();
        truncate_at_char_boundary(&mut s, 20);
        assert!(s.is_char_boundary(s.len()));
        assert_eq!(s, "a".repeat(19));

        // Shorter than the cap: untouched.
        let mut s = "short".to_string();
        truncate_at_char_boundary(&mut s, 20);
        assert_eq!(s, "short");
    }
}
