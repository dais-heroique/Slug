//! Delta / event model — SEMANTIC-SCHEMA §5.
//!
//! At milestone 1 there is no Wayland frame loop: `SlugDelta` frames are produced
//! by `slug-bridge` from AT-SPI2 signals (`StateChanged`, `ChildrenChanged`,
//! focus changes) per the step-1 adaptation in the task brief. The wire format
//! below is exactly the §5.2 format regardless of how the delta was produced.

use serde::{Deserialize, Serialize};

use crate::{Bounds, SlugNode, SlugState, Validation};

/// A patch describing only the changed fields of an existing node (§5.2).
///
/// `Option<Option<T>>` is used for nullable fields so the three states are
/// distinguishable on the wire:
/// * outer `None`  → field absent (no change),
/// * `Some(None)`  → field changed to `null`,
/// * `Some(Some)`  → field changed to a value.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SlugNodePatch {
    #[serde(rename = "ref")]
    pub slug_ref: String,

    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub states: Option<Vec<SlugState>>,

    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub value: Option<Option<String>>,

    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub name: Option<Option<String>>,

    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub bounds: Option<Option<Bounds>>,

    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub validation: Option<Option<Validation>>,
}

impl SlugNodePatch {
    pub fn new(slug_ref: impl Into<String>) -> Self {
        SlugNodePatch { slug_ref: slug_ref.into(), ..Default::default() }
    }

    /// Whether this patch carries any change beyond the ref.
    pub fn is_empty(&self) -> bool {
        self.states.is_none()
            && self.value.is_none()
            && self.name.is_none()
            && self.bounds.is_none()
            && self.validation.is_none()
    }
}

/// A re-ordering of one parent's children (§5.2).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SlugReorder {
    pub parent_ref: String,
    /// Complete new ordered list of child refs.
    pub child_refs: Vec<String>,
}

/// A delta frame (§5.2). `frame_id` is monotonically increasing per surface.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SlugDelta {
    pub frame_id: u64,
    pub surface_id: String,
    /// Microseconds, `CLOCK_MONOTONIC`.
    pub timestamp_us: u64,

    #[serde(default)]
    pub created: Vec<SlugNode>,
    #[serde(default)]
    pub updated: Vec<SlugNodePatch>,
    #[serde(default)]
    pub destroyed: Vec<String>,
    #[serde(default)]
    pub reordered: Vec<SlugReorder>,
    /// Ref of newly focused node, or `null`.
    #[serde(default)]
    pub focus_changed: Option<String>,
}

impl SlugDelta {
    /// Whether the delta carries no observable change.
    pub fn is_empty(&self) -> bool {
        self.created.is_empty()
            && self.updated.is_empty()
            && self.destroyed.is_empty()
            && self.reordered.is_empty()
            && self.focus_changed.is_none()
    }
}

/// High-level semantic events (§5.3 D-Bus signals), modelled as a tagged enum.
///
/// `slug-bridge` derives these from AT-SPI2 signals; `slug-mcp` exposes them to
/// the agent through `slug_wait_for`. The tag matches the §5.3 signal name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum SlugEvent {
    NodeCreated { node: SlugNode },
    NodeDestroyed { #[serde(rename = "ref")] slug_ref: String },
    NodeUpdated { patch: SlugNodePatch },
    FocusChanged { #[serde(rename = "ref")] slug_ref: Option<String> },
    ActionCompleted { action_id: String, #[serde(rename = "ref")] slug_ref: String, success: bool },
    WindowOpened { window_id: String, app_id: String },
    WindowClosed { window_id: String },
    NotificationPosted { node: SlugNode },
    SemanticSuspended { reason: String },
}

impl SlugEvent {
    /// The §5.3 signal name (used as the `event_type` filter in `slug_wait_for`).
    pub fn type_name(&self) -> &'static str {
        match self {
            SlugEvent::NodeCreated { .. } => "node_created",
            SlugEvent::NodeDestroyed { .. } => "node_destroyed",
            SlugEvent::NodeUpdated { .. } => "node_updated",
            SlugEvent::FocusChanged { .. } => "focus_changed",
            SlugEvent::ActionCompleted { .. } => "action_completed",
            SlugEvent::WindowOpened { .. } => "window_opened",
            SlugEvent::WindowClosed { .. } => "window_closed",
            SlugEvent::NotificationPosted { .. } => "notification_posted",
            SlugEvent::SemanticSuspended { .. } => "semantic_suspended",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_nullable_tri_state() {
        // Absent value → key omitted.
        let p = SlugNodePatch::new("r1");
        let j = serde_json::to_value(&p).unwrap();
        assert!(j.get("value").is_none());

        // value = Some(None) → JSON null.
        let mut p2 = SlugNodePatch::new("r1");
        p2.value = Some(None);
        let j2 = serde_json::to_value(&p2).unwrap();
        assert!(j2["value"].is_null());

        // value = Some(Some(..)) → JSON string.
        let mut p3 = SlugNodePatch::new("r1");
        p3.value = Some(Some("hi".into()));
        let j3 = serde_json::to_value(&p3).unwrap();
        assert_eq!(j3["value"], "hi");
    }

    #[test]
    fn event_tag_matches_signal_name() {
        let e = SlugEvent::FocusChanged { slug_ref: Some("r5".into()) };
        let j = serde_json::to_value(&e).unwrap();
        assert_eq!(j["event"], "focus_changed");
        assert_eq!(j["ref"], "r5");
    }
}
