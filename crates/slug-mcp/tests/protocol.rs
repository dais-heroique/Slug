//! MCP protocol-level integration tests.
//!
//! These exercise the JSON-RPC dispatch end to end through `mcp::handle` and run
//! anywhere — no accessibility bus required. Tool calls that need the bus are
//! expected to come back as `isError` tool results (never protocol errors), which
//! is exactly the contract we assert here.

use serde_json::{json, Value};
use slug_mcp::mcp::{handle, handle_with_control, JsonRpcRequest};
use slug_mcp::{AgentController, Session};

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
async fn tools_list_exposes_the_perception_and_agent_tools() {
    let session = Session::new();
    let resp = handle(&session, req(2, "tools/list", json!({}))).await.expect("response");
    let v = serde_json::to_value(&resp).unwrap();
    let tools = v["result"]["tools"].as_array().expect("tools array");
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    // The four perception/action tools …
    for expected in [
        "slug_snapshot", "slug_invoke", "slug_launch", "slug_click", "slug_scroll",
        "slug_key", "slug_activate", "slug_sequence", "slug_wait_for", "slug_list_apps",
    ] {
        assert!(names.contains(&expected), "missing tool {expected}");
    }
    // … plus the M2.5 agent-control tools.
    for expected in [
        "slug_agent_start_task",
        "slug_agent_status",
        "slug_agent_pause",
        "slug_agent_resume",
        "slug_agent_stop",
    ] {
        assert!(names.contains(&expected), "missing agent tool {expected}");
    }
    // Every tool must publish a JSON-Schema object input schema.
    for t in tools {
        assert_eq!(t["inputSchema"]["type"], "object", "tool {} schema", t["name"]);
    }
}

#[tokio::test]
async fn snapshot_tool_clarifies_it_is_not_a_screenshot() {
    let session = Session::new();
    let resp = handle(&session, req(3, "tools/list", json!({}))).await.expect("response");
    let v = serde_json::to_value(&resp).unwrap();
    let tools = v["result"]["tools"].as_array().unwrap();
    let snap = tools.iter().find(|t| t["name"] == "slug_snapshot").unwrap();
    let desc = snap["description"].as_str().unwrap();
    assert!(desc.contains("NOT a screenshot"), "snapshot must disclaim screenshots");
}

