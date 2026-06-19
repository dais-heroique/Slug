//! The platform-neutral accessibility backend abstraction.
//!
//! Every OS plugs in behind [`AccessibilityBackend`]: AT-SPI2 on Linux
//! ([`crate::backend_atspi`]), UI Automation on Windows ([`crate::backend_uia`]),
//! and the Accessibility (AX) API on macOS ([`crate::backend_ax`]). The
//! [`crate::Bridge`] facade selects the right backend per `cfg(target_os)` and
//! exposes the same async API to `slug-mcp` on all platforms — the semantic
//! model, the MCP server, and the agent stay platform-agnostic.
//!
//! ## Async trait
//!
//! The task brief sketches a synchronous trait, but AT-SPI is async-native (zbus)
//! and the bridge is consumed from async `slug-mcp`. The methods that do I/O are
//! therefore returned as boxed futures — the same `dyn`-compatible async-trait
//! convention already used by `slug-brain`'s `LlmBackend`. The Windows/macOS COM
//! and Core-Foundation calls are synchronous and simply run inside those futures.
//!
//! ## Node identity (brief §4)
//!
//! Each backend derives a `backend_node_id` string from its native stable
//! identity — Linux `{unique_bus_name}:{accessible_path}`, Windows the stringified
//! UIA `RuntimeId`, macOS a hash of `{pid}:{ax_tree_path}` — and hashes it into
//! the schema's ULID via [`slug_core::derive_ref`]. The Bridge addresses nodes by
//! that derived ref ([`BackendNodeId`]); each backend keeps the ref → native
//! handle mapping captured during the walk. The agent only ever sees short
//! aliases (`b1`, `e5`), exactly as before.

use std::future::Future;
use std::pin::Pin;

use slug_core::{SlugEvent, SlugNode};
use tokio::sync::mpsc;

use crate::action::Action;
use crate::coverage::Coverage;
use crate::error::{BridgeError, Result};

/// A boxed, `Send` future — the return type of the async backend methods.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A running application exposing an accessibility tree.
#[derive(Clone, Debug)]
pub struct AppHandle {
    /// Display name of the application (e.g. `Text Editor`, `Notepad`, `Finder`).
    pub app_id: String,
    /// Best-effort window/title text.
    pub title: String,
    /// The backend's native stable id for the application's root node (brief §4).
    pub backend_node_id: String,
}

/// The addressing token the [`crate::Bridge`] uses to act on a node.
///
/// It carries the node's derived ref (the ULID-shaped string). The backend maps
/// it back to the live native handle it captured during the snapshot walk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackendNodeId(pub String);

impl BackendNodeId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A sink the backend pushes live [`SlugEvent`]s into. Backends translate native
/// signals (AT-SPI `StateChanged`, UIA property/structure events, AX
/// notifications) into `SlugEvent`/`SlugDelta` and emit them here.
#[derive(Clone)]
pub struct EventSink {
    tx: mpsc::Sender<SlugEvent>,
}

impl EventSink {
    pub fn new(tx: mpsc::Sender<SlugEvent>) -> Self {
        EventSink { tx }
    }

    /// Emit an event, awaiting capacity. Returns `false` if the receiver is gone.
    pub async fn emit(&self, event: SlugEvent) -> bool {
        self.tx.send(event).await.is_ok()
    }

    /// Non-blocking emit (for callbacks that can't await, e.g. an AX/COM thread).
    /// Drops the event if the channel is full or closed.
    pub fn try_emit(&self, event: SlugEvent) -> bool {
        self.tx.try_send(event).is_ok()
    }

    /// Whether the receiver has been dropped.
    pub fn is_closed(&self) -> bool {
        self.tx.is_closed()
    }
}

/// A live subscription handle. Dropping it stops event delivery (the backend's
/// background task is aborted).
pub struct Subscription {
    task: Option<tokio::task::JoinHandle<()>>,
}

impl Subscription {
    pub fn new(task: tokio::task::JoinHandle<()>) -> Self {
        Subscription { task: Some(task) }
    }

    /// A subscription that holds nothing (e.g. backends that drive their own
    /// dedicated OS thread, which exits when its [`EventSink`] closes).
    pub fn detached() -> Self {
        Subscription { task: None }
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

/// The contract every OS accessibility backend implements.
pub trait AccessibilityBackend: Send + Sync {
    /// Short backend name for logs (`atspi` / `uia` / `ax`).
    fn label(&self) -> &'static str;

    /// Enumerate running applications that expose an accessibility tree.
    fn enumerate_apps(&self) -> BoxFuture<'_, Result<Vec<AppHandle>>>;

    /// The frontmost / focused application, if the backend can identify it
    /// cheaply. Used to make `focused`/`window` snapshots fast: only this one app
    /// is deep-walked instead of the whole desktop. The default returns `None`, so
    /// callers fall back to a full-desktop harvest (correct, just slower).
    fn focused_app(&self) -> BoxFuture<'_, Result<Option<AppHandle>>> {
        Box::pin(async move { Ok(None) })
    }

    /// Walk one application into a bounded list of [`SlugNode`]s (BFS/DFS), and
    /// capture the ref → native-handle map needed by [`AccessibilityBackend::invoke`].
    fn snapshot_app<'a>(&'a self, app: &'a AppHandle) -> BoxFuture<'a, Result<Vec<SlugNode>>>;

    /// Perform an action on the node addressed by `node_id` (its derived ref).
    fn invoke<'a>(
        &'a self,
        node_id: &'a BackendNodeId,
        action: &'a Action,
    ) -> BoxFuture<'a, Result<()>>;

    /// Inject synthetic OS input (a key chord or literal text) into the
    /// **currently focused application**, independent of any node. This is what
    /// lets the agent drive apps that expose no (or only a partial) accessibility
    /// tree — without ever capturing a pixel. Only [`Action::Key`] and
    /// [`Action::TypeText`] are valid here.
    ///
    /// The default implementation reports the platform does not support synthetic
    /// input, so backends that don't implement it compile unchanged.
    fn synth_input<'a>(&'a self, action: &'a Action) -> BoxFuture<'a, Result<()>> {
        let _ = action;
        Box::pin(async move {
            Err(BridgeError::Unsupported(
                "synthetic input is not implemented on this platform".to_string(),
            ))
        })
    }

    /// Subscribe to live events; emitted as [`SlugEvent`]s on `sink`.
    fn subscribe_events(&self, sink: EventSink) -> BoxFuture<'_, Result<Subscription>>;

    /// Opaque-app heuristic for an application (unchanged across platforms);
    /// computed from the most recent snapshot of that app.
    fn coverage(&self, app: &AppHandle) -> Coverage;
}
