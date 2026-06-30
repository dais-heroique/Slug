//! `slug-notes` binary: serve the notes editor on the slug-ui bus.
//!
//! ```text
//! slug-notes [socket-path]      # default: $SLUG_UI_SOCKET or /tmp/slug-notes.sock
//! ```
//!
//! Connect an agent/bridge to the printed Unix socket and drive it with
//! `snapshot` / `invoke` / `call_tool` — no window or screenshots required.

use slug_ui::{bus, shared};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_writer(std::io::stderr).with_env_filter(
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("slug_ui=info,slug_notes=info")),
    ).init();

    let socket = std::env::args().nth(1).unwrap_or_else(|| {
        std::env::var("SLUG_UI_SOCKET").unwrap_or_else(|_| "/tmp/slug-notes.sock".to_string())
    });

    let runtime = shared(slug_notes::build_app());
    eprintln!("[slug-notes] serving on {socket}");
    eprintln!("[slug-notes] try: snapshot / invoke / call_tool over the Unix socket");
    bus::serve(&socket, runtime).await?;
    Ok(())
}
