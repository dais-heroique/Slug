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

/// A server-side filter for [`Session::snapshot_filtered`]. When *active*, the
/// snapshot is rendered as a compact **flat list** of only the matching nodes
/// (each with its `ref` and centre `@x,y`) instead of the full indented tree —
/// this is the fast path that avoids shipping an 80k-char document just to find
/// one button. See [`slug_core::yaml::render_filtered`].
#[derive(Clone, Debug, Default)]
pub struct SnapshotFilter {
    /// Case-insensitive substring matched against each node's display label.
    pub query: Option<String>,
    /// Keep only these lower-case role names (e.g. `["button", "entry"]`).
    pub roles: Vec<String>,
    /// Keep only directly actionable controls (buttons, fields, links, …).
    pub interactive_only: bool,
    /// Cap on emitted nodes (defaults to [`SnapshotFilter::DEFAULT_LIMIT`]).
    pub limit: Option<usize>,
    /// Include centre `@x,y` on every match (default: only opaque surfaces, to
    /// keep the result lean — normal controls are clicked by `ref`).
    pub coords: bool,
}

impl SnapshotFilter {
    /// Default cap on nodes returned in filtered mode.
    pub const DEFAULT_LIMIT: usize = 50;

    /// Whether any constraint is set. When `false`, callers render the full tree.
    /// `limit` alone counts — a bare `{limit: 20}` should cap output, not be
    /// silently ignored because no query/roles/interactive_only came with it.
    pub fn is_active(&self) -> bool {
        self.query.is_some() || !self.roles.is_empty() || self.interactive_only || self.limit.is_some()
    }

    fn limit(&self) -> usize {
        self.limit.unwrap_or(Self::DEFAULT_LIMIT)
    }
}

/// Errors surfaced to the MCP tool layer (returned in the tool result object,
/// never as protocol errors).
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("not connected to the accessibility bus: {0}")]
    NotConnected(String),
    #[error("unknown ref alias '{0}' — refs change whenever the UI changes; re-run slug_snapshot to get fresh refs, then retry")]
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

/// A cached snapshot for one scope. We cache the **scoped sub-document** (not the
/// rendered string) so that, within the TTL, both a full-tree render and any
/// number of filtered "find" renders are served from one harvest.
struct CachedSnapshot {
    scope: Scope,
    at: Instant,
    doc: SlugDocument,
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

    /// Produce a full-tree YAML snapshot for the given scope.
    pub async fn snapshot(self: &Arc<Self>, scope: Scope) -> Result<SnapshotOutput> {
        self.snapshot_filtered(scope, &SnapshotFilter::default()).await
    }

    /// Snapshot a specific application **by name**, independent of which window the
    /// OS currently has focused. This is the reliable way to read an app you are
    /// driving from another window (the controlling client steals OS focus between
    /// calls, so `scope:"focused"` would otherwise read the wrong app).
    pub async fn snapshot_app(
        self: &Arc<Self>,
        app: &str,
        filter: &SnapshotFilter,
    ) -> Result<SnapshotOutput> {
        let bridge = self.ensure_bridge().await?;
        let harvested = bridge.snapshot_app(app).await?;
        let opaque = harvested.opaque.clone();
        let mut state = self.state.lock().await;
        state.document = harvested.document;
        let scoped = state.document.clone();
        for node in scoped.bfs_order() {
            state.aliases.assign(&node.slug_ref, node.role);
        }
        let yaml = render(&scoped, &state.aliases, filter);
        drop(state);
        // Don't write the scope cache (it's keyed by scope, not app name); a stale
        // app doc must never be served for a later focused/desktop read.
        *self.cache.lock().await = None;
        Ok(SnapshotOutput { yaml, opaque })
    }

