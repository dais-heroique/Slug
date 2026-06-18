//! Live event subscription: AT-SPI2 `org.a11y.atspi.Event.Object` signals →
//! Slug semantic events / deltas (§5; step-1 adaptation rule #2).
//!
//! We register for `StateChanged` and `ChildrenChanged` object events, consume
//! the connection's event stream on a background task, translate each signal into
//! one or more [`SlugEvent`]s, and forward them over an mpsc channel. The session
//! daemon (`slug-mcp`) applies these to its in-memory tree and re-broadcasts them
//! to the agent.

use atspi::events::object::{ChildrenChangedEvent, StateChangedEvent};
use atspi::proxy::accessible::ObjectRefExt;
use atspi::{AccessibilityConnection, Event, ObjectEvents, Operation};
use futures::StreamExt;
use slug_core::{SlugEvent, SlugNodePatch};
use tokio::sync::mpsc;
use tracing::{debug, trace, warn};

use crate::error::Result;
use super::harvest::{obj_ref, read_node};
use super::mapping::{map_state, map_states};

/// Subscribe to live AT-SPI object events and receive translated [`SlugEvent`]s.
///
/// Takes ownership of an [`AccessibilityConnection`] (the background task needs a
/// `'static` owner of the stream). Returns the receiving end of the event
/// channel; dropping it stops delivery, and the task exits when the stream ends.
pub async fn subscribe(conn: AccessibilityConnection) -> Result<mpsc::Receiver<SlugEvent>> {
    // Register the object events we care about so the registry forwards them.
    conn.register_event::<StateChangedEvent>().await?;
    conn.register_event::<ChildrenChangedEvent>().await?;

    let (tx, rx) = mpsc::channel::<SlugEvent>(256);

    tokio::spawn(async move {
        // Independent connection handle for re-querying object state inside the
        // loop without entangling borrows with the borrowed event stream.
        let zconn = conn.connection().clone();
        let mut stream = conn.event_stream();
        debug!("AT-SPI event stream started");

        while let Some(item) = stream.next().await {
            let event = match item {
                Ok(e) => e,
                Err(e) => {
                    trace!(error = %e, "dropped malformed AT-SPI event");
                    continue;
                }
            };

            let slug_events = translate(&zconn, event).await;
            for ev in slug_events {
                if tx.send(ev).await.is_err() {
                    debug!("event receiver dropped; stopping AT-SPI subscription");
                    return;
                }
            }
        }
        warn!("AT-SPI event stream ended");
    });

    Ok(rx)
}

/// Translate one AT-SPI event into zero or more [`SlugEvent`]s.
async fn translate(zconn: &zbus::Connection, event: Event) -> Vec<SlugEvent> {
    match event {
        Event::Object(ObjectEvents::StateChanged(e)) => translate_state_changed(zconn, e).await,
        Event::Object(ObjectEvents::ChildrenChanged(e)) => translate_children_changed(zconn, e).await,
        _ => Vec::new(),
    }
}

async fn translate_state_changed(zconn: &zbus::Connection, e: StateChangedEvent) -> Vec<SlugEvent> {
    let slug_ref = obj_ref(&e.item);
    let mut out = Vec::new();

    // Focus is surfaced as a first-class FocusChanged event (§5.3).
    if map_state(e.state) == Some(slug_core::SlugState::Focused) {
        out.push(SlugEvent::FocusChanged {
            slug_ref: if e.enabled { Some(slug_ref.clone()) } else { None },
        });
    }

    // Re-query the full state set so the patch carries the canonical vector
    // (§5.2 patches replace the whole `states` field).
    if let Ok(proxy) = e.item.as_accessible_proxy(zconn).await {
        if let Ok(set) = proxy.get_state().await {
            let mut patch = SlugNodePatch::new(&slug_ref);
            patch.states = Some(map_states(set));
            out.push(SlugEvent::NodeUpdated { patch });
        }
    }

    out
}

async fn translate_children_changed(
    zconn: &zbus::Connection,
    e: ChildrenChangedEvent,
) -> Vec<SlugEvent> {
    let parent_ref = obj_ref(&e.item);
    match e.operation {
        Operation::Insert => {
            if e.child.is_null() {
                return Vec::new();
            }
            match read_node(zconn, &e.child, Some(parent_ref)).await {
                Ok(node) => vec![SlugEvent::NodeCreated { node }],
                Err(err) => {
                    trace!(error = %err, "could not read newly-created node");
                    Vec::new()
                }
            }
        }
        Operation::Delete => {
            vec![SlugEvent::NodeDestroyed { slug_ref: obj_ref(&e.child) }]
        }
    }
}
