//! The session daemon layer: owns the [`Bridge`], the materialised
//! [`SlugDocument`], and the session [`AliasTable`].
//!
//! This is where step-1 rule #1 is enforced: the agent only ever sees short
//! aliases (`b1`, `e5`); ULIDs never leave this module. Snapshots render YAML via
//! the alias table, and `invoke`/`wait_for` translate aliases back to ULIDs
//! before touching the bridge.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use slug_bridge::{Bridge, Coverage};
use slug_core::{AliasTable, SlugDocument, SlugEvent, SlugNode, SlugRole, SlugState};
use tokio::sync::{broadcast, Mutex};
use tracing::{info, warn};

/// Snapshot scope requested by the agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Focused,
    Window,
    Desktop,
}

impl Scope {
    pub fn parse(s: &str) -> Option<Scope> {
        match s.trim().to_ascii_lowercase().as_str() {
            "focused" => Some(Scope::Focused),
            "window" => Some(Scope::Window),
            "desktop" => Some(Scope::Desktop),
            _ => None,
        }
    }
}

/// Errors surfaced to the MCP tool layer (returned in the tool result object,
/// never as protocol errors).
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("not connected to the AT-SPI accessibility bus: {0}")]
    NotConnected(String),
    #[error("unknown ref alias: {0}")]
    UnknownAlias(String),
    #[error(transparent)]
    Bridge(#[from] slug_bridge::BridgeError),
}

type Result<T> = std::result::Result<T, SessionError>;

struct DocState {
    document: SlugDocument,
    aliases: AliasTable,
}

/// How long a snapshot is reused before re-harvesting. Short enough that the
/// agent's post-action snapshots are always fresh (and any action invalidates the
/// cache anyway), long enough to dedupe the dashboard's rapid polling.
const SNAPSHOT_TTL: Duration = Duration::from_millis(250);

/// A cached snapshot for one scope.
struct CachedSnapshot {
    scope: Scope,
    at: Instant,
    yaml: String,
    opaque: Vec<Coverage>,
}

/// The MCP session.
pub struct Session {
    bridge: Mutex<Option<Arc<Bridge>>>,
    state: Arc<Mutex<DocState>>,
    events_tx: broadcast::Sender<SlugEvent>,
    subscribed: AtomicBool,
    cache: Mutex<Option<CachedSnapshot>>,
}

impl Session {
    /// Create a session. Does **not** connect to the bus yet — connection is
    /// lazy so the server starts even where no a11y bus is present.
    pub fn new() -> Arc<Self> {
        let (events_tx, _) = broadcast::channel(1024);
        Arc::new(Session {
            bridge: Mutex::new(None),
            state: Arc::new(Mutex::new(DocState {
                document: SlugDocument::new(),
                aliases: AliasTable::new(),
            })),
            events_tx,
            subscribed: AtomicBool::new(false),
            cache: Mutex::new(None),
        })
    }

    /// Drop any cached snapshot (called after an action mutates the UI).
    async fn invalidate_cache(&self) {
        *self.cache.lock().await = None;
    }

    /// Ensure we have a live bridge, connecting (and starting the live-event
    /// subscription) on first use.
    async fn ensure_bridge(self: &Arc<Self>) -> Result<Arc<Bridge>> {
        let mut guard = self.bridge.lock().await;
        if let Some(b) = guard.as_ref() {
            return Ok(b.clone());
        }
        let bridge = Arc::new(
            Bridge::connect().await.map_err(|e| SessionError::NotConnected(e.to_string()))?,
        );
        *guard = Some(bridge.clone());
        drop(guard);

        self.start_subscription(&bridge).await;
        Ok(bridge)
    }

    /// Start the live-event pump once: bridge mpsc → (apply to doc) + broadcast.
    async fn start_subscription(self: &Arc<Self>, bridge: &Arc<Bridge>) {
        if self.subscribed.swap(true, Ordering::SeqCst) {
            return;
        }
        let mut rx = match bridge.subscribe().await {
            Ok(rx) => rx,
            Err(e) => {
                warn!(error = %e, "could not subscribe to live events");
                self.subscribed.store(false, Ordering::SeqCst);
                return;
            }
        };
        let state = self.state.clone();
        let tx = self.events_tx.clone();
        tokio::spawn(async move {
            info!("live event pump started");
            while let Some(event) = rx.recv().await {
                apply_event(&state, &event).await;
                let _ = tx.send(event); // ignore: no subscribers is fine
            }
            warn!("live event pump ended");
        });
    }

