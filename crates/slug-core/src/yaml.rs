//! Playwright-MCP-style YAML snapshot renderer.
//!
//! Output is an indented tree, one node per line:
//!
//! ```text
//! - window "Text Editor" [ref=e1]
//!   - button "Open" [ref=b1]
//!   - entry "filename" [ref=i1] [focused]
//!     - text "untitled" [ref=e2]
//! ```
//!
//! Every line is `- <role> [\"<label>\"] [ref=<alias>] [<state>]...`, where the
//! ref is the **short session alias** (never the ULID — step-1 rule #1). Only
//! salient states are shown (see [`crate::SlugState::is_salient`]); `disabled` is
//! synthesised for interactive nodes lacking `ENABLED`.

use crate::{AliasTable, SlugDocument, SlugNode, SlugRole, SlugState};

/// Render the whole document rooted at its root nodes.
pub fn render(doc: &SlugDocument, aliases: &AliasTable) -> String {
    let mut out = String::new();
    for root in doc.roots() {
        render_node(doc, root, aliases, 0, &mut out);
    }
    if out.is_empty() {
        out.push_str("# (empty document)\n");
    }
    out
}

fn render_node(
    doc: &SlugDocument,
    slug_ref: &str,
    aliases: &AliasTable,
    depth: usize,
    out: &mut String,
) {
    let Some(node) = doc.get(slug_ref) else {
        return;
    };

    for _ in 0..depth {
        out.push_str("  ");
    }
    out.push_str("- ");
    out.push_str(&node.role.yaml_name());

    if let Some(label) = node.display_label() {
        out.push_str(" \"");
        out.push_str(&escape(label));
        out.push('"');
    }

    // Ref alias. If the table has no alias yet (caller didn't pre-assign), fall
    // back to a parenthesised marker rather than leaking the ULID.
    match aliases.alias_for(slug_ref) {
        Some(alias) => {
            out.push_str(" [ref=");
            out.push_str(alias);
            out.push(']');
        }
        None => out.push_str(" [ref=?]"),
    }

    for state in salient_states(node) {
        out.push_str(" [");
        out.push_str(&state.to_ascii_lowercase());
        out.push(']');
    }

    // For opaque surfaces (canvas/graphics: Generic/Image) that have geometry but
    // typically no clickable child, expose the centre coordinate so the agent can
    // `slug_click` into them. Kept off normal controls to avoid token bloat — those
    // are clicked by ref.
    if matches!(node.role, SlugRole::Generic | SlugRole::Image) {
        if let Some(b) = node.bounds {
            let (cx, cy) = (b.x + b.width / 2.0, b.y + b.height / 2.0);
            out.push_str(&format!(" @{},{}", cx.round() as i64, cy.round() as i64));
        }
    }

    out.push('\n');

    for child in &node.child_refs {
        render_node(doc, child, aliases, depth + 1, out);
    }
}

/// Render a compact, **flat** list of only the nodes matching a filter — a
/// server-side "grep" over the tree. This is the fast path: instead of shipping
/// an 80k-char indented tree and letting the caller grep it, we walk the document
/// once and emit one line per matching node:
///
/// ```text
/// - button "Add to Basket" [ref=b7] @812,540
/// - entry "Search" [ref=i1] [focused] @640,80
/// ```
///
/// Each line carries the `ref` (for `slug_invoke`) and, when geometry is known,
/// the centre `@x,y` (for a `slug_click` fallback) — so the result is everything
/// the agent needs to act, and nothing else.
///
/// * `query` — case-insensitive substring matched against the node's display
///   label (name → value → text). `None` ⇒ no text constraint.
/// * `roles` — lower-case [`SlugRole::yaml_name`]s to keep (e.g. `["button"]`).
///   Empty ⇒ all roles.
/// * `interactive_only` — keep only directly actionable controls.
/// * `limit` — cap on emitted nodes; a trailing note reports any overflow.
pub fn render_filtered(
    doc: &SlugDocument,
    aliases: &AliasTable,
    query: Option<&str>,
    roles: &[String],
    interactive_only: bool,
    limit: usize,
) -> String {
    let needle = query.map(|s| s.to_ascii_lowercase());
    let mut out = String::new();
    let mut shown = 0usize;
    let mut matched = 0usize;

    for node in doc.bfs_order() {
        if !filter_matches(node, needle.as_deref(), roles, interactive_only) {
            continue;
        }
        matched += 1;
        if shown >= limit {
            continue;
        }
        render_flat_line(node, aliases, &mut out);
        shown += 1;
    }

    if out.is_empty() {
        out.push_str("# no nodes matched the filter\n");
    } else if matched > shown {
        out.push_str(&format!(
            "# … {} more matched; raise 'limit' or refine 'filter'/'roles' …\n",
            matched - shown
        ));
    }
    out
}

