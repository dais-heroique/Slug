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

    out.push('\n');

    for child in &node.child_refs {
        render_node(doc, child, aliases, depth + 1, out);
    }
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
