//! `SlugState` — the canonical state enumeration (SEMANTIC-SCHEMA §3.2).
//!
//! JSON wire form is `SCREAMING_SNAKE_CASE` (e.g. `FOCUSED`, `HAS_POPUP`,
//! `MULTI_LINE`). The AT-SPI2 `State` → `SlugState` mapping lives in `slug-bridge`.

use serde::{Deserialize, Serialize};

/// Canonical Slug state. Faithful mirror of the §3.2 table, including the two
/// Slug extensions (`READS_CONTENT` is Slug-only; `INVALID_ENTRY` mirrors the
/// richer `validation` field).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SlugState {
    Active,
    Animated,
    Busy,
    Checked,
    Collapsed,
    Default,
    Defunct,
    Editable,
    Enabled,
    Expanded,
    Focusable,
    Focused,
    HasPopup,
    Horizontal,
    Iconified,
    Indeterminate,
    InvalidEntry,
    ManagesDescendants,
    Modal,
    MultiLine,
    MultiSelectable,
    Opaque,
    Pressed,
    ReadsContent,
    Required,
    Resizable,
    Selectable,
    Selected,
    Sensitive,
    Showing,
    SingleLine,
    Stale,
    SupportsAutocompletion,
    Transient,
    Truncated,
    Vertical,
    Visible,
    Visited,
}

impl SlugState {
    /// Canonical `SCREAMING_SNAKE_CASE` name (matches the JSON wire form).
    pub fn as_str(&self) -> &'static str {
        use SlugState::*;
        match self {
            Active => "ACTIVE",
            Animated => "ANIMATED",
            Busy => "BUSY",
            Checked => "CHECKED",
            Collapsed => "COLLAPSED",
            Default => "DEFAULT",
            Defunct => "DEFUNCT",
            Editable => "EDITABLE",
            Enabled => "ENABLED",
            Expanded => "EXPANDED",
            Focusable => "FOCUSABLE",
            Focused => "FOCUSED",
            HasPopup => "HAS_POPUP",
            Horizontal => "HORIZONTAL",
            Iconified => "ICONIFIED",
            Indeterminate => "INDETERMINATE",
            InvalidEntry => "INVALID_ENTRY",
            ManagesDescendants => "MANAGES_DESCENDANTS",
            Modal => "MODAL",
            MultiLine => "MULTI_LINE",
            MultiSelectable => "MULTI_SELECTABLE",
            Opaque => "OPAQUE",
            Pressed => "PRESSED",
            ReadsContent => "READS_CONTENT",
            Required => "REQUIRED",
            Resizable => "RESIZABLE",
            Selectable => "SELECTABLE",
            Selected => "SELECTED",
            Sensitive => "SENSITIVE",
            Showing => "SHOWING",
            SingleLine => "SINGLE_LINE",
            Stale => "STALE",
            SupportsAutocompletion => "SUPPORTS_AUTOCOMPLETION",
            Transient => "TRANSIENT",
            Truncated => "TRUNCATED",
            Vertical => "VERTICAL",
            Visible => "VISIBLE",
            Visited => "VISITED",
        }
    }

    /// Whether this state is worth surfacing in the compact YAML snapshot.
    ///
    /// The full state set is noisy for an agent (every node carries SHOWING,
    /// VISIBLE, SENSITIVE, ENABLED, FOCUSABLE…). The snapshot therefore shows only
    /// states that change how the agent should reason about a control, in the
    /// spirit of Playwright-MCP's curated attribute display.
    pub fn is_salient(&self) -> bool {
        use SlugState::*;
        matches!(
            self,
            Checked
                | Pressed
                | Expanded
                | Collapsed
                | Selected
                | Focused
                | Indeterminate
                | InvalidEntry
                | Required
                | Busy
                | Modal
                | Stale
                | Truncated
                | ManagesDescendants
                | Visited
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_form_is_screaming_snake() {
        assert_eq!(serde_json::to_string(&SlugState::HasPopup).unwrap(), "\"HAS_POPUP\"");
        assert_eq!(serde_json::to_string(&SlugState::MultiLine).unwrap(), "\"MULTI_LINE\"");
        assert_eq!(
            serde_json::to_string(&SlugState::SupportsAutocompletion).unwrap(),
            "\"SUPPORTS_AUTOCOMPLETION\""
        );
    }

    #[test]
    fn round_trips() {
        let s: SlugState = serde_json::from_str("\"FOCUSED\"").unwrap();
        assert_eq!(s, SlugState::Focused);
    }
}