/// Whether a node passes the [`render_filtered`] predicate.
fn filter_matches(
    node: &SlugNode,
    needle: Option<&str>,
    roles: &[String],
    interactive_only: bool,
) -> bool {
    if interactive_only && !node.role.is_interactive() {
        return false;
    }
    if !roles.is_empty() {
        let name = node.role.yaml_name();
        if !roles.iter().any(|r| r == &name) {
            return false;
        }
    }
    if let Some(needle) = needle {
        let label = node.display_label().unwrap_or("").to_ascii_lowercase();
        if !label.contains(needle) {
            return false;
        }
    }
    true
}

/// Emit one flat (un-indented) node line for the filtered renderer, always
/// including centre `@x,y` when bounds are known (the agent may need to
/// `slug_click` it as a fallback).
fn render_flat_line(node: &SlugNode, aliases: &AliasTable, out: &mut String) {
    out.push_str("- ");
    out.push_str(&node.role.yaml_name());

    if let Some(label) = node.display_label() {
        out.push_str(" \"");
        out.push_str(&escape(label));
        out.push('"');
    }

    match aliases.alias_for(&node.slug_ref) {
        Some(alias) => {
            out.push_str(" [ref=");
            out.push_str(alias);
            out.push(']');
        }
        None => out.push_str(" [ref=?]"),
    }

    for state in salient_states(node) {
        out.push_str(" [");
        out.push_str(&state.to_ascii_lowercase());
        out.push(']');
    }

    if let Some(b) = node.bounds {
        let (cx, cy) = (b.x + b.width / 2.0, b.y + b.height / 2.0);
        out.push_str(&format!(" @{},{}", cx.round() as i64, cy.round() as i64));
    }

    out.push('\n');
}

/// The states to display for a node: salient present states, plus a synthesised
/// `disabled` for interactive nodes that lack `ENABLED`.
fn salient_states(node: &SlugNode) -> Vec<String> {
    let mut out: Vec<String> = node
        .states
        .iter()
        .filter(|s| s.is_salient())
        .map(|s| s.as_str().to_string())
        .collect();

    if node.role.is_interactive() && !node.has_state(SlugState::Enabled) {
        out.push("DISABLED".to_string());
    }
    out
}

/// Escape a label for inclusion in a double-quoted YAML scalar on one line.
fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")
}