    /// Produce a snapshot for `scope`, optionally narrowed by `filter`.
    ///
    /// When `filter.is_active()`, the result is a compact **flat list** of only
    /// the matching nodes (each with its `ref` and centre `@x,y`) — the fast path
    /// that avoids shipping the whole tree. Otherwise the full indented tree is
    /// rendered. Both share the same harvest via the short-lived scope cache.
    pub async fn snapshot_filtered(
        self: &Arc<Self>,
        scope: Scope,
        filter: &SnapshotFilter,
    ) -> Result<SnapshotOutput> {
        // Serve from the short-lived cache if a same-scope harvest is still fresh.
        let cached = {
            let cache = self.cache.lock().await;
            cache
                .as_ref()
                .filter(|c| c.scope == scope && c.at.elapsed() < SNAPSHOT_TTL)
                .map(|c| (c.doc.clone(), c.opaque.clone()))
        };
        if let Some((doc, opaque)) = cached {
            let state = self.state.lock().await;
            let yaml = render(&doc, &state.aliases, filter);
            return Ok(SnapshotOutput { yaml, opaque });
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
        let yaml = render(&scoped, &state.aliases, filter);
        drop(state);
        *self.cache.lock().await =
            Some(CachedSnapshot { scope, at: Instant::now(), doc: scoped, opaque: opaque.clone() });
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

    /// Bring an already-running app to the foreground so subsequent synthetic input
    /// lands in it (not in the controlling client's window). Does not require the
    /// accessibility bus — it's a window-server action.
    pub async fn activate(&self, name: &str) -> Result<()> {
        slug_bridge::activate::activate(name).map_err(SessionError::Bridge)?;
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
            Ok(Some(ev)) => {
                // A UI change was observed — drop the snapshot cache so the next
                // snapshot reflects it immediately.
                self.invalidate_cache().await;
                Ok(Some(self.aliasize_event(ev).await))
            }
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

/// Render a scoped document either as the full indented tree or, when the filter
/// is active, as a compact flat list of only the matching nodes.
fn render(doc: &SlugDocument, aliases: &AliasTable, filter: &SnapshotFilter) -> String {
    if filter.is_active() {
        slug_core::yaml::render_filtered(
            doc,
            aliases,
            filter.query.as_deref(),
            &filter.roles,
            filter.interactive_only,
            filter.limit(),
            filter.coords,
        )
    } else {
        doc.to_yaml(aliases)
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
    fn snapshot_filter_activity_and_limit() {
        let none = SnapshotFilter::default();
        assert!(!none.is_active());
        assert_eq!(none.limit(), SnapshotFilter::DEFAULT_LIMIT);

        let by_role = SnapshotFilter { roles: vec!["button".into()], ..Default::default() };
        assert!(by_role.is_active());

        let by_query = SnapshotFilter { query: Some("save".into()), ..Default::default() };
        assert!(by_query.is_active());

        let interactive = SnapshotFilter { interactive_only: true, ..Default::default() };
        assert!(interactive.is_active());

        let capped = SnapshotFilter { limit: Some(5), ..Default::default() };
        assert!(capped.is_active(), "a bare limit must activate filtered rendering, not be ignored");
        assert_eq!(capped.limit(), 5);
    }

    #[test]
    fn render_dispatches_full_vs_filtered() {
        let mut win = SlugNode::new("W", SlugRole::Window);
        win.name = Some("Editor".into());
        win.child_refs = vec!["B".into()];
        let mut b = SlugNode::new("B", SlugRole::Button);
        b.parent_ref = Some("W".into());
        b.name = Some("Save".into());
        b.states = vec![SlugState::Enabled];
        let doc = SlugDocument::from_nodes([win, b]);
        let mut aliases = AliasTable::new();
        for n in doc.bfs_order() {
            aliases.assign(&n.slug_ref, n.role);
        }

        // No filter → full indented tree (window present, nested).
        let full = render(&doc, &aliases, &SnapshotFilter::default());
        assert!(full.contains("- window \"Editor\""));
        assert!(full.contains("  - button \"Save\""));

        // Filter on buttons → flat list, no window line, no indentation.
        let filtered =
            render(&doc, &aliases, &SnapshotFilter { roles: vec!["button".into()], ..Default::default() });
        assert!(filtered.contains("- button \"Save\""));
        assert!(!filtered.contains("window"), "filtered output should be flat: {filtered}");
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
