//! `SlugDocument` — an in-memory arena of [`SlugNode`]s keyed by ULID ref.
//!
//! The session daemon (here, `slug-mcp`) materialises the current tree as a
//! `SlugDocument`, applies [`crate::SlugDelta`] frames to it, and renders agent
//! snapshots from it. Identity is the ULID `ref` (§4); there is no separate
//! integer id — the schema's `ref` *is* the arena key.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{AliasTable, CapabilityToken, SlugDelta, SlugNode, SlugNodePatch};

/// The materialised semantic tree for one or more surfaces.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SlugDocument {
    /// ULID ref → node.
    nodes: HashMap<String, SlugNode>,
    /// Root refs (nodes whose `parent_ref` is `None`), in insertion order.
    roots: Vec<String>,
}

impl SlugDocument {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a document from a flat node list (e.g. a §5.4 snapshot array). Nodes
    /// with no `parent_ref`, or whose parent is absent from the set, become roots.
    pub fn from_nodes(nodes: impl IntoIterator<Item = SlugNode>) -> Self {
        let mut doc = SlugDocument::new();
        for node in nodes {
            doc.insert(node);
        }
        doc.recompute_roots();
        doc
    }

    /// Insert or replace a node. Does not touch the root set; call
    /// [`Self::recompute_roots`] after a batch of inserts, or use
    /// [`Self::from_nodes`].
    pub fn insert(&mut self, node: SlugNode) {
        self.nodes.insert(node.slug_ref.clone(), node);
    }

    /// Look up a node by ref.
    pub fn get(&self, slug_ref: &str) -> Option<&SlugNode> {
        self.nodes.get(slug_ref)
    }

    /// Mutable lookup.
    pub fn get_mut(&mut self, slug_ref: &str) -> Option<&mut SlugNode> {
        self.nodes.get_mut(slug_ref)
    }

    /// Number of nodes.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Root refs in order.
    pub fn roots(&self) -> &[String] {
        &self.roots
    }

    /// Iterate over all nodes (unordered).
    pub fn iter(&self) -> impl Iterator<Item = &SlugNode> {
        self.nodes.values()
    }

    /// Recompute the root set: any node whose parent is `None` or missing.
    pub fn recompute_roots(&mut self) {
        let mut roots: Vec<String> = Vec::new();
        // Deterministic order: sort root refs lexically so snapshots are stable.
        for node in self.nodes.values() {
            let is_root = match &node.parent_ref {
                None => true,
                Some(p) => !self.nodes.contains_key(p),
            };
            if is_root {
                roots.push(node.slug_ref.clone());
            }
        }
        roots.sort();
        self.roots = roots;
    }

    /// Flatten the tree into breadth-first order starting from the roots — the
    /// node ordering used by the §5.4 snapshot array.
    pub fn bfs_order(&self) -> Vec<&SlugNode> {
        let mut out = Vec::with_capacity(self.nodes.len());
        let mut queue: std::collections::VecDeque<&str> =
            self.roots.iter().map(String::as_str).collect();
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        while let Some(r) = queue.pop_front() {
            if !seen.insert(r) {
                continue;
            }
            if let Some(node) = self.nodes.get(r) {
                out.push(node);
                for child in &node.child_refs {
                    if !seen.contains(child.as_str()) {
                        queue.push_back(child);
                    }
                }
            }
        }
        out
    }

    /// Produce a §5.4-shaped snapshot: the full tree as a flat BFS array, gated
    /// behind a capability check (stubbed at M1, see [`CapabilityToken`]).
    pub fn snapshot(&self, token: &CapabilityToken) -> Result<Snapshot, crate::CapabilityError> {
        token.check()?;
        Ok(Snapshot { nodes: self.bfs_order().into_iter().cloned().collect() })
    }

