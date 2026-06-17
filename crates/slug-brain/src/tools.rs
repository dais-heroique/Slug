//! Bridging the brain to the MCP tools.
//!
//! The brain drives the *same* tools the MCP server exposes — `slug_snapshot`,
//! `slug_invoke`, `slug_wait_for`, `slug_list_apps` — by reusing `slug-mcp`'s
//! tool definitions and dispatch. This guarantees the agent and an external MCP
//! client see identical schemas and behaviour.

use std::sync::Arc;

use serde_json::{json, Value};
use slug_mcp::mcp::{self, JsonRpcRequest};
use slug_mcp::Session;

use crate::backend::ToolSpec;

/// Max characters of a snapshot fed back to the model. Oversized accessibility
/// snapshots are a known failure mode for UI agents — we cap context here.
const MAX_SNAPSHOT_CHARS: usize = 12_000;

/// Build the [`ToolSpec`]s from the MCP server's published tool definitions.
pub fn tool_specs() -> Vec<ToolSpec> {
    mcp::tool_definitions()
        .into_iter()
        .filter_map(|def| {
            let name = def.get("name")?.as_str()?.to_string();
            let description = def.get("description").and_then(Value::as_str).unwrap_or("").to_string();
            let input_schema = def.get("inputSchema").cloned().unwrap_or_else(|| json!({"type":"object"}));
            Some(ToolSpec { name, description, input_schema })
        })
        .collect()
}

/// The outcome of executing a tool: the text result and whether it was an error.
pub struct ToolOutcome {
    pub text: String,
    pub is_error: bool,
}

/// Execute one tool call through the MCP dispatch and return its text result.
///
/// For `slug_snapshot` we default the scope to `focused` (smaller context) and
/// truncate oversized output.
pub async fn execute(session: &Arc<Session>, name: &str, args: &Value) -> ToolOutcome {
    let mut args = args.clone();
    if name == "slug_snapshot" && args.get("scope").is_none() {
        args["scope"] = json!("focused");
    }

    let req: JsonRpcRequest = match serde_json::from_value(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": { "name": name, "arguments": args },
    })) {
        Ok(r) => r,
        Err(e) => {
            return ToolOutcome { text: format!("internal error building request: {e}"), is_error: true }
        }
    };

    let Some(resp) = mcp::handle(session, req).await else {
        return ToolOutcome { text: "no response from tool".into(), is_error: true };
    };

    let v = match serde_json::to_value(&resp) {
        Ok(v) => v,
        Err(e) => return ToolOutcome { text: format!("internal error: {e}"), is_error: true },
    };

    // Protocol-level error (bad params / unknown tool).
    if let Some(err) = v.get("error") {
        let msg = err.get("message").and_then(Value::as_str).unwrap_or("protocol error");
        return ToolOutcome { text: msg.to_string(), is_error: true };
    }

    let result = &v["result"];
    let is_error = result.get("isError").and_then(Value::as_bool).unwrap_or(false);
    let mut text = result
        .get("content")
        .and_then(Value::as_array)
        .and_then(|c| c.first())
        .and_then(|b| b.get("text"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    if name == "slug_snapshot" && text.len() > MAX_SNAPSHOT_CHARS {
        text.truncate(MAX_SNAPSHOT_CHARS);
        text.push_str("\n# … snapshot truncated (focus the relevant window to narrow it) …");
    }

    ToolOutcome { text, is_error }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specs_mirror_mcp_tools() {
        let specs = tool_specs();
        let names: Vec<&str> = specs.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"slug_snapshot"));
        assert!(names.contains(&"slug_invoke"));
        assert!(names.contains(&"slug_wait_for"));
        assert!(names.contains(&"slug_list_apps"));
        for s in &specs {
            assert_eq!(s.input_schema["type"], "object");
        }
    }

    #[tokio::test]
    async fn execute_without_bus_returns_error_outcome() {
        let session = Session::new();
        let out = execute(&session, "slug_list_apps", &json!({})).await;
        assert!(out.is_error);
        assert!(out.text.contains("not connected"));
    }
}