#[allow(dead_code)]
fn role_hint(role: SlugRole) -> char {
    role.alias_prefix()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SlugNode;

    #[test]
    fn renders_indented_tree_with_aliases() {
        let mut root = SlugNode::new("R", SlugRole::Window);
        root.name = Some("Editor".into());
        root.child_refs = vec!["B".into(), "I".into()];

        let mut b = SlugNode::new("B", SlugRole::Button);
        b.parent_ref = Some("R".into());
        b.name = Some("Save".into());
        b.states = vec![SlugState::Enabled];

        let mut i = SlugNode::new("I", SlugRole::Entry);
        i.parent_ref = Some("R".into());
        i.name = Some("name".into());
        i.states = vec![SlugState::Enabled, SlugState::Focused];

        let doc = SlugDocument::from_nodes([root, b, i]);
        let mut aliases = AliasTable::new();
        let yaml = doc.to_yaml_assigning(&mut aliases);

        let expected = "\
- window \"Editor\" [ref=e1]
  - button \"Save\" [ref=b1]
  - entry \"name\" [ref=i1] [focused]
";
        assert_eq!(yaml, expected);
    }

    #[test]
    fn renders_center_coords_for_opaque_surfaces_only() {
        use crate::Bounds;
        // A Generic surface (e.g. a canvas) with bounds gets a @cx,cy hint.
        let mut canvas = SlugNode::new("C", SlugRole::Generic);
        canvas.bounds = Some(Bounds { x: 100.0, y: 200.0, width: 40.0, height: 20.0 });
        // A normal button with bounds does NOT (clicked by ref, saves tokens).
        let mut btn = SlugNode::new("B", SlugRole::Button);
        btn.name = Some("Save".into());
        btn.bounds = Some(Bounds { x: 0.0, y: 0.0, width: 10.0, height: 10.0 });
        btn.states = vec![SlugState::Enabled];
        let doc = SlugDocument::from_nodes([canvas, btn]);
        let mut aliases = AliasTable::new();
        let yaml = doc.to_yaml_assigning(&mut aliases);
        assert!(yaml.contains("@120,210"), "canvas center expected, got: {yaml}");
        assert!(!yaml.contains("@5,5"), "button must not carry coords, got: {yaml}");
    }

    fn sample_doc() -> SlugDocument {
        use crate::Bounds;
        let mut win = SlugNode::new("W", SlugRole::Window);
        win.name = Some("Shop".into());
        win.child_refs = vec!["B1".into(), "B2".into(), "I".into(), "T".into()];

        let mut b1 = SlugNode::new("B1", SlugRole::Button);
        b1.parent_ref = Some("W".into());
        b1.name = Some("Add to Basket".into());
        b1.states = vec![SlugState::Enabled];
        b1.bounds = Some(Bounds { x: 800.0, y: 530.0, width: 24.0, height: 20.0 });

        let mut b2 = SlugNode::new("B2", SlugRole::Button);
        b2.parent_ref = Some("W".into());
        b2.name = Some("Buy now".into());
        b2.states = vec![SlugState::Enabled];

        let mut i = SlugNode::new("I", SlugRole::Entry);
        i.parent_ref = Some("W".into());
        i.name = Some("Search".into());
        i.states = vec![SlugState::Enabled, SlugState::Focused];

        let mut t = SlugNode::new("T", SlugRole::StaticText);
        t.parent_ref = Some("W".into());
        t.text_content = Some("1. e4 e5".into());

        SlugDocument::from_nodes([win, b1, b2, i, t])
    }

    #[test]
    fn filtered_flat_list_by_query_carries_ref_and_coords() {
        let doc = sample_doc();
        let mut aliases = AliasTable::new();
        for n in doc.bfs_order() {
            aliases.assign(&n.slug_ref, n.role);
        }
        let yaml = render_filtered(&doc, &aliases, Some("basket"), &[], false, 50);
        // Only the matching button, flat (no indentation), with ref + centre coords.
        assert!(yaml.starts_with("- button \"Add to Basket\""), "got: {yaml}");
        assert!(yaml.contains("[ref=b1]"), "got: {yaml}");
        assert!(yaml.contains("@812,540"), "centre coords expected, got: {yaml}");
        assert!(!yaml.contains("Buy now"), "non-matching node leaked: {yaml}");
    }

    #[test]
    fn filtered_by_role_only() {
        let doc = sample_doc();
        let mut aliases = AliasTable::new();
        for n in doc.bfs_order() {
            aliases.assign(&n.slug_ref, n.role);
        }
        let yaml = render_filtered(&doc, &aliases, None, &["static_text".to_string()], false, 50);
        assert!(yaml.contains("1. e4 e5"), "move text expected, got: {yaml}");
        assert!(!yaml.contains("button"), "non-text role leaked: {yaml}");
    }

    #[test]
    fn filtered_interactive_only_drops_static_text() {
        let doc = sample_doc();
        let mut aliases = AliasTable::new();
        for n in doc.bfs_order() {
            aliases.assign(&n.slug_ref, n.role);
        }
        let yaml = render_filtered(&doc, &aliases, None, &[], true, 50);
        assert!(yaml.contains("Add to Basket"));
        assert!(yaml.contains("Search"));
        assert!(!yaml.contains("e4 e5"), "static text must be dropped: {yaml}");
    }

    #[test]
    fn filtered_limit_reports_overflow() {
        let doc = sample_doc();
        let mut aliases = AliasTable::new();
        for n in doc.bfs_order() {
            aliases.assign(&n.slug_ref, n.role);
        }
        let yaml = render_filtered(&doc, &aliases, None, &["button".to_string()], false, 1);
        // Two buttons match, but only one is shown.
        assert_eq!(yaml.matches("- button").count(), 1, "got: {yaml}");
        assert!(yaml.contains("1 more matched"), "overflow note expected, got: {yaml}");
    }

    #[test]
    fn filtered_no_match_is_explicit() {
        let doc = sample_doc();
        let aliases = AliasTable::new();
        let yaml = render_filtered(&doc, &aliases, Some("nonexistent"), &[], false, 50);
        assert!(yaml.contains("no nodes matched"), "got: {yaml}");
    }

    #[test]
    fn synthesises_disabled() {
        let mut b = SlugNode::new("B", SlugRole::Button);
        b.name = Some("Off".into());
        // no ENABLED state
        let doc = SlugDocument::from_nodes([b]);
        let mut aliases = AliasTable::new();
        let yaml = doc.to_yaml_assigning(&mut aliases);
        assert!(yaml.contains("[disabled]"), "got: {yaml}");
    }
}
