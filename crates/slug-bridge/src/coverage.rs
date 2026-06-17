//! Coverage heuristic: flag applications whose accessibility tree is empty or
//! suspiciously flat as "opaque" — candidates for the vision/screenshot fallback
//! (design axiom A1; §3.1 CANVAS/IMAGE/MEDIA notes).
//!
//! An app that exposes only a window with no (or very few) descendants, or whose
//! tree has no interactive nodes, almost certainly renders its UI on a canvas
//! (Electron without a11y, games, some Java/Qt apps) and cannot be driven through
//! the semantic layer alone.

use serde::Serialize;

/// Why an app was judged opaque.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OpaqueReason {
    /// The app exposed no accessible nodes at all.
    Empty,
    /// The tree is too shallow/small to represent a real UI.
    Flat,
}

/// A per-application coverage report.
#[derive(Clone, Debug, Serialize)]
pub struct Coverage {
    pub app_id: String,
    pub app_ref: String,
    pub node_count: usize,
    pub max_depth: usize,
    /// `Some(reason)` if this app should fall back to vision; `None` if the tree
    /// looks rich enough to drive semantically.
    pub opaque: Option<OpaqueReason>,
}

impl Coverage {
    pub fn is_opaque(&self) -> bool {
        self.opaque.is_some()
    }
}

/// Minimum descendants/depth for a tree to be considered non-opaque.
const MIN_NODES: usize = 3;
const MIN_DEPTH: usize = 2;

/// Assess an application's harvested tree.
pub fn assess(app_id: &str, app_ref: &str, node_count: usize, max_depth: usize) -> Coverage {
    let opaque = if node_count == 0 {
        Some(OpaqueReason::Empty)
    } else if node_count < MIN_NODES || max_depth < MIN_DEPTH {
        Some(OpaqueReason::Flat)
    } else {
        None
    };
    Coverage { app_id: app_id.to_string(), app_ref: app_ref.to_string(), node_count, max_depth, opaque }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_tree_is_opaque() {
        let c = assess("app", "r", 0, 0);
        assert_eq!(c.opaque, Some(OpaqueReason::Empty));
    }

    #[test]
    fn flat_tree_is_opaque() {
        let c = assess("app", "r", 1, 0);
        assert_eq!(c.opaque, Some(OpaqueReason::Flat));
    }

    #[test]
    fn rich_tree_is_not_opaque() {
        let c = assess("app", "r", 200, 8);
        assert!(!c.is_opaque());
    }
}
