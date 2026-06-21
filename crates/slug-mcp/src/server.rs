//! Transports: newline-delimited JSON-RPC over stdio, and streamable HTTP.

use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing::{error, info};

use crate::agent::AgentController;
use crate::mcp::{self, JsonRpcRequest};
use crate::session::Session;

/// The static control dashboard (served at `GET /dashboard`).
const DASHBOARD_HTML: &str = include_str!("dashboard.html");

/// Run the MCP server over stdio (the transport `claude mcp add` uses by
/// default). Reads one JSON-RPC message per line from stdin; writes one JSON
/// response per line to stdout.
pub async fn run_stdio(session: Arc<Session>, control: Arc<AgentController>) -> anyhow::Result<()> {
    info!("slug-mcp listening on stdio");
    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin).lines();
    let mut stdout = tokio::io::stdout();

    while let Some(line) = reader.next_line().await? {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let req: JsonRpcRequest = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(e) => {
                error!(error = %e, "failed to parse JSON-RPC request");
                let resp = parse_error_response();
                write_line(&mut stdout, &resp).await?;
                continue;
            }
        };

        if let Some(resp) = mcp::handle_with_control(&session, Some(control.clone()), req).await {
            let body = serde_json::to_string(&resp)?;
            write_line(&mut stdout, &body).await?;
        }
    }
    info!("stdin closed; shutting down");
    Ok(())
}

async fn write_line(
    stdout: &mut tokio::io::Stdout,
    body: &str,
) -> anyhow::Result<()> {
    stdout.write_all(body.as_bytes()).await?;
    stdout.write_all(b"\n").await?;
    stdout.flush().await?;
    Ok(())
}

fn parse_error_response() -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": null,
        "error": { "code": -32700, "message": "parse error" }
    })
    .to_string()
}

/// Run the MCP server over streamable HTTP. A single `POST /mcp` endpoint accepts
/// a JSON-RPC message and returns the JSON-RPC response (or `202 Accepted` for
/// notifications). A `GET /healthz` is provided for liveness checks.
pub async fn run_http(
    session: Arc<Session>,
    control: Arc<AgentController>,
    addr: std::net::SocketAddr,
) -> anyhow::Result<()> {
    use axum::extract::State;
    use axum::http::{header, StatusCode};
    use axum::response::{Html, IntoResponse};
    use axum::routing::{get, post};
    use axum::{Json, Router};

    #[derive(Clone)]
    struct AppState {
        session: Arc<Session>,
        control: Arc<AgentController>,
    }

    async fn mcp_endpoint(
        State(state): State<AppState>,
        headers: axum::http::HeaderMap,
        body: axum::body::Bytes,
    ) -> axum::response::Response {
        // Security: this localhost server can read screen content and drive the
        // desktop, so it must not be reachable from a web page in the user's
        // browser (DNS-rebinding / CSRF). Reject any request whose Origin or Host
        // is not local. Non-browser clients (Claude Code, curl) send no Origin and
        // a local Host, so they pass.
        if !local_request_ok(&headers) {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({
                    "jsonrpc": "2.0", "id": null,
                    "error": { "code": -32600, "message": "forbidden: non-local Origin/Host rejected" }
                })),
            )
                .into_response();
        }
        let req: JsonRpcRequest = match serde_json::from_slice(&body) {
            Ok(r) => r,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "jsonrpc": "2.0", "id": null,
                        "error": { "code": -32700, "message": format!("parse error: {e}") }
                    })),
                )
                    .into_response();
            }
        };
        match mcp::handle_with_control(&state.session, Some(state.control.clone()), req).await {
            Some(resp) => Json(resp).into_response(),
            None => StatusCode::ACCEPTED.into_response(),
        }
    }

    async fn dashboard() -> impl IntoResponse {
        ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], Html(DASHBOARD_HTML))
    }

    /// List destructive actions awaiting human approval (polled by the dashboard).
    async fn list_approvals(
        State(state): State<AppState>,
        headers: axum::http::HeaderMap,
    ) -> axum::response::Response {
        if !local_request_ok(&headers) {
            return StatusCode::FORBIDDEN.into_response();
        }
        Json(state.control.approvals().list().await).into_response()
    }

    /// Approve or deny a pending destructive action. Body: `{ "id": N, "approved": bool }`.
    async fn decide_approval(
        State(state): State<AppState>,
        headers: axum::http::HeaderMap,
        Json(body): Json<serde_json::Value>,
    ) -> axum::response::Response {
        if !local_request_ok(&headers) {
            return StatusCode::FORBIDDEN.into_response();
        }
        let id = body.get("id").and_then(|v| v.as_u64());
        let approved = body.get("approved").and_then(|v| v.as_bool());
        match (id, approved) {
            (Some(id), Some(approved)) => match state.control.approvals().decide(id, approved).await {
                Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
                Err(e) => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": e }))).into_response(),
            },
            _ => (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "expected { id: number, approved: boolean }" })),
            )
                .into_response(),
        }
    }

    /// Accept a request only if it is local: any present `Origin` and `Host`
    /// header must point at loopback. This blocks cross-site / DNS-rebinding
    /// attacks while leaving local CLI clients (no Origin) untouched.
    fn local_request_ok(headers: &axum::http::HeaderMap) -> bool {
        fn host_is_local(h: &str) -> bool {
            // Strip scheme and any path, then the port — keeping bracketed IPv6
            // ([::1] / [::1]:port) intact — and compare the host only.
            let h = h.strip_prefix("http://").or_else(|| h.strip_prefix("https://")).unwrap_or(h);
            let host = h.split('/').next().unwrap_or(h);
            let host = if host.starts_with('[') {
                // Bracketed IPv6: keep up to and including ']'.
                host.find(']').map(|i| &host[..=i]).unwrap_or(host)
            } else if let Some((a, _)) = host.rsplit_once(':') {
                a
            } else {
                host
            };
            matches!(host, "127.0.0.1" | "localhost" | "[::1]" | "::1")
        }
        if let Some(origin) = headers.get(axum::http::header::ORIGIN).and_then(|v| v.to_str().ok()) {
            // `null` and any non-local origin are rejected.
            if !host_is_local(origin) {
                return false;
            }
        }
        if let Some(host) = headers.get(axum::http::header::HOST).and_then(|v| v.to_str().ok()) {
            if !host_is_local(host) {
                return false;
            }
        }
        true
    }

    let app = Router::new()
        .route("/mcp", post(mcp_endpoint))
        .route("/dashboard", get(dashboard))
        .route("/approvals", get(list_approvals))
        .route("/approve", post(decide_approval))
        .route("/healthz", get(|| async { "ok" }))
        .with_state(AppState { session, control });

    info!(%addr, "slug-mcp listening on streamable HTTP at POST /mcp (dashboard at GET /dashboard)");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
