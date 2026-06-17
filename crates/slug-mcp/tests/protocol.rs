//! MCP protocol-level integration tests.
//!
//! These exercise the JSON-RPC dispatch end to end through `mcp::handle` and run
//! anywhere — no accessibility bus required. Tool calls that need the bus are
//! expected to come back as `isError` tool results (never protocol errors), which
//! is exactly the contract we assert here.

use serde_json::{json, Value};
use slug_mcp::mcp::{handle, JsonRpcRequest};
use slug_mcp::Session;

fn req(id: i64, method: &str, params: Value) -> JsonRpcRequest {
    serde_json::from_value(json!({
        "jsonrpc": "2.0", "id": id, "method": method, "params": params
    }))
    .unwrap()
}

#[tokio::test]
async fn initialize_advertises_tools_capability() {
    let session = Session::new();
    let resp = handle(&session, req(1, "initialize", json!({}))).await.expect("response");
    let v = serde_json::to_value(&resp).unwrap();
    assert_eq!(v["result"]["serverInfo"]["name"], "slug-mcp");
    assert!(v["result"]["capabilities"]["tools"].is_object());
    assert!(v["result"]["protocolVersion"].is_string());
}

#[tokio::test]
async fn tools_list_exposes_the_four_tools() {
    let session = Session::new();
    let resp = handle(&session, req(2, "tools/list", json!({}))).await.expect("response");
    let v = serde_json::to_value(&resp).unwrap();
    let tools = v["result"]["tools"].as_array().expect("tools array");
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names.len(), 4);
    for expected in ["slug_snapshot", "slug_invoke", "slug_wait_for", "slug_list_apps"] {
        assert!(names.contains(&expected), "missing tool {expected}");
    }
    // Every tool must publish a JSON-Schema object input schema.
    for t in tools {
        assert_eq!(t["inputSchema"]["type"], "object", "tool {} schema", t["name"]);
    }
}

#[tokio::test]
async fn notifications_get_no_response() {
    let session = Session::new();
    let resp = handle(&session, req_notification("notifications/initialized")).await;
    assert!(resp.is_none(), "notifications must not produce a response");
}

fn req_notification(method: &str) -> JsonRpcRequest {
    serde_json::from_value(json!({ "jsonrpc": "2.0", "method": method })).unwrap()
}

#[tokio::test]
async fn unknown_method_is_a_protocol_error() {
    let session = Session::new();
    let resp = handle(&session, req(3, "does/not/exist", json!({}))).await.expect("response");
    let v = serde_json::to_value(&resp).unwrap();
    assert_eq!(v["error"]["code"], -32601);
}

#[tokio::test]
async fn tool_call_without_bus_is_an_iserror_result_not_protocol_error() {
    let session = Session::new();
    let resp = handle(
        &session,
        req(4, "tools/call", json!({ "name": "slug_list_apps", "arguments": {} })),
    )
    .await
    .expect("response");
    let v = serde_json::to_value(&resp).unwrap();
    // Protocol-level success...
    assert!(v["error"].is_null(), "must not be a protocol error");
    // ...but the tool result flags the failure.
    assert_eq!(v["result"]["isError"], true);
    assert!(v["result"]["content"][0]["text"].as_str().unwrap().contains("not connected"));
}

#[tokio::test]
async fn unknown_tool_name_is_a_protocol_error() {
    let session = Session::new();
    let resp = handle(
        &session,
        req(5, "tools/call", json!({ "name": "nope", "arguments": {} })),
    )
    .await
    .expect("response");
    let v = serde_json::to_value(&resp).unwrap();
    assert_eq!(v["error"]["code"], -32602);
}
