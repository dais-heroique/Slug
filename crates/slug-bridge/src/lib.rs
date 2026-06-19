//! # slug-bridge
//!
//! The cross-platform accessibility harvester. It connects to the OS
//! accessibility service, walks application trees into a [`slug_core::SlugDocument`],
//! executes actions, and streams live semantic events — exposing one async API to
//! `slug-mcp` regardless of platform.
//!
//! ## Backends (M1.5)
//!
//! All platform-specific perception/action lives behind the
//! [`AccessibilityBackend`] trait ([`backend`]); the [`Bridge`] facade selects one
//! per `cfg(target_os)`:
//!
//! | OS | Module | Source |
//! |----|--------|--------|
//! | Linux | [`backend_atspi`] | AT-SPI2 over D-Bus (the original harvester) |
//! | Windows | `backend_uia` | UI Automation (`IUIAutomation`) |
//! | macOS | `backend_ax` | the Accessibility (AX) API (`AXUIElement`) |
//!
//! The semantic model (`slug-core`), the MCP server (`slug-mcp`), and the agent
//! (`slug-brain`) are platform-agnostic and unchanged.
//!
//! ## Milestone-1 adaptations (carried forward)
//!
//! Live [`slug_core::SlugDelta`]/[`slug_core::SlugEvent`] frames are produced from
//! native accessibility signals (not Wayland frame commits). Node refs are derived
//! ULIDs hashed from each backend's native stable identity (brief §4); the agent
//! only ever sees short session aliases.

pub mod action;
pub mod backend;
pub mod coverage;
pub mod error;

#[cfg(target_os = "linux")]
pub mod backend_atspi;
#[cfg(target_os = "macos")]
pub mod backend_ax;
#[cfg(target_os = "macos")]
pub mod synth_macos;
#[cfg(target_os = "windows")]
pub mod backend_uia;

use std::collections::HashSet;
use std::sync::Mutex;

use slug_core::{derive_ref, SlugDocument, SlugEvent};
use serde::Serialize;
use tokio::sync::mpsc;
use tracing::{info, instrument};

pub use action::Action;
pub use backend::{AccessibilityBackend, AppHandle, BackendNodeId, EventSink, Subscription};
pub use coverage::{Coverage, OpaqueReason};
pub use error::{BridgeError, Result};

/// Summary of a running accessible application (returned to `slug-mcp`).
#[derive(Clone, Debug, Serialize)]
pub struct AppInfo {
    /// Display name of the application (e.g. `Text Editor`, `Notepad`, `Finder`).
    pub app_id: String,
    /// The application's stable Slug ref (derived ULID).
    pub app_ref: String,
    /// The backend's native stable identity for the app (brief §4).
    pub bus_name: String,
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

/// The platform-neutral bridge to the OS accessibility service.
///
/// Public API is identical across platforms and unchanged from M1, so `slug-mcp`
/// is platform-agnostic.
pub struct Bridge {
    backend: Box<dyn AccessibilityBackend>,
    /// Refs known from the most recent snapshots (for [`Bridge::knows_ref`]).
    known: Mutex<HashSet<String>>,
    /// Live subscriptions kept alive for the Bridge's lifetime.
    subs: Mutex<Vec<Subscription>>,
}

impl Bridge {
    /// Connect to the platform accessibility backend.
    pub async fn connect() -> Result<Self> {
        let backend = select_backend().await?;
        info!(backend = backend.label(), "accessibility backend ready");
        Ok(Bridge {
            backend,
            known: Mutex::new(HashSet::new()),
            subs: Mutex::new(Vec::new()),
        })
    }

    /// List the applications currently exposing an accessibility tree.
    pub async fn list_apps(&self) -> Result<Vec<AppInfo>> {
        let apps = self.backend.enumerate_apps().await?;
        Ok(apps
            .into_iter()
            .map(|a| AppInfo {
                app_ref: derive_ref(&a.backend_node_id),
                app_id: a.app_id,
                bus_name: a.backend_node_id,
            })
            .collect())
    }

    /// Harvest the entire desktop (all applications) into a fresh document.
    #[instrument(skip(self))]
    pub async fn snapshot_desktop(&self) -> Result<SnapshotResult> {
        let apps = self.backend.enumerate_apps().await?;
        self.snapshot_apps(&apps).await
    }

