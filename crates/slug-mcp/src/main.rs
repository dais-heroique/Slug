//! `slug-mcp` binary entrypoint.
//!
//! Transports:
//! * default / `--stdio`      → newline-delimited JSON-RPC over stdio
//! * `--http [ADDR]`          → streamable HTTP (default `127.0.0.1:7333`)
//!
//! Logging goes to **stderr** (stdout is reserved for the stdio JSON-RPC stream).
//! Control verbosity with `RUST_LOG`, e.g. `RUST_LOG=slug=debug`.

use std::net::SocketAddr;

use slug_mcp::{server, AgentController, Session};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let mut args = std::env::args().skip(1);
    let mode = args.next();

    // Load saved provider API keys (~/.slug/secrets.env) into the environment so
    // the brain/agent use them without re-entry. The real environment wins.
    slug_mcp::dashboard_api::load_secrets_into_env();

    let session = Session::new();
    let control = AgentController::new();

    match mode.as_deref() {
        Some("--http") => {
            let addr: SocketAddr = args
                .next()
                .unwrap_or_else(|| "127.0.0.1:7333".to_string())
                .parse()?;
            server::run_http(session, control, addr).await
        }
        Some("--stdio") | None => server::run_stdio(session, control).await,
        Some(other) => {
            eprintln!("unknown argument: {other}\nusage: slug-mcp [--stdio | --http [ADDR]]");
            std::process::exit(2);
        }
    }
}

fn init_tracing() {
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("slug_mcp=info,slug_bridge=info,slug_core=info"));
    tracing_subscriber::registry()
        .with(filter)
        // stderr: stdout is the JSON-RPC channel in stdio mode.
        .with(fmt::layer().with_writer(std::io::stderr))
        .init();
}