    /// Apply a [`SlugDelta`] to the in-memory tree (§5.1). Returns once the tree
    /// reflects the delta. Roots are recomputed if structure changed.
    pub fn apply_delta(&mut self, delta: &SlugDelta) {
        let mut structural = false;

        for node in &delta.created {
            self.insert(node.clone());
            structural = true;
        }
        for patch in &delta.updated {
            self.apply_patch(patch);
        }
        for r in &delta.destroyed {
            self.nodes.remove(r);
            structural = true;
        }
        for reorder in &delta.reordered {
            if let Some(parent) = self.nodes.get_mut(&reorder.parent_ref) {
                parent.child_refs = reorder.child_refs.clone();
            }
            structural = true;
        }

        if structural {
            self.recompute_roots();
        }
    }

    /// Apply a single field patch to an existing node.
    fn apply_patch(&mut self, patch: &SlugNodePatch) {
        let Some(node) = self.nodes.get_mut(&patch.slug_ref) else {
            return;
        };
        if let Some(states) = &patch.states {
            node.states = states.clone();
        }
        if let Some(value) = &patch.value {
            node.value = value.clone();
        }
        if let Some(name) = &patch.name {
            node.name = name.clone();
        }
        if let Some(bounds) = &patch.bounds {
            node.bounds = *bounds;
        }
        if let Some(validation) = &patch.validation {
            node.validation = validation.clone();
        }
    }

    /// Render a Playwright-MCP-style YAML snapshot using short session aliases.
    /// See [`crate::yaml`].
    pub fn to_yaml(&self, aliases: &AliasTable) -> String {
        crate::yaml::render(self, aliases)
    }

    /// Render YAML, assigning any missing aliases on the fly. Convenience for
    /// callers that own the alias table mutably (e.g. `slug-mcp` building a fresh
    /// snapshot).
    pub fn to_yaml_assigning(&self, aliases: &mut AliasTable) -> String {
        for node in self.bfs_order() {
            aliases.assign(&node.slug_ref, node.role);
        }
        crate::yaml::render(self, aliases)
    }
}

/// A §5.4 initial snapshot: the complete tree as a flat BFS array.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub nodes: Vec<SlugNode>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SlugRole;

    fn child(r: &str, parent: &str, role: SlugRole) -> SlugNode {
        let mut n = SlugNode::new(r, role);
        n.parent_ref = Some(parent.to_string());
        n
    }

    #[test]
    fn bfs_and_roots() {
        let mut root = SlugNode::new("root", SlugRole::Window);
        root.child_refs = vec!["a".into(), "b".into()];
        let a = child("a", "root", SlugRole::Button);
        let b = child("b", "root", SlugRole::Label);
        let doc = SlugDocument::from_nodes([root, a, b]);
        assert_eq!(doc.roots(), &["root".to_string()]);
        let order: Vec<&str> = doc.bfs_order().iter().map(|n| n.slug_ref.as_str()).collect();
        assert_eq!(order, ["root", "a", "b"]);
    }

    #[test]
    fn apply_delta_updates_and_destroys() {
        let mut root = SlugNode::new("root", SlugRole::Window);
        root.child_refs = vec!["a".into()];
        let a = child("a", "root", SlugRole::Button);
        let mut doc = SlugDocument::from_nodes([root, a]);

        let mut patch = SlugNodePatch::new("a");
        patch.name = Some(Some("Save".into()));
        let delta = SlugDelta {
            updated: vec![patch],
            destroyed: vec![],
            focus_changed: Some("a".into()),
            ..Default::default()
        };
        doc.apply_delta(&delta);
        assert_eq!(doc.get("a").unwrap().name.as_deref(), Some("Save"));

        let del = SlugDelta { destroyed: vec!["a".into()], ..Default::default() };
        doc.apply_delta(&del);
        assert!(doc.get("a").is_none());
    }

    #[test]
    fn snapshot_passes_stub_capability() {
        let doc = SlugDocument::from_nodes([SlugNode::new("root", SlugRole::Window)]);
        let snap = doc.snapshot(&CapabilityToken::anonymous()).unwrap();
        assert_eq!(snap.nodes.len(), 1);
    }
}