#[tokio::test]
async fn slug_help_is_listed_and_returns_a_cheat_sheet_without_a_bus() {
    let session = Session::new();
    // listed
    let resp = handle(&session, req(30, "tools/list", json!({}))).await.expect("response");
    let v = serde_json::to_value(&resp).unwrap();
    let names: Vec<&str> =
        v["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"slug_help"), "slug_help should be listed");
    // callable without the accessibility bus — it's static and must not error.
    let resp = handle(
        &session,
        req(31, "tools/call", json!({ "name": "slug_help", "arguments": {} })),
    )
    .await
    .expect("response");
    let v = serde_json::to_value(&resp).unwrap();
    assert!(v["error"].is_null());
    assert_eq!(v["result"]["isError"], false);
    let text = v["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("slug_snapshot") && text.contains("slug_invoke"), "cheat sheet incomplete");
}

#[tokio::test]
async fn slug_status_is_listed_and_reports_without_a_bus() {
    let session = Session::new();
    let resp = handle(&session, req(32, "tools/list", json!({}))).await.expect("response");
    let v = serde_json::to_value(&resp).unwrap();
    let names: Vec<&str> =
        v["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"slug_status"), "slug_status should be listed");
    // callable without the accessibility bus and without an agent controller —
    // it reports unreachable state as text, never as a protocol/tool error.
    let resp = handle(
        &session,
        req(33, "tools/call", json!({ "name": "slug_status", "arguments": {} })),
    )
    .await
    .expect("response");
    let v = serde_json::to_value(&resp).unwrap();
    assert!(v["error"].is_null());
    assert_eq!(v["result"]["isError"], false);
    let text = v["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("Slug v"), "status report missing version line");
    assert!(text.contains("Brain:"), "status report missing brain line");
    assert!(text.contains("Accessibility bus:"), "status report missing bus line");
    assert!(text.contains("Agent control: not available"), "status report should note no controller");
}

#[tokio::test]
async fn snapshot_tool_advertises_server_side_filter() {
    let session = Session::new();
    let resp = handle(&session, req(9, "tools/list", json!({}))).await.expect("response");
    let v = serde_json::to_value(&resp).unwrap();
    let tools = v["result"]["tools"].as_array().unwrap();
    let snap = tools.iter().find(|t| t["name"] == "slug_snapshot").unwrap();
    let props = &snap["inputSchema"]["properties"];
    // The fast-path filter params must be discoverable by the model.
    assert!(props["filter"].is_object(), "filter param missing");
    assert!(props["roles"].is_object(), "roles param missing");
    assert!(props["interactive_only"].is_object(), "interactive_only param missing");
    assert!(props["limit"].is_object(), "limit param missing");
    assert_eq!(props["roles"]["type"], "array");
}

#[tokio::test]
async fn snapshot_advertises_app_targeting_to_beat_focus_theft() {
    // The model must be able to discover the focus-independent `app` param (the fix
    // for "scope:focused always returns my own window").
    let session = Session::new();
    let resp = handle(&session, req(50, "tools/list", json!({}))).await.expect("response");
    let v = serde_json::to_value(&resp).unwrap();
    let tools = v["result"]["tools"].as_array().unwrap();
    let snap = tools.iter().find(|t| t["name"] == "slug_snapshot").unwrap();
    assert!(snap["inputSchema"]["properties"]["app"].is_object(), "app param missing");

    // Calling it by app name still needs the bus → clean isError, not a panic.
    let resp = handle(
        &session,
        req(51, "tools/call", json!({ "name": "slug_snapshot", "arguments": { "app": "Notes" } })),
    )
    .await
    .expect("response");
    let v = serde_json::to_value(&resp).unwrap();
    assert!(v["error"].is_null());
    assert_eq!(v["result"]["isError"], true);
}

#[tokio::test]
async fn filtered_snapshot_without_bus_is_an_iserror_result() {
    // A filtered snapshot still needs the bus; it must fail as an isError tool
    // result (not a protocol error), exactly like an unfiltered one.
    let session = Session::new();
    let resp = handle(
        &session,
        req(
            10,
            "tools/call",
            json!({
                "name": "slug_snapshot",
                "arguments": { "scope": "focused", "filter": "basket", "roles": ["button"] }
            }),
        ),
    )
    .await
    .expect("response");
    let v = serde_json::to_value(&resp).unwrap();
    assert!(v["error"].is_null(), "must not be a protocol error");
    assert_eq!(v["result"]["isError"], true);
}

#[tokio::test]
async fn sequence_rejects_an_empty_step_list_as_a_protocol_error_free_iserror() {
    // A malformed sequence (no steps) is a clean isError tool result, never a panic
    // or protocol error — the contract every tool follows.
    let session = Session::new();
    let resp = handle(
        &session,
        req(40, "tools/call", json!({ "name": "slug_sequence", "arguments": { "steps": [] } })),
    )
    .await
    .expect("response");
    let v = serde_json::to_value(&resp).unwrap();
    assert!(v["error"].is_null(), "must not be a protocol error");
    assert_eq!(v["result"]["isError"], true);
    assert!(v["result"]["content"][0]["text"].as_str().unwrap().contains("empty"));
}

#[tokio::test]
async fn sequence_with_a_wait_only_step_runs_without_a_bus() {
    // A {wait_ms} step needs neither the bus nor focus, so a wait-only sequence
    // succeeds end to end — proving steps execute in-process, atomically.
    let session = Session::new();
    let resp = handle(
        &session,
        req(
            41,
            "tools/call",
            json!({ "name": "slug_sequence", "arguments": { "steps": [ { "wait_ms": 1 } ] } }),
        ),
    )
    .await
    .expect("response");
    let v = serde_json::to_value(&resp).unwrap();
    assert!(v["error"].is_null());
    assert_eq!(v["result"]["isError"], false);
    assert!(v["result"]["content"][0]["text"].as_str().unwrap().contains("ran 1 steps"));
}

#[tokio::test]
async fn sequence_advertises_the_atomic_focus_fix() {
    // The model must be able to discover that slug_sequence is the atomic combo
    // that prevents focus theft.
    let session = Session::new();
    let resp = handle(&session, req(42, "tools/list", json!({}))).await.expect("response");
    let v = serde_json::to_value(&resp).unwrap();
    let tools = v["result"]["tools"].as_array().unwrap();
    let seq = tools.iter().find(|t| t["name"] == "slug_sequence").unwrap();
    let desc = seq["description"].as_str().unwrap();
    assert!(desc.contains("atomic") && desc.contains("focus"), "sequence must explain the fix");
    assert_eq!(seq["inputSchema"]["properties"]["steps"]["type"], "array");
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
async fn launch_without_name_or_uri_is_an_iserror_result() {
    let session = Session::new();
    let resp = handle(
        &session,
        req(6, "tools/call", json!({ "name": "slug_launch", "arguments": {} })),
    )
    .await
    .expect("response");
    let v = serde_json::to_value(&resp).unwrap();
    assert!(v["error"].is_null(), "must not be a protocol error");
    assert_eq!(v["result"]["isError"], true);
    assert!(v["result"]["content"][0]["text"].as_str().unwrap().contains("provide"));
}

#[tokio::test]
async fn click_without_coords_is_an_iserror_result() {
    let session = Session::new();
    let resp = handle(
        &session,
        req(7, "tools/call", json!({ "name": "slug_click", "arguments": { "x": 10 } })),
    )
    .await
    .expect("response");
    let v = serde_json::to_value(&resp).unwrap();
    assert!(v["error"].is_null(), "must not be a protocol error");
    assert_eq!(v["result"]["isError"], true);
    assert!(v["result"]["content"][0]["text"].as_str().unwrap().contains("'y'"));
}

#[tokio::test]
async fn key_without_keys_is_an_iserror_result() {
    let session = Session::new();
    let resp = handle(
        &session,
        req(8, "tools/call", json!({ "name": "slug_key", "arguments": {} })),
    )
    .await
    .expect("response");
    let v = serde_json::to_value(&resp).unwrap();
    assert!(v["error"].is_null(), "must not be a protocol error");
    assert_eq!(v["result"]["isError"], true);
}

#[tokio::test]
async fn destructive_invoke_is_gated_for_external_clients() {
    // With a controller attached (the daemon path), destructive actions are
    // enforced server-side. Set deny mode so the test is deterministic and never
    // blocks waiting for a human.
    std::env::set_var("SLUG_DESTRUCTIVE", "deny");
    let session = Session::new();
    let control = AgentController::new();

    // A destructive invoke is blocked by policy BEFORE it ever touches the bus.
    let resp = handle_with_control(
        &session,
        Some(control.clone()),
        req(
            20,
            "tools/call",
            json!({ "name": "slug_invoke",
                "arguments": { "ref": "b1", "action": "click", "reasoning": "delete the account" } }),
        ),
    )
    .await
    .expect("response");
    let v = serde_json::to_value(&resp).unwrap();
    assert!(v["error"].is_null());
    assert_eq!(v["result"]["isError"], true);
    let t = v["result"]["content"][0]["text"].as_str().unwrap();
    assert!(t.contains("denied"), "destructive action should be denied; got: {t}");

    // A benign invoke is NOT gated: it passes the gate and fails only at the bus.
    let resp = handle_with_control(
        &session,
        Some(control),
        req(
            21,
            "tools/call",
            json!({ "name": "slug_invoke",
                "arguments": { "ref": "b1", "action": "focus", "reasoning": "focus the field" } }),
        ),
    )
    .await
    .expect("response");
    let v = serde_json::to_value(&resp).unwrap();
    assert_eq!(v["result"]["isError"], true);
    let t = v["result"]["content"][0]["text"].as_str().unwrap();
    assert!(t.contains("not connected"), "benign action should reach the bus; got: {t}");

    std::env::remove_var("SLUG_DESTRUCTIVE");
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
