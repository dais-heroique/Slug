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

use crate::session::{Scope, Session};

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

/// Handle a single JSON-RPC message. Returns `None` for notifications (no reply).
pub async fn handle(session: &Arc<Session>, req: JsonRpcRequest) -> Option<JsonRpcResponse> {
    debug!(method = %req.method, "rpc request");
    let is_notification = req.id.is_none();
    let id = req.id.clone().unwrap_or(Value::Null);

    let resp = match req.method.as_str() {
        "initialize" => JsonRpcResponse::ok(id, initialize_result()),
        "notifications/initialized" | "notifications/cancelled" => return None,
        "ping" => JsonRpcResponse::ok(id, json!({})),
        "tools/list" => JsonRpcResponse::ok(id, json!({ "tools": tool_definitions() })),
        "tools/call" => match handle_tool_call(session, req.params).await {
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
        "instructions": "Slug exposes the Linux AT-SPI2 accessibility tree as a \
            semantic document. Call slug_snapshot to read the UI as YAML (each node \
            has a short [ref=...]), then slug_invoke with that ref to click, type, \
            focus, or set values. Use slug_wait_for to await UI changes and \
            slug_list_apps to see running applications."
    })
}

/// JSON-Schema tool definitions (`tools/list`).
pub fn tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "slug_snapshot",
            "description": "Read the current UI as a Playwright-style YAML tree. \
                Each interactive node carries a short [ref=...] used by slug_invoke. \
                Apps with no accessible tree are reported as opaque (vision needed).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "scope": {
                        "type": "string",
                        "enum": ["focused", "window", "desktop"],
                        "description": "focused/window = the focused top-level window; \
                            desktop = every running application.",
                        "default": "window"
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
    ]
}

/// Dispatch a `tools/call`. The `Err` variant is a *protocol* error (bad params);
/// tool execution failures are returned as `Ok` with `isError: true`.
async fn handle_tool_call(
    session: &Arc<Session>,
    params: Value,
) -> std::result::Result<Value, (i64, String)> {
    let name = params.get("name").and_then(Value::as_str).ok_or((-32602, "missing tool name".into()))?;
    let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));

    let result = match name {
        "slug_snapshot" => tool_snapshot(session, &args).await,
        "slug_invoke" => tool_invoke(session, &args).await,
        "slug_wait_for" => tool_wait_for(session, &args).await,
        "slug_list_apps" => tool_list_apps(session).await,
        other => return Err((-32602, format!("unknown tool: {other}"))),
    };

    Ok(match result {
        Ok(text) => tool_text(text, false),
        Err(msg) => tool_text(msg, true),
    })
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

    let out = session.snapshot(scope).await.map_err(|e| e.to_string())?;
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