    /// Produce a YAML snapshot for the given scope. Re-harvests, refreshes the
    /// alias table, and renders Playwright-MCP-style YAML (aliases only).
    pub async fn snapshot(self: &Arc<Self>, scope: Scope) -> Result<SnapshotOutput> {
        // Serve from the short-lived cache if a same-scope snapshot is still fresh.
        {
            let cache = self.cache.lock().await;
            if let Some(c) = cache.as_ref() {
                if c.scope == scope && c.at.elapsed() < SNAPSHOT_TTL {
                    return Ok(SnapshotOutput { yaml: c.yaml.clone(), opaque: c.opaque.clone() });
                }
            }
        }

        let bridge = self.ensure_bridge().await?;
        // Fast path: focused/window only deep-walks the frontmost app, not the
        // whole desktop — the main latency win.
        let harvested = match scope {
            Scope::Desktop => bridge.snapshot_desktop().await?,
            Scope::Focused | Scope::Window => bridge.snapshot_focused().await?,
        };
        let opaque = harvested.opaque.clone();

        let scoped = select_scope(&harvested.document, scope);

        let mut state = self.state.lock().await;
        // Replace the materialised tree with the freshly-harvested desktop so the
        // action index (bridge side) and our document agree.
        state.document = harvested.document;
        // Aliases persist across snapshots (stable for the session) but we ensure
        // every currently-visible node has one.
        for node in scoped.bfs_order() {
            state.aliases.assign(&node.slug_ref, node.role);
        }
        let yaml = scoped.to_yaml(&state.aliases);
        drop(state);
        *self.cache.lock().await =
            Some(CachedSnapshot { scope, at: Instant::now(), yaml: yaml.clone(), opaque: opaque.clone() });
        Ok(SnapshotOutput { yaml, opaque })
    }

    /// Execute an action on a node identified by its **alias**.
    pub async fn invoke(
        self: &Arc<Self>,
        alias: &str,
        action: &str,
        args: Option<&str>,
        reasoning: Option<&str>,
    ) -> Result<bool> {
        let bridge = self.ensure_bridge().await?;
        let slug_ref = {
            let state = self.state.lock().await;
            state
                .aliases
                .ref_for(alias)
                .map(str::to_string)
                .ok_or_else(|| SessionError::UnknownAlias(alias.to_string()))?
        };
        let ok = bridge.invoke(&slug_ref, action, args, reasoning).await?;
        self.invalidate_cache().await; // the UI may have changed
        Ok(ok)
    }

    /// Inject synthetic OS input into the focused app. Drives **any** app —
    /// including opaque ones with no accessibility tree — with no pixels involved.
    /// If `focus_alias` is given, that node is focused first (so keys land in the
    /// right field); otherwise the input goes to whatever the OS has focused.
    pub async fn synth(
        self: &Arc<Self>,
        verb: &str,
        args: Option<&str>,
        focus_alias: Option<&str>,
        reasoning: Option<&str>,
    ) -> Result<bool> {
        let bridge = self.ensure_bridge().await?;
        if let Some(alias) = focus_alias {
            let slug_ref = {
                let state = self.state.lock().await;
                state
                    .aliases
                    .ref_for(alias)
                    .map(str::to_string)
                    .ok_or_else(|| SessionError::UnknownAlias(alias.to_string()))?
            };
            bridge.invoke(&slug_ref, "focus", None, reasoning).await?;
        }
        let ok = bridge.synth_input(verb, args, reasoning).await?;
        self.invalidate_cache().await; // input may have changed the UI
        Ok(ok)
    }

    /// Launch an application by name (and optionally open a URI / deep link with
    /// it) — e.g. "open Spotify". Does not require the accessibility bus.
    pub async fn launch(&self, name: &str, uri: Option<&str>) -> Result<()> {
        slug_bridge::launch::launch(name, uri).map_err(SessionError::Bridge)?;
        self.invalidate_cache().await;
        Ok(())
    }

    /// List running accessible applications.
    pub async fn list_apps(self: &Arc<Self>) -> Result<Vec<slug_bridge::AppInfo>> {
        let bridge = self.ensure_bridge().await?;
        Ok(bridge.list_apps().await?)
    }

