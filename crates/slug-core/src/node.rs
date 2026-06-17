//! `SlugNode` and its sub-structures — SEMANTIC-SCHEMA §2.
//!
//! This is a faithful Rust mirror of the TypeScript `SlugNode` interface. Field
//! names match the wire form (with `ref` mapped to the reserved-word-safe
//! `slug_ref`). Optional-and-nullable fields use `Option<T>` with
//! `skip_serializing_if` so that "omit ≡ unknown/not applicable" (§2) holds on
//! the wire.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{SlugRole, SlugState};

/// Axis-aligned bounds in compositor-space, unscaled pixels (§2). At milestone 1
/// these come from the AT-SPI2 `Component.GetExtents` call in screen coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// A selectable option for combo boxes, list boxes and radio groups (§2).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SlugOption {
    pub value: String,
    pub label: String,
    pub selected: bool,
}

/// An interaction affordance the agent can perform on a node (§2).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SlugAction {
    /// Stable action id, e.g. `activate`, `expand`, `set_value`.
    pub id: String,
    /// Human/agent-readable description.
    pub label: String,
}

/// Validation state, richer than AT-SPI2's single `INVALID_ENTRY` flag (§2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ValidationState {
    Valid,
    Invalid,
    Pending,
}

/// The `validation` object (§2).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Validation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<ValidationState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// A single semantic node. Faithful mirror of the §2 `SlugNode` interface.
///
/// Only `slug_ref` (`ref` on the wire) and `role` are mandatory; everything else
/// is omitted when unknown. `app_id`, `window_id` and `surface_id` are typed as
/// required `string` in the schema and therefore always serialized (empty string
/// when not yet known at milestone 1).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SlugNode {
    // --- Identity ---
    /// 128-bit ULID-shaped ref (§4). Derived from `{bus_name}:{path}` at M1.
    #[serde(rename = "ref")]
    pub slug_ref: String,
    /// Parent ref; `None` only for the root node.
    #[serde(default)]
    pub parent_ref: Option<String>,
    /// Ordered child refs.
    #[serde(default)]
    pub child_refs: Vec<String>,

    // --- Classification ---
    pub role: SlugRole,
    #[serde(default)]
    pub states: Vec<SlugState>,

    // --- Human-readable labels ---
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub placeholder: Option<String>,

    // --- Geometry ---
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub bounds: Option<Bounds>,

    // --- Value semantics ---
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub value_min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub value_max: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub value_step: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub options: Option<Vec<SlugOption>>,

    // --- Text content ---
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub text_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub heading_level: Option<u8>,

    // --- Interaction affordances ---
    #[serde(default)]
    pub actions: Vec<SlugAction>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub keyboard_shortcut: Option<String>,

    // --- Application context ---
    #[serde(default)]
    pub app_id: String,
    #[serde(default)]
    pub window_id: String,
    #[serde(default)]
    pub surface_id: String,

    // --- Validation ---
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub validation: Option<Validation>,

    // --- Extension bag ---
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub extensions: Option<BTreeMap<String, String>>,
}

impl SlugNode {
    /// Construct a minimal node with only the two mandatory fields populated.
    pub fn new(slug_ref: impl Into<String>, role: SlugRole) -> Self {
        SlugNode {
            slug_ref: slug_ref.into(),
            parent_ref: None,
            child_refs: Vec::new(),
            role,
            states: Vec::new(),
            name: None,
            description: None,
            placeholder: None,
            bounds: None,
            value: None,
            value_min: None,
            value_max: None,
            value_step: None,
            options: None,
            text_content: None,
            heading_level: None,
            actions: Vec::new(),
            keyboard_shortcut: None,
            app_id: String::new(),
            window_id: String::new(),
            surface_id: String::new(),
            validation: None,
            extensions: None,
        }
    }

    /// Whether the node carries a given state.
    pub fn has_state(&self, state: SlugState) -> bool {
        self.states.contains(&state)
    }

    /// The best human-readable label for display: `name`, falling back to
    /// `value`, then `text_content`.
    pub fn display_label(&self) -> Option<&str> {
        self.name
            .as_deref()
            .filter(|s| !s.is_empty())
            .or(self.value.as_deref().filter(|s| !s.is_empty()))
            .or(self.text_content.as_deref().filter(|s| !s.is_empty()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ref_field_serializes_as_ref() {
        let n = SlugNode::new("01ABC", SlugRole::Button);
        let json = serde_json::to_value(&n).unwrap();
        assert_eq!(json["ref"], "01ABC");
        assert_eq!(json["role"], "BUTTON");
        // Unknown optional fields are omitted.
        assert!(json.get("name").is_none());
        assert!(json.get("bounds").is_none());
        // Required-string context fields are always present.
        assert_eq!(json["app_id"], "");
    }

    #[test]
    fn round_trips_full_node() {
        let mut n = SlugNode::new("01ABC", SlugRole::Slider);
        n.name = Some("Volume".into());
        n.value = Some("50".into());
        n.value_min = Some(0.0);
        n.value_max = Some(100.0);
        n.states = vec![SlugState::Focusable, SlugState::Enabled];
        n.bounds = Some(Bounds { x: 1.0, y: 2.0, width: 3.0, height: 4.0 });
        let json = serde_json::to_string(&n).unwrap();
        let back: SlugNode = serde_json::from_str(&json).unwrap();
        assert_eq!(n, back);
    }
}
