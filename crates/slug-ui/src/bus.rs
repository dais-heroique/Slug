//! Bus export over a local IPC channel.
//!
//! On macOS/Linux a Unix-domain socket is used; on Windows a named pipe.
//! The app serves its semantic tree + tools and accepts `invoke`/`call_tool`
//! from the bus — the native Slug path with no AT-SPI, no screenshots.
//! Frames are length-prefixed JSON; see `protocol.rs`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;
use tokio::sync::Mutex;
use tracing::{debug, warn};

use crate::app::UiRuntime;
use crate::protocol::{read_frame, write_frame, BusSnapshot, ClientMsg, ServerMsg};

/// A shared, thread-safe handle to a running app.
pub type SharedRuntime = Arc<Mutex<Box<dyn UiRuntime>>>;

/// Wrap a runtime for sharing with the bus server.
pub fn shared(runtime: impl UiRuntime + 'static) -> SharedRuntime {
    Arc::new(Mutex::new(Box::new(runtime)))
}

// ── platform stream type ──────────────────────────────────────────────────────

#[cfg(unix)]
type BusStream = tokio::net::UnixStream;

// On Windows, NamedPipeClient implements AsyncRead + AsyncWrite + Unpin.
#[cfg(windows)]
type BusStream = tokio::net::windows::named_pipe::NamedPipeClient;

// ── Windows helper: filesystem path → named-pipe name ────────────────────────

#[cfg(windows)]
fn to_pipe_name(path: &Path) -> String {
    // Named pipes live in \\.\pipe\<name>. Derive the name from the last path
    // component so `/tmp/slug-notes.sock` → `\\.\pipe\slug-notes.sock`.
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("slug-ui");
    format!(r"\\.\pipe\{name}")
}

// ── serve ─────────────────────────────────────────────────────────────────────

/// Serve a runtime on the local IPC channel. Runs until the listener errors.
///
/// On Unix the `path` argument is the socket file path (any stale file is
/// removed first). On Windows it is converted to a named-pipe name using
/// the final path component.
#[cfg(unix)]
pub async fn serve(path: impl AsRef<Path>, runtime: SharedRuntime) -> std::io::Result<()> {
    use tokio::net::UnixListener;
    let path: PathBuf = path.as_ref().to_path_buf();
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    debug!(socket = %path.display(), "slug-ui bus listening");
    loop {
        let (stream, _) = listener.accept().await?;
        let rt = runtime.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_conn(stream, rt).await {
                debug!(error = %e, "bus connection closed");
            }
        });
    }
}

#[cfg(windows)]
pub async fn serve(path: impl AsRef<Path>, runtime: SharedRuntime) -> std::io::Result<()> {
    use tokio::net::windows::named_pipe::ServerOptions;
    let name = to_pipe_name(path.as_ref());
    debug!(pipe = %name, "slug-ui bus listening");
    // Create the first server instance before any client tries to connect.
    let mut server = ServerOptions::new().first_pipe_instance(true).create(&name)?;
    loop {
        server.connect().await?;
        let connected = server;
        let rt = runtime.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_conn(connected, rt).await {
                debug!(error = %e, "bus connection closed");
            }
        });
        // Prepare the next instance so the pipe is always available.
        server = ServerOptions::new().create(&name)?;
    }
}

// ── connection handler (shared, generic over stream type) ─────────────────────

async fn handle_conn<S>(mut stream: S, runtime: SharedRuntime) -> std::io::Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    while let Some(msg) = read_frame::<_, ClientMsg>(&mut stream).await? {
        let resp = dispatch(&runtime, msg).await;
        write_frame(&mut stream, &resp).await?;
    }
    Ok(())
}

async fn dispatch(runtime: &SharedRuntime, msg: ClientMsg) -> ServerMsg {
    let mut rt = runtime.lock().await;
    match msg {
        ClientMsg::Snapshot => ServerMsg::Snapshot(rt.snapshot()),
        ClientMsg::Invoke { slug_ref, action, args } => {
            match rt.invoke(&slug_ref, &action, args.as_deref()) {
                Ok(()) => ServerMsg::Snapshot(rt.snapshot()),
                Err(e) => {
                    warn!(%slug_ref, %action, error = %e, "invoke failed");
                    ServerMsg::InvokeResult { ok: false, error: Some(e) }
                }
            }
        }
        ClientMsg::CallTool { name, args } => match rt.call_tool(&name, args) {
            Ok(value) => ServerMsg::ToolResult { ok: true, value, error: None },
            Err(e) => ServerMsg::ToolResult { ok: false, value: Value::Null, error: Some(e) },
        },
    }
}

// ── BusClient ─────────────────────────────────────────────────────────────────

/// A minimal client for driving an app over the bus (used by tests and tools).
pub struct BusClient {
    stream: BusStream,
}

impl BusClient {
    #[cfg(unix)]
    pub async fn connect(path: impl AsRef<Path>) -> std::io::Result<Self> {
        Ok(BusClient { stream: tokio::net::UnixStream::connect(path).await? })
    }

    #[cfg(windows)]
    pub async fn connect(path: impl AsRef<Path>) -> std::io::Result<Self> {
        use tokio::net::windows::named_pipe::ClientOptions;
        let name = to_pipe_name(path.as_ref());
        Ok(BusClient { stream: ClientOptions::new().open(&name)? })
    }

    async fn round_trip(&mut self, msg: &ClientMsg) -> std::io::Result<ServerMsg> {
        write_frame(&mut self.stream, msg).await?;
        read_frame::<_, ServerMsg>(&mut self.stream)
            .await?
            .ok_or_else(|| std::io::Error::other("server closed the connection"))
    }

    /// Fetch a fresh semantic snapshot.
    pub async fn snapshot(&mut self) -> std::io::Result<BusSnapshot> {
        match self.round_trip(&ClientMsg::Snapshot).await? {
            ServerMsg::Snapshot(s) => Ok(s),
            other => Err(std::io::Error::other(format!("unexpected reply: {other:?}"))),
        }
    }

    /// Invoke an action on a node; returns the resulting snapshot.
    pub async fn invoke(
        &mut self,
        slug_ref: &str,
        action: &str,
        args: Option<&str>,
    ) -> std::io::Result<BusSnapshot> {
        let msg = ClientMsg::Invoke {
            slug_ref: slug_ref.to_string(),
            action: action.to_string(),
            args: args.map(str::to_string),
        };
        match self.round_trip(&msg).await? {
            ServerMsg::Snapshot(s) => Ok(s),
            ServerMsg::InvokeResult { error, .. } => {
                Err(std::io::Error::other(error.unwrap_or_else(|| "invoke failed".into())))
            }
            other => Err(std::io::Error::other(format!("unexpected reply: {other:?}"))),
        }
    }

    /// Call a high-level tool.
    pub async fn call_tool(&mut self, name: &str, args: Value) -> std::io::Result<Value> {
        let msg = ClientMsg::CallTool { name: name.to_string(), args };
        match self.round_trip(&msg).await? {
            ServerMsg::ToolResult { ok: true, value, .. } => Ok(value),
            ServerMsg::ToolResult { ok: false, error, .. } => {
                Err(std::io::Error::other(error.unwrap_or_else(|| "tool failed".into())))
            }
            other => Err(std::io::Error::other(format!("unexpected reply: {other:?}"))),
        }
    }
}