    /// Wait for the next event of `event_type` (or any event if `None`), up to
    /// `timeout_ms`. Returns the event with refs rewritten to aliases, or `None`
    /// on timeout.
    pub async fn wait_for(
        self: &Arc<Self>,
        event_type: Option<&str>,
        timeout_ms: u64,
    ) -> Result<Option<SlugEvent>> {
        // Ensure the subscription is running.
        self.ensure_bridge().await?;
        let mut rx = self.events_tx.subscribe();
        let deadline = tokio::time::Duration::from_millis(timeout_ms);

        let fut = async {
            loop {
                match rx.recv().await {
                    Ok(ev) => {
                        let matches = match event_type {
                            None => true,
                            Some(t) => t == ev.type_name(),
                        };
                        if matches {
                            return Some(ev);
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        };

        match tokio::time::timeout(deadline, fut).await {
            Ok(Some(ev)) => Ok(Some(self.aliasize_event(ev).await)),
            Ok(None) | Err(_) => Ok(None),
        }
    }

    /// Rewrite the ULID refs inside an event to session aliases (assigning new
    /// ones if needed), so events never leak ULIDs to the agent.
    async fn aliasize_event(self: &Arc<Self>, ev: SlugEvent) -> SlugEvent {
        let mut state = self.state.lock().await;
        let alias_of = |st: &mut DocState, r: &str, role: SlugRole| st.aliases.assign(r, role);
        match ev {
            SlugEvent::NodeCreated { mut node } => {
                let a = alias_of(&mut state, &node.slug_ref, node.role);
                node.slug_ref = a;
                if let Some(p) = &node.parent_ref {
                    if let Some(pa) = state.aliases.alias_for(p) {
                        node.parent_ref = Some(pa.to_string());
                    }
                }
                SlugEvent::NodeCreated { node }
            }
            SlugEvent::NodeDestroyed { slug_ref } => SlugEvent::NodeDestroyed {
                slug_ref: state.aliases.alias_for(&slug_ref).unwrap_or(&slug_ref).to_string(),
            },
            SlugEvent::NodeUpdated { mut patch } => {
                if let Some(a) = state.aliases.alias_for(&patch.slug_ref) {
                    patch.slug_ref = a.to_string();
                }
                SlugEvent::NodeUpdated { patch }
            }
            SlugEvent::FocusChanged { slug_ref } => SlugEvent::FocusChanged {
                slug_ref: slug_ref
                    .map(|r| state.aliases.alias_for(&r).unwrap_or(&r).to_string()),
            },
            other => other,
        }
    }
}

/// The result of a snapshot.
pub struct SnapshotOutput {
    pub yaml: String,
    pub opaque: Vec<Coverage>,
}

/// Apply a live event to the materialised document (kept in sync for snapshots).
async fn apply_event(state: &Arc<Mutex<DocState>>, event: &SlugEvent) {
    let mut state = state.lock().await;
    match event {
        SlugEvent::NodeCreated { node } => {
            state.document.insert(node.clone());
            state.document.recompute_roots();
        }
        SlugEvent::NodeDestroyed { slug_ref } => {
            let delta = slug_core::SlugDelta {
                destroyed: vec![slug_ref.clone()],
                ..Default::default()
            };
            state.document.apply_delta(&delta);
        }
        SlugEvent::NodeUpdated { patch } => {
            let delta = slug_core::SlugDelta { updated: vec![patch.clone()], ..Default::default() };
            state.document.apply_delta(&delta);
        }
        _ => {}
    }
}

/// Select the sub-document to render for a scope.
///
/// * `Desktop` → the whole document.
/// * `Window`/`Focused` → the top-level window subtree containing the focused
///   node; falls back to the whole document if nothing is focused.
fn select_scope(doc: &SlugDocument, scope: Scope) -> SlugDocument {
    if scope == Scope::Desktop {
        return doc.clone();
    }

    // Find a focused node.
    let focused = doc.iter().find(|n| n.has_state(SlugState::Focused));
    let Some(focused) = focused else {
        return doc.clone();
    };

    // Walk up to the top-level window (or the highest ancestor we can reach).
    let mut top = focused.slug_ref.clone();
    let mut current = focused;
    while let Some(parent_ref) = &current.parent_ref {
        let Some(parent) = doc.get(parent_ref) else { break };
        top = parent.slug_ref.clone();
        if matches!(parent.role, SlugRole::Window | SlugRole::Dialog) {
            break;
        }
        current = parent;
    }

    // Collect the subtree rooted at `top`.
    let mut nodes: Vec<SlugNode> = Vec::new();
    let mut stack = vec![top];
    while let Some(r) = stack.pop() {
        if let Some(n) = doc.get(&r) {
            nodes.push(n.clone());
            stack.extend(n.child_refs.iter().cloned());
        }
    }
    let mut sub = SlugDocument::from_nodes(nodes);
    sub.recompute_roots();
    sub
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_parsing() {
        assert_eq!(Scope::parse("Desktop"), Some(Scope::Desktop));
        assert_eq!(Scope::parse("focused"), Some(Scope::Focused));
        assert_eq!(Scope::parse("nope"), None);
    }

    #[test]
    fn select_scope_focuses_window_subtree() {
        let mut win = SlugNode::new("W", SlugRole::Window);
        win.child_refs = vec!["B".into()];
        let mut b = SlugNode::new("B", SlugRole::Button);
        b.parent_ref = Some("W".into());
        b.states = vec![SlugState::Focused];
        // A second, unfocused window that should be excluded.
        let other = SlugNode::new("O", SlugRole::Window);
        let doc = SlugDocument::from_nodes([win, b, other]);

        let scoped = select_scope(&doc, Scope::Focused);
        assert!(scoped.get("W").is_some());
        assert!(scoped.get("B").is_some());
        assert!(scoped.get("O").is_none());
    }
}
