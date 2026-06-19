//! The bus wire protocol.
//!
//! `slug-ui` exports its semantic tree + Slug tools to `slug-bus` over a local
//! Unix socket. Frames are length-prefixed (`u32` big-endian length, then a JSON
//! body). The canonical schema is also provided as Cap'n Proto in
//! [`schema/slug_ui.capnp`](../schema/slug_ui.capnp); JSON framing is used at this
//! milestone because it drops straight into the serde-based Slug stack and needs
//! no schema compiler. The message *shapes* mirror the `.capnp` exactly, so the
//! encoder can be swapped without touching callers.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use slug_core::{SlugRole, SlugState};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// A semantic node on the wire (derived from a widget; never authored directly).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct BusNode {
    /// The agent-facing stable ref (a derived ULID-shaped string).
    #[serde(rename = "ref")]
    pub slug_ref: String,
    pub role: SlugRole,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub value: Option<String>,
    #[serde(default)]
    pub states: Vec<SlugState>,
    /// Action ids the node accepts (`click`, `set_text`, `set_value`, `toggle`, …).
    #[serde(default)]
    pub actions: Vec<String>,
    /// `[x, y, w, h]` in logical pixels.
    pub bounds: [f64; 4],
    #[serde(default)]
    pub children: Vec<String>,
}

/// A high-level imperative tool a window/widget registers (WebMCP-style),
/// beyond raw widgets.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// JSON Schema for the tool's parameters.
    pub params_schema: Value,
}

/// A full semantic snapshot of an application.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BusSnapshot {
    pub app: String,
    pub root: String,
    pub nodes: Vec<BusNode>,
    pub tools: Vec<ToolSpec>,
}

/// Agent → app messages.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    /// Request a fresh snapshot.
    Snapshot,
    /// Perform an action on a node by its ref.
    Invoke {
        #[serde(rename = "ref")]
        slug_ref: String,
        action: String,
        #[serde(default)]
        args: Option<String>,
    },
    /// Call a registered high-level tool.
    CallTool {
        name: String,
        #[serde(default)]
        args: Value,
    },
}

/// App → agent messages.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    Snapshot(BusSnapshot),
    InvokeResult { ok: bool, #[serde(default)] error: Option<String> },
    ToolResult { ok: bool, #[serde(default)] value: Value, #[serde(default)] error: Option<String> },
}

/// Maximum frame body size (4 MiB) — guards against a hostile/oversized peer.
const MAX_FRAME: u32 = 4 * 1024 * 1024;

/// Write one length-prefixed JSON frame.
pub async fn write_frame<W, T>(w: &mut W, msg: &T) -> std::io::Result<()>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let body = serde_json::to_vec(msg).map_err(std::io::Error::other)?;
    let len = u32::try_from(body.len())
        .map_err(|_| std::io::Error::other("frame too large"))?;
    w.write_all(&len.to_be_bytes()).await?;
    w.write_all(&body).await?;
    w.flush().await
}

/// Read one length-prefixed JSON frame, or `None` at clean EOF.
pub async fn read_frame<R, T>(r: &mut R) -> std::io::Result<Option<T>>
where
    R: AsyncRead + Unpin,
    T: for<'de> Deserialize<'de>,
{
    let mut len_buf = [0u8; 4];
    match r.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_be_bytes(len_buf);
    if len > MAX_FRAME {
        return Err(std::io::Error::other("frame exceeds maximum size"));
    }
    let mut body = vec![0u8; len as usize];
    r.read_exact(&mut body).await?;
    let msg = serde_json::from_slice(&body).map_err(std::io::Error::other)?;
    Ok(Some(msg))
}
