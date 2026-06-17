//! `slug` — a small CLI for driving the Slug semantic bus by hand.
//!
//! It reuses the same session layer as the MCP server (`slug_mcp::Session`), so
//! snapshots, ref aliases, and actions behave identically to what an agent sees.
//!
//! Examples:
//! ```text
//! slug apps                       # list accessible applications
//! slug snapshot --scope desktop   # print the YAML semantic tree
//! slug invoke b1 click            # click the node shown as [ref=b1]
//! slug invoke i1 set_text "hello" --reasoning "fill the search box"
//! slug live                       # snapshot, then stream live events
//! ```
//!
//! Note: ref aliases (`b1`, `e5`) are stable within an unchanged tree. `invoke`
//! takes a fresh desktop snapshot first to (re)build the alias table, then acts.

use std::sync::Arc;

use anyhow::Context;
use clap::{Parser, Subcommand};
use slug_mcp::{Scope, Session};

#[derive(Parser)]
#[command(name = "slug", version, about = "Drive the Slug AT-SPI semantic bus from the terminal")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List running applications exposing an accessibility tree.
    Apps,
    /// Print the current UI as a Playwright-style YAML tree.
    Snapshot {
        /// focused | window | desktop
        #[arg(long, default_value = "desktop")]
        scope: String,
    },
    /// Perform an action on a node by its ref alias (from a snapshot).
    Invoke {
        /// Node ref alias, e.g. b1 or e5.
        r#ref: String,
        /// activate | click | press | focus | set_text | set_value | toggle | ...
        action: String,
        /// Argument (text for set_text, number for set_value).
        #[arg(long)]
        args: Option<String>,
        /// Why you're taking this action (logged).
        #[arg(long)]
        reasoning: Option<String>,
    },
    /// Snapshot once, then stream live semantic events until Ctrl-C.
    Live {
        /// focused | window | desktop for the initial snapshot
        #[arg(long, default_value = "desktop")]
        scope: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    let cli = Cli::parse();
    let session = Session::new();

    match cli.cmd {
        Cmd::Apps => cmd_apps(&session).await,
        Cmd::Snapshot { scope } => cmd_snapshot(&session, &scope).await,
        Cmd::Invoke { r#ref, action, args, reasoning } => {
            cmd_invoke(&session, &r#ref, &action, args.as_deref(), reasoning.as_deref()).await
        }
        Cmd::Live { scope } => cmd_live(&session, &scope).await,
    }
}

async fn cmd_apps(session: &Arc<Session>) -> anyhow::Result<()> {
    let apps = session.list_apps().await.context("listing apps")?;
    if apps.is_empty() {
        println!("(no accessible applications found)");
        return Ok(());
    }
    for a in apps {
        println!(
            "{:<32} {}",
            if a.app_id.is_empty() { "<unnamed>" } else { &a.app_id },
            a.bus_name
        );
    }
    Ok(())
}

async fn cmd_snapshot(session: &Arc<Session>, scope: &str) -> anyhow::Result<()> {
    let scope = Scope::parse(scope).context("scope must be focused|window|desktop")?;
    let out = session.snapshot(scope).await.context("taking snapshot")?;
    print!("{}", out.yaml);
    if !out.opaque.is_empty() {
        eprintln!("\n# opaque apps (no/flat accessibility tree — vision fallback):");
        for c in &out.opaque {
            eprintln!("#   - {} ({:?})", c.app_id, c.opaque.unwrap());
        }
    }
    Ok(())
}

async fn cmd_invoke(
    session: &Arc<Session>,
    r#ref: &str,
    action: &str,
    args: Option<&str>,
    reasoning: Option<&str>,
) -> anyhow::Result<()> {
    // Build/refresh the alias table so the ref resolves.
    let _ = session.snapshot(Scope::Desktop).await.context("snapshot before invoke")?;
    let ok = session
        .invoke(r#ref, action, args, reasoning)
        .await
        .with_context(|| format!("invoking {action} on {ref}", ref = r#ref))?;
    if ok {
        println!("ok: {action} on {} succeeded", r#ref);
    } else {
        println!("note: {action} on {} dispatched, toolkit reported no effect", r#ref);
    }
    Ok(())
}

async fn cmd_live(session: &Arc<Session>, scope: &str) -> anyhow::Result<()> {
    let scope = Scope::parse(scope).context("scope must be focused|window|desktop")?;
    let out = session.snapshot(scope).await.context("initial snapshot")?;
    print!("{}", out.yaml);
    println!("\n--- streaming live events (Ctrl-C to stop) ---");

    loop {
        // Long poll for any event; print as it arrives.
        match session.wait_for(None, 60_000).await {
            Ok(Some(ev)) => {
                println!("[{}] {}", ev.type_name(), serde_json::to_string(&ev).unwrap_or_default());
            }
            Ok(None) => { /* timeout window elapsed; keep waiting */ }
            Err(e) => {
                eprintln!("event stream error: {e}");
                break;
            }
        }
    }
    Ok(())
}

fn init_tracing() {
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("slug_cli=info,slug_bridge=info"));
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_writer(std::io::stderr))
        .init();
}
