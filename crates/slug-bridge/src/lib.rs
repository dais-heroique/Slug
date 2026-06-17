//! # slug-bridge
//!
//! The AT-SPI2 harvester: connects to the accessibility bus, walks application
//! trees into a [`slug_core::SlugDocument`], executes actions, and streams live
//! semantic events.
//!
//! ## Milestone-1 adaptation (rule #2)
//!
//! Live [`slug_core::SlugDelta`]/[`slug_core::SlugEvent`] frames are produced from
//! AT-SPI2 signals (`StateChanged`, `ChildrenChanged`, focus) rather than Wayland
//! frame commits — see [`events`]. Node refs are the step-1 derived ULIDs
//! (`{unique_bus_name}:{accessible_path}`) minted in [`harvest`].

pub mod actions;
pub mod coverage;
pub mod error;
pub mod events;
pub mod harvest;
pub mod mapping;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use atspi::proxy::accessible::ObjectRefExt;
use atspi::{AccessibilityConnection, ObjectRefOwned};
use serde::Serialize;
use slug_core::{SlugDocument, SlugEvent};
use tokio::sync::mpsc;
use tracing::{info, instrument};

pub use coverage::{Coverage, OpaqueReason};
pub use error::{BridgeError, Result};
pub use harvest::Harvest;

/// Summary of a running accessible application (from the registry).
#[derive(Clone, Debug, Serialize)]
pub struct AppInfo {
    /// Accessible name of the application (e.g. `gnome-text-editor`, `Firefox`).
    pub app_id: String,
    /// The application's stable Slug ref.
    pub app_ref: String,
    /// The D-Bus unique name hosting the application.
    pub bus_name: String,
}

/// The bridge to the AT-SPI2 accessibility bus.
pub struct Bridge {
    conn: AccessibilityConnection,
    /// ref → AT-SPI object handle, refreshed on every harvest.
    index: Arc<Mutex<HashMap<String, ObjectRefOwned>>>,
}

impl Bridge {
    /// Connect to the running a11y bus. Enables accessibility for this session if
    /// it is not already on.
    pub async fn connect() -> Result<Self> {
        // Best-effort: ask the session to enable a11y so toolkits expose trees.
        if let Err(e) = atspi::connection::set_session_accessibility(true).await {
            tracing::debug!(error = %e, "could not toggle session accessibility (continuing)");
        }
        let conn = AccessibilityConnection::new().await?;
        info!("connected to AT-SPI accessibility bus");
        Ok(Bridge { conn, index: Arc::new(Mutex::new(HashMap::new())) })
    }

    /// The underlying accessibility connection.
    pub fn connection(&self) -> &AccessibilityConnection {
        &self.conn
    }

    /// List the applications currently registered on the a11y bus.
    pub async fn list_apps(&self) -> Result<Vec<AppInfo>> {
        let root = self.conn.root_accessible_on_registry().await?;
        let children = root.get_children().await?;
        let mut apps = Vec::new();
        for child in children {
            if child.is_null() {
                continue;
            }
            let app_ref = harvest::obj_ref(&child);
            let bus_name = child.name_as_str().unwrap_or("").to_string();
            let app_id = match child.as_accessible_proxy(self.conn.connection()).await {
                Ok(p) => p.name().await.unwrap_or_default(),
                Err(_) => String::new(),
            };
            apps.push(AppInfo { app_id, app_ref, bus_name });
        }
        Ok(apps)
    }

    /// Harvest the entire desktop (all applications) into a fresh document and
    /// refresh the internal action index.
    #[instrument(skip(self))]
    pub async fn snapshot_desktop(&self) -> Result<SnapshotResult> {
        let harvest = harvest::harvest_desktop(&self.conn).await?;
        Ok(self.commit_harvest(harvest))
    }

    /// Harvest a single application subtree by its bus name.
    #[instrument(skip(self))]
    pub async fn snapshot_app(&self, bus_name: &str) -> Result<SnapshotResult> {
        let root = self.conn.root_accessible_on_registry().await?;
        let children = root.get_children().await?;
        let app = children
            .into_iter()
            .find(|c| c.name_as_str() == Some(bus_name))
            .ok_or_else(|| BridgeError::UnknownRef(bus_name.to_string()))?;
        let harvest = harvest::harvest_apps(self.conn.connection(), &[app]).await?;
        Ok(self.commit_harvest(harvest))
    }

    /// Store a harvest's index and return the document + coverage to the caller.
    fn commit_harvest(&self, harvest: Harvest) -> SnapshotResult {
        let opaque: Vec<Coverage> =
            harvest.coverage.iter().filter(|c| c.is_opaque()).cloned().collect();
        {
            let mut idx = self.index.lock().expect("index mutex poisoned");
            // Merge rather than replace so refs from other scopes remain valid.
            idx.extend(harvest.index);
        }
        SnapshotResult { document: harvest.document, coverage: harvest.coverage, opaque }
    }

    /// Execute an action on a node identified by its (internal) Slug ref.
    ///
    /// `verb` is an action verb (`click`, `set_text`, `set_value`, `focus`, …);
    /// `arg` is its optional argument; `reasoning` is the agent's rationale, which
    /// is logged with the action.
    #[instrument(skip(self), fields(reasoning = reasoning.unwrap_or("")))]
    pub async fn invoke(
        &self,
        slug_ref: &str,
        verb: &str,
        arg: Option<&str>,
        reasoning: Option<&str>,
    ) -> Result<bool> {
        let objref = {
            let idx = self.index.lock().expect("index mutex poisoned");
            idx.get(slug_ref).cloned()
        }
        .ok_or_else(|| BridgeError::UnknownRef(slug_ref.to_string()))?;

        let action = actions::Action::parse(verb, arg)?;
        actions::invoke(self.conn.connection(), &objref, &action, reasoning).await
    }

    /// Subscribe to live semantic events. Opens a dedicated accessibility
    /// connection for the subscription so harvesting and eventing don't contend.
    pub async fn subscribe(&self) -> Result<mpsc::Receiver<SlugEvent>> {
        let conn = AccessibilityConnection::new().await?;
        events::subscribe(conn).await
    }

    /// Whether a ref is currently known to the action index.
    pub fn knows_ref(&self, slug_ref: &str) -> bool {
        self.index.lock().expect("index mutex poisoned").contains_key(slug_ref)
    }
}

/// The result of a harvest exposed to callers.
pub struct SnapshotResult {
    /// The materialised semantic tree.
    pub document: SlugDocument,
    /// Coverage report for every application harvested.
    pub coverage: Vec<Coverage>,
    /// Just the applications flagged opaque (vision-fallback candidates).
    pub opaque: Vec<Coverage>,
}