    /// Harvest a single application by name / native id / ref.
    #[instrument(skip(self))]
    pub async fn snapshot_app(&self, app_key: &str) -> Result<SnapshotResult> {
        let apps = self.backend.enumerate_apps().await?;
        let app = apps
            .into_iter()
            .find(|a| {
                a.backend_node_id == app_key
                    || a.app_id == app_key
                    || derive_ref(&a.backend_node_id) == app_key
            })
            .ok_or_else(|| BridgeError::UnknownRef(app_key.to_string()))?;
        self.snapshot_apps(&[app]).await
    }

    /// Snapshot a set of apps into one document, recording known refs + coverage.
    async fn snapshot_apps(&self, apps: &[AppHandle]) -> Result<SnapshotResult> {
        let mut doc = SlugDocument::new();
        let mut coverage = Vec::new();

        for app in apps {
            let nodes = match self.backend.snapshot_app(app).await {
                Ok(n) => n,
                Err(e) => {
                    tracing::warn!(app = %app.app_id, error = %e, "failed to snapshot app; skipping");
                    continue;
                }
            };
            {
                let mut known = self.known.lock().expect("known mutex poisoned");
                for n in &nodes {
                    known.insert(n.slug_ref.clone());
                }
            }
            for n in nodes {
                doc.insert(n);
            }
            coverage.push(self.backend.coverage(app));
        }

        doc.recompute_roots();
        let opaque: Vec<Coverage> = coverage.iter().filter(|c| c.is_opaque()).cloned().collect();
        Ok(SnapshotResult { document: doc, coverage, opaque })
    }

    /// Execute an action on a node identified by its (internal) Slug ref.
    ///
    /// `verb` is an action verb (`click`, `set_text`, `set_value`, `focus`, …);
    /// `arg` is its optional argument; `reasoning` is the agent's rationale, logged
    /// with the action (structured-logging requirement).
    #[instrument(skip(self), fields(reasoning = reasoning.unwrap_or("")))]
    pub async fn invoke(
        &self,
        slug_ref: &str,
        verb: &str,
        arg: Option<&str>,
        reasoning: Option<&str>,
    ) -> Result<bool> {
        let action = Action::parse(verb, arg)?;
        info!(%slug_ref, action = %action.id(), "invoke");
        self.backend.invoke(&BackendNodeId(slug_ref.to_string()), &action).await?;
        Ok(true)
    }

    /// Inject synthetic OS input (key chord or literal text) into the focused
    /// application. Works on any app, including opaque ones — no node ref, no
    /// pixels. `verb` must be a synthetic verb (`key`/`hotkey`/`type_text`).
    #[instrument(skip(self), fields(reasoning = reasoning.unwrap_or("")))]
    pub async fn synth_input(
        &self,
        verb: &str,
        arg: Option<&str>,
        reasoning: Option<&str>,
    ) -> Result<bool> {
        let action = Action::parse(verb, arg)?;
        if !action.is_synthetic() {
            return Err(BridgeError::InvalidArgs {
                action: action.id(),
                detail: "not a synthetic-input verb (use key/hotkey/type_text)".into(),
            });
        }
        info!(action = %action.id(), "synth_input");
        self.backend.synth_input(&action).await?;
        Ok(true)
    }

    /// Subscribe to live semantic events. The subscription is kept alive for the
    /// lifetime of the Bridge; dropping the returned receiver stops delivery.
    pub async fn subscribe(&self) -> Result<mpsc::Receiver<SlugEvent>> {
        let (tx, rx) = mpsc::channel::<SlugEvent>(256);
        let sub = self.backend.subscribe_events(EventSink::new(tx)).await?;
        self.subs.lock().expect("subs mutex poisoned").push(sub);
        Ok(rx)
    }

    /// Whether a ref is currently known from a recent snapshot.
    pub fn knows_ref(&self, slug_ref: &str) -> bool {
        self.known.lock().expect("known mutex poisoned").contains(slug_ref)
    }

    /// The active backend's short label (`atspi` / `uia` / `ax`).
    pub fn backend_label(&self) -> &'static str {
        self.backend.label()
    }
}

/// Construct the accessibility backend for the current platform.
async fn select_backend() -> Result<Box<dyn AccessibilityBackend>> {
    #[cfg(target_os = "linux")]
    {
        Ok(Box::new(backend_atspi::AtspiBackend::connect().await?))
    }
    #[cfg(target_os = "windows")]
    {
        Ok(Box::new(backend_uia::UiaBackend::new()?))
    }
    #[cfg(target_os = "macos")]
    {
        Ok(Box::new(backend_ax::AxBackend::new()?))
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        Err(BridgeError::Unsupported(std::env::consts::OS.to_string()))
    }
}
