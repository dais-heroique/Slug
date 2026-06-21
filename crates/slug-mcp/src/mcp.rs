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
            refs are per-snapshot, never reuse old ones."
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
                Prefer that over reading the whole tree.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "scope": {
                        "type": "string",
                        "enum": ["focused", "window", "desktop"],
                        "description": "focused/window = the focused top-level window; \
                            desktop = every running application.",
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
                        "description": "Keep only these roles, lower-case as shown in \
                            snapshots, e.g. [\"button\"], [\"entry\",\"combo_box\"], \
                            [\"static_text\"], [\"link\",\"heading\"]. Combine with filter."
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
                        "description": "Max nodes returned in filtered mode (default 50)."
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
            "description": "Synthetic left mouse click at absolute screen coordinates \
                (x, y). Lets the agent click ANYWHERE, including inside opaque apps, when \
                it has a position (e.g. from a node's bounds). No pixels are captured. \
                macOS + Windows implemented; Linux is OS-constrained.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "x": { "type": "number", "description": "Absolute screen X." },
                    "y": { "type": "number", "description": "Absolute screen Y." },
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
                literal text (mode=text). This drives ANY app, including opaque ones \
                with no accessibility tree, and still captures NO pixels: it injects \
                an OS input event, not a node action. Optionally focus a node first \
                via 'ref'. (macOS implemented; Linux/Windows: follow-up.)",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "keys": { "type": "string", "description": "Chord like 'cmd+shift+z', a key name like 'return'/'tab'/'escape'/'up', or literal text when mode=text." },
                    "mode": { "type": "string", "enum": ["chord", "text"], "default": "chord",
                              "description": "chord = key combo; text = type the string literally." },
                    "ref": { "type": "string", "description": "Optional node ref to focus before sending input." },
                    "reasoning": { "type": "string", "description": "Why (logged for auditing)." }
                },
                "required": ["keys"],
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
        "slug_wait_for" => tool_wait_for(session, &args).await,
        "slug_list_apps" => tool_list_apps(session).await,
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
    };

    let out = session.snapshot_filtered(scope, &filter).await.map_err(|e| e.to_string())?;
    let mut text = out.yaml;
    if !out.opaque.is_empty() {
        text.push_str("\n# opaque apps (no/flat accessibility tree — vision fallback):\n");
        for c in &out.opaque {
            text.push_str(&format!("#   - {} ({:?})\n", c.app_id, c.opaque.unwrap()));
        }
    }
    Ok(text)
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
    session.synth("click_at", Some(&format!("{x},{y}")), None, reasoning).await.map_err(|e| e.to_string())?;
    Ok(format!("ok: clicked at {x},{y}"))
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

    session.synth(verb, Some(keys), focus, reasoning).await.map_err(|e| e.to_string())?;
    Ok(format!("ok: sent {mode} '{keys}' to the focused app"))
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
