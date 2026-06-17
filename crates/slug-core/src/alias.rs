//! Session ref aliases — step-1 adaptation rule #1.
//!
//! Internally every node is identified by its ULID-shaped `ref` (§4). The agent,
//! however, never sees ULIDs: it works with short, session-scoped aliases like
//! `b1` (first button) or `e5` (fifth generic element). The YAML snapshot and all
//! MCP tools speak aliases exclusively; `slug-mcp` / `slug-bridge` own an
//! [`AliasTable`] and translate at the boundary.
//!
//! Aliases are 1:1 with ULIDs within a session and are assigned lazily and
//! idempotently as nodes are first surfaced to the agent.

use std::collections::HashMap;

use crate::SlugRole;

/// A bidirectional, session-scoped map between ULID refs and short aliases.
#[derive(Debug, Default, Clone)]
pub struct AliasTable {
    /// ULID ref → alias (e.g. `01J..` → `b3`).
    to_alias: HashMap<String, String>,
    /// alias → ULID ref.
    to_ref: HashMap<String, String>,
    /// Per-prefix counter (e.g. `'b'` → 3 means `b1`,`b2`,`b3` used).
    counters: HashMap<char, u32>,
}

impl AliasTable {
    /// Create an empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Forget all assignments (e.g. on a fresh desktop snapshot). After this,
    /// previously issued aliases are invalid.
    pub fn clear(&mut self) {
        self.to_alias.clear();
        self.to_ref.clear();
        self.counters.clear();
    }

    /// Return the alias for a ref if one has already been assigned.
    pub fn alias_for(&self, slug_ref: &str) -> Option<&str> {
        self.to_alias.get(slug_ref).map(String::as_str)
    }

    /// Resolve an agent-facing alias back to its internal ULID ref.
    pub fn ref_for(&self, alias: &str) -> Option<&str> {
        self.to_ref.get(alias).map(String::as_str)
    }

    /// Assign (or return the existing) alias for `slug_ref`. Idempotent: calling
    /// it repeatedly for the same ref always yields the same alias. The prefix is
    /// derived from `role` (see [`SlugRole::alias_prefix`]).
    pub fn assign(&mut self, slug_ref: &str, role: SlugRole) -> String {
        if let Some(existing) = self.to_alias.get(slug_ref) {
            return existing.clone();
        }
        let prefix = role.alias_prefix();
        let counter = self.counters.entry(prefix).or_insert(0);
        *counter += 1;
        let alias = format!("{prefix}{counter}");
        self.to_alias.insert(slug_ref.to_string(), alias.clone());
        self.to_ref.insert(alias.clone(), slug_ref.to_string());
        alias
    }

    /// Number of aliases currently assigned.
    pub fn len(&self) -> usize {
        self.to_alias.len()
    }

    /// Whether no aliases are assigned.
    pub fn is_empty(&self) -> bool {
        self.to_alias.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assignment_is_idempotent_and_one_to_one() {
        let mut t = AliasTable::new();
        let a1 = t.assign("ULID-A", SlugRole::Button);
        let a1_again = t.assign("ULID-A", SlugRole::Button);
        assert_eq!(a1, a1_again);
        assert_eq!(a1, "b1");

        let a2 = t.assign("ULID-B", SlugRole::Button);
        assert_eq!(a2, "b2");

        assert_eq!(t.ref_for("b1"), Some("ULID-A"));
        assert_eq!(t.ref_for("b2"), Some("ULID-B"));
        assert_eq!(t.alias_for("ULID-A"), Some("b1"));
    }

    #[test]
    fn prefix_follows_role_class() {
        let mut t = AliasTable::new();
        assert_eq!(t.assign("r1", SlugRole::Link), "l1");
        assert_eq!(t.assign("r2", SlugRole::Entry), "i1");
        assert_eq!(t.assign("r3", SlugRole::StaticText), "e1");
        assert_eq!(t.assign("r4", SlugRole::Button), "b1");
    }
}
