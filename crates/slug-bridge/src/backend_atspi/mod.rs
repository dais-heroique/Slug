//! Linux accessibility backend: AT-SPI2 over D-Bus.
//!
//! This is the original, unchanged AT-SPI harvester ([`harvest`], [`mapping`],
//! [`events`], [`actions`]) wrapped behind the platform-neutral
//! [`AccessibilityBackend`] trait. All Linux behaviour — role/state mapping,
//! the bounded tree walk, action execution, the live event stream, and the
//! opaque-app heuristic — is preserved.

pub mod actions;
pub mod events;
pub mod harvest;
pub mod mapping;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use atspi::proxy::accessible::ObjectRefExt;
use atspi::{AccessibilityConnection, ObjectRefOwned};
use slug_core::SlugNode;
use tracing::info;

use crate::action::Action;
use crate::backend::{AccessibilityBackend, AppHandle, BackendNodeId, BoxFuture, EventSink, Subscription};
use crate::coverage::{self, Coverage};
use crate::error::{BridgeError, Result};

/// The AT-SPI2 backend.
pub struct AtspiBackend {
    conn: AccessibilityConnection,
    /// derived ref → live AT-SPI handle, captured during each snapshot.
    handles: Arc<Mutex<HashMap<String, ObjectRefOwned>>>,
    /// app backend_node_id → that app's root AT-SPI handle.
    app_handles: Arc<Mutex<HashMap<String, ObjectRefOwned>>>,
    /// app backend_node_id → coverage from its most recent snapshot.
    coverage: Arc<Mutex<HashMap<String, Coverage>>>,
}

impl AtspiBackend {
    /// Connect to the running a11y bus, enabling session accessibility (best
    /// effort) so toolkits expose their trees.
    pub async fn connect() -> Result<Self> {
        if let Err(e) = atspi::connection::set_session_accessibility(true).await {
            tracing::debug!(error = %e, "could not toggle session accessibility (continuing)");
        }
        let conn = AccessibilityConnection::new().await?;
        info!("connected to AT-SPI accessibility bus");
        Ok(AtspiBackend {
            conn,
            handles: Arc::new(Mutex::new(HashMap::new())),
            app_handles: Arc::new(Mutex::new(HashMap::new())),
            coverage: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Resolve an application's root object-ref, from cache or by re-querying the
    /// registry (matching the stable backend node id).
    async fn app_objref(&self, backend_node_id: &str) -> Result<ObjectRefOwned> {
        if let Some(objref) = self.app_handles.lock().expect("mutex").get(backend_node_id).cloned() {
            return Ok(objref);
        }
        let root = self.conn.root_accessible_on_registry().await?;
        let children = root.get_children().await?;
        for child in children {
            if child.is_null() {
                continue;
            }
            if harvest::native_id(&child) == backend_node_id {
                self.app_handles
                    .lock()
                    .expect("mutex")
                    .insert(backend_node_id.to_string(), child.clone());
                return Ok(child);
            }
        }
        Err(BridgeError::UnknownRef(backend_node_id.to_string()))
    }
}

impl AccessibilityBackend for AtspiBackend {
    fn label(&self) -> &'static str {
        "atspi"
    }

    fn enumerate_apps(&self) -> BoxFuture<'_, Result<Vec<AppHandle>>> {
        Box::pin(async move {
            let root = self.conn.root_accessible_on_registry().await?;
            let children = root.get_children().await?;
            let mut apps = Vec::new();
            for child in children {
                if child.is_null() {
                    continue;
                }
                let backend_node_id = harvest::native_id(&child);
                let app_id = match child.as_accessible_proxy(self.conn.connection()).await {
                    Ok(p) => p.name().await.unwrap_or_default(),
                    Err(_) => String::new(),
                };
                self.app_handles
                    .lock()
                    .expect("mutex")
                    .insert(backend_node_id.clone(), child.clone());
                apps.push(AppHandle { title: app_id.clone(), app_id, backend_node_id });
            }
            Ok(apps)
        })
    }

    fn snapshot_app<'a>(&'a self, app: &'a AppHandle) -> BoxFuture<'a, Result<Vec<SlugNode>>> {
        Box::pin(async move {
            let objref = self.app_objref(&app.backend_node_id).await?;
            let harvested = harvest::harvest_apps(self.conn.connection(), &[objref]).await?;

            // Capture ref → handle for the action layer.
            {
                let mut handles = self.handles.lock().expect("mutex");
                handles.extend(harvested.index);
            }
            // Cache the per-app coverage for `coverage()`.
            if let Some(cov) = harvested.coverage.into_iter().next() {
                self.coverage.lock().expect("mutex").insert(app.backend_node_id.clone(), cov);
            }

            Ok(harvested.document.iter().cloned().collect())
        })
    }

    fn invoke<'a>(
        &'a self,
        node_id: &'a BackendNodeId,
        action: &'a Action,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let objref = self
                .handles
                .lock()
                .expect("mutex")
                .get(node_id.as_str())
                .cloned()
                .ok_or_else(|| BridgeError::UnknownRef(node_id.0.clone()))?;
            actions::perform(self.conn.connection(), &objref, action, None).await?;
            Ok(())
        })
    }

    fn subscribe_events(&self, sink: EventSink) -> BoxFuture<'_, Result<Subscription>> {
        Box::pin(async move {
            // Dedicated connection so harvesting and eventing don't contend.
            let conn = AccessibilityConnection::new().await?;
            let mut rx = events::subscribe(conn).await?;
            let task = tokio::spawn(async move {
                while let Some(ev) = rx.recv().await {
                    if !sink.emit(ev).await {
                        break;
                    }
                }
            });
            Ok(Subscription::new(task))
        })
    }

    fn coverage(&self, app: &AppHandle) -> Coverage {
        self.coverage
            .lock()
            .expect("mutex")
            .get(&app.backend_node_id)
            .cloned()
            .unwrap_or_else(|| {
                coverage::assess(&app.app_id, &slug_core::derive_ref(&app.backend_node_id), 0, 0)
            })
    }
}
