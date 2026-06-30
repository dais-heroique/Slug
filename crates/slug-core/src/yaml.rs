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

    // For opaque surfaces (canvas/graphics/media: Canvas, Image, Media, Generic)
    // that have geometry but typically no clickable child, expose the centre
    // coordinate so the agent can `slug_click` / `slug_scroll` into them — this is
    // how apps with no usable tree (games, chess boards, maps) are driven. Kept off
    // normal controls to avoid token bloat — those are clicked by ref.
    if node.role.is_opaque_surface() || node.role == SlugRole::Generic {
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
    coords: bool,
) -> String {
    let needle = query.map(|s| s.to_ascii_lowercase());

    // Collect matches, scored by how well the name matches the query so the best
    // hit comes first (so `limit: 1` returns exactly the control you meant).
    //
    // Speed: when there is NO text query there is nothing to rank, so we keep at
    // most `limit` nodes (and never allocate/sort a vector the size of the whole
    // tree) while still counting every match for the overflow note. With a query
    // we must see them all to rank, so we collect then stable-sort.
    let mut hits: Vec<(u8, &SlugNode)> = Vec::new();
    let mut matched = 0usize;
    for node in doc.bfs_order() {
        if !filter_matches(node, needle.as_deref(), roles, interactive_only) {
            continue;
        }
        matched += 1;
        match needle.as_deref() {
            Some(n) => hits.push((match_score(node, n), node)),
            None => {
                if hits.len() < limit {
                    hits.push((3, node));
                }
            }
        }
    }
    if needle.is_some() {
        // Stable sort by score (exact → prefix → word → contains); BFS order
        // within a score is preserved.
        hits.sort_by_key(|(s, _)| *s);
    }

    let mut out = String::new();
    for (_, node) in hits.iter().take(limit) {
        render_flat_line(node, aliases, coords, &mut out);
    }

    if out.is_empty() {
        out.push_str("# no nodes matched the filter\n");
    } else if matched > limit {
        out.push_str(&format!(
            "# … {} more matched; raise 'limit' or refine 'filter'/'roles' …\n",
            matched - limit
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
        let label = node.display_label().unwrap_or("");
        if !roles.iter().any(|r| role_matches_token(node.role, r, label)) {
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

/// Match a requested `roles` token against a node's role (and, for `price`,
/// its label). A token is either an exact lower-case role name (`button`,
/// `entry`, `static_text`, …) or a friendly **group**: `clickable` (any
/// actionable control), `field`/`input` (text entries, combos, spinners),
/// `text` (static text / labels / paragraphs / headings), `link`, `heading`,
/// `price`/`money` (any label that looks like a currency amount, in whatever
/// role it's rendered in — Amazon-style storefronts put the price in a plain
/// link or span, not a dedicated "price" role, so this matches on content,
/// not role).
fn role_matches_token(role: SlugRole, token: &str, label: &str) -> bool {
    use SlugRole::*;
    let t = token.trim().to_ascii_lowercase();
    match t.as_str() {
        "clickable" | "actionable" => role.is_interactive(),
        "field" | "input" | "textbox" => matches!(
            role,
            Entry | EntryMultiline | EntryPassword | EntrySearch | ComboBox | SpinButton
        ),
        "text" => matches!(role, StaticText | Label | Paragraph | Heading | Caption),
        "link" => role == Link,
        "heading" | "title" => role == Heading,
        "price" | "money" | "currency" => looks_like_price(label),
        other => role.yaml_name() == other,
    }
}

/// Currency symbols recognised by [`looks_like_price`]. Deliberately broad —
/// false positives just mean an extra line in an already-filtered result;
/// false negatives mean the agent can't find the price at all.
const CURRENCY_SYMBOLS: &[char] = &['$', '€', '£', '¥', '₹', '₩', '₽', '₺', '₫', '₪', '฿', '¢'];

/// ISO-4217-ish currency codes seen as a whole word next to an amount
/// (`"EUR 26,32"`, `"26.32 USD"`). Lower-case; matched case-insensitively.
const CURRENCY_CODES: &[&str] = &[
    "eur", "usd", "gbp", "chf", "jpy", "cad", "aud", "cny", "inr", "sek", "nok", "dkk", "pln",
    "czk", "huf", "brl", "mxn", "zar", "krw", "try", "rub", "sgd", "hkd", "nzd",
];

/// Whether a label looks like a currency amount — a currency symbol/code plus
/// a digit, in either order (`"$26.32"`, `"26,32 €"`, `"EUR 26,32"`). Content
/// based, not regex (no `regex` dependency in this crate): a plain symbol/code
/// scan is enough to separate "price-shaped" text from star ratings, review
/// counts, or quantities, which carry digits but never a currency marker.
fn looks_like_price(label: &str) -> bool {
    let has_digit = label.chars().any(|c| c.is_ascii_digit());
    if !has_digit {
        return false;
    }
    if label.chars().any(|c| CURRENCY_SYMBOLS.contains(&c)) {
        return true;
    }
    label
        .split(|c: char| !c.is_alphanumeric())
        .any(|w| !w.is_empty() && CURRENCY_CODES.contains(&w.to_ascii_lowercase().as_str()))
}

/// Score a node's label against the query: 0 exact, 1 prefix, 2 word-boundary,
/// 3 contains (lower = better). Assumes the node already passed `contains`.
fn match_score(node: &SlugNode, needle: &str) -> u8 {
    let label = node.display_label().unwrap_or("").to_ascii_lowercase();
    if label == needle {
        0
    } else if label.starts_with(needle) {
        1
    } else if label.split(|c: char| !c.is_alphanumeric()).any(|w| w == needle) {
        2
    } else {
        3
    }
}

/// Emit one flat (un-indented) node line for the filtered renderer. Centre `@x,y`
/// is included only when `coords` is requested, or for opaque surfaces (canvas/
/// image/media/generic) where clicking by coordinate is the usual path — normal
/// controls are clicked by `ref`, so omitting their coords keeps the result lean.
fn render_flat_line(node: &SlugNode, aliases: &AliasTable, coords: bool, out: &mut String) {
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

    let opaque = node.role.is_opaque_surface() || node.role == SlugRole::Generic;
    if coords || opaque {
        if let Some(b) = node.bounds {
            let (cx, cy) = (b.x + b.width / 2.0, b.y + b.height / 2.0);
            out.push_str(&format!(" @{},{}", cx.round() as i64, cy.round() as i64));
        }
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
        let mut generic = SlugNode::new("G", SlugRole::Generic);
        generic.bounds = Some(Bounds { x: 100.0, y: 200.0, width: 40.0, height: 20.0 });
        // A true Canvas (e.g. a chess board / game) must ALSO get coordinates.
        let mut canvas = SlugNode::new("C", SlugRole::Canvas);
        canvas.bounds = Some(Bounds { x: 300.0, y: 300.0, width: 100.0, height: 100.0 });
        // A normal button with bounds does NOT (clicked by ref, saves tokens).
        let mut btn = SlugNode::new("B", SlugRole::Button);
        btn.name = Some("Save".into());
        btn.bounds = Some(Bounds { x: 0.0, y: 0.0, width: 10.0, height: 10.0 });
        btn.states = vec![SlugState::Enabled];
        let doc = SlugDocument::from_nodes([generic, canvas, btn]);
        let mut aliases = AliasTable::new();
        let yaml = doc.to_yaml_assigning(&mut aliases);
        assert!(yaml.contains("@120,210"), "generic center expected, got: {yaml}");
        assert!(yaml.contains("@350,350"), "canvas center expected, got: {yaml}");
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

    fn aliased(doc: &SlugDocument) -> AliasTable {
        let mut a = AliasTable::new();
        for n in doc.bfs_order() {
            a.assign(&n.slug_ref, n.role);
        }
        a
    }

    #[test]
    fn filtered_flat_list_is_lean_then_coords_on_demand() {
        let doc = sample_doc();
        let aliases = aliased(&doc);
        // Lean by default: the matching button has its ref but NO coords.
        let lean = render_filtered(&doc, &aliases, Some("basket"), &[], false, 50, false);
        assert!(lean.starts_with("- button \"Add to Basket\""), "got: {lean}");
        assert!(lean.contains("[ref=b1]"), "got: {lean}");
        assert!(!lean.contains("@812,540"), "normal control should be lean: {lean}");
        assert!(!lean.contains("Buy now"), "non-matching node leaked: {lean}");
        // coords:true adds the centre coordinate for the click fallback.
        let withxy = render_filtered(&doc, &aliases, Some("basket"), &[], false, 50, true);
        assert!(withxy.contains("@812,540"), "coords requested but missing: {withxy}");
    }

    #[test]
    fn filtered_role_groups() {
        let doc = sample_doc();
        let aliases = aliased(&doc);
        // "field" group → only the entry, not buttons/text.
        let fields = render_filtered(&doc, &aliases, None, &["field".to_string()], false, 50, false);
        assert!(fields.contains("entry \"Search\""), "got: {fields}");
        assert!(!fields.contains("button"), "field group leaked a button: {fields}");
        // "clickable" group → the two buttons + the entry, not the static text.
        let click = render_filtered(&doc, &aliases, None, &["clickable".to_string()], false, 50, false);
        assert!(click.contains("Add to Basket") && click.contains("Buy now") && click.contains("Search"));
        assert!(!click.contains("e4 e5"), "static text is not clickable: {click}");
        // exact role still works.
        let txt = render_filtered(&doc, &aliases, None, &["static_text".to_string()], false, 50, false);
        assert!(txt.contains("1. e4 e5") && !txt.contains("button"));
    }

    #[test]
    fn price_group_matches_currency_text_regardless_of_role_or_symbol() {
        let mut win = SlugNode::new("W", SlugRole::Window);
        win.child_refs = vec!["P1".into(), "P2".into(), "P3".into(), "R".into(), "Q".into()];

        // European-style amount with the symbol after the number, in a plain
        // static text node (no literal "$" anywhere — this is the exact shape
        // that defeated a literal `filter:"$"` against an Amazon.fr page).
        let mut p1 = SlugNode::new("P1", SlugRole::StaticText);
        p1.parent_ref = Some("W".into());
        p1.text_content = Some("26,32 €".into());

        // US-style amount, symbol-first, wrapped in a Link (Amazon often makes
        // the price clickable as part of the product link).
        let mut p2 = SlugNode::new("P2", SlugRole::Link);
        p2.parent_ref = Some("W".into());
        p2.name = Some("$19.99".into());

        // Currency-code form with no symbol at all.
        let mut p3 = SlugNode::new("P3", SlugRole::StaticText);
        p3.parent_ref = Some("W".into());
        p3.text_content = Some("EUR 26,32".into());

        // A star rating and a review count: both carry digits but no currency
        // marker, so they must NOT be swept up by the price filter.
        let mut rating = SlugNode::new("R", SlugRole::StaticText);
        rating.parent_ref = Some("W".into());
        rating.text_content = Some("4.5 out of 5 stars".into());
        let mut reviews = SlugNode::new("Q", SlugRole::StaticText);
        reviews.parent_ref = Some("W".into());
        reviews.text_content = Some("2,014 ratings".into());

        let doc = SlugDocument::from_nodes([win, p1, p2, p3, rating, reviews]);
        let aliases = aliased(&doc);

        let prices = render_filtered(&doc, &aliases, None, &["price".to_string()], false, 50, false);
        assert!(prices.contains("26,32 €"), "got: {prices}");
        assert!(prices.contains("$19.99"), "got: {prices}");
        assert!(prices.contains("EUR 26,32"), "got: {prices}");
        assert!(!prices.contains("stars"), "rating leaked into price filter: {prices}");
        assert!(!prices.contains("ratings"), "review count leaked into price filter: {prices}");

        // "money" and "currency" are accepted aliases for the same group.
        let alias1 = render_filtered(&doc, &aliases, None, &["money".to_string()], false, 50, false);
        let alias2 = render_filtered(&doc, &aliases, None, &["currency".to_string()], false, 50, false);
        assert_eq!(prices, alias1);
        assert_eq!(prices, alias2);
    }

    #[test]
    fn filtered_ranks_exact_match_first() {
        use crate::Bounds;
        let mut win = SlugNode::new("W", SlugRole::Window);
        win.child_refs = vec!["A".into(), "B".into()];
        let mut a = SlugNode::new("A", SlugRole::Button);
        a.parent_ref = Some("W".into());
        a.name = Some("Send later".into()); // contains "send"
        a.states = vec![SlugState::Enabled];
        a.bounds = Some(Bounds { x: 0.0, y: 0.0, width: 2.0, height: 2.0 });
        let mut b = SlugNode::new("B", SlugRole::Button);
        b.parent_ref = Some("W".into());
        b.name = Some("Send".into()); // exact
        b.states = vec![SlugState::Enabled];
        let doc = SlugDocument::from_nodes([win, a, b]);
        let aliases = aliased(&doc);
        // limit 1 + query "send" → the EXACT "Send" wins even though "Send later"
        // appears first in document order.
        let top = render_filtered(&doc, &aliases, Some("send"), &["button".to_string()], false, 1, false);
        assert!(top.contains("\"Send\"") && !top.contains("Send later"), "got: {top}");
    }

    #[test]
    fn filtered_interactive_only_drops_static_text() {
        let doc = sample_doc();
        let aliases = aliased(&doc);
        let yaml = render_filtered(&doc, &aliases, None, &[], true, 50, false);
        assert!(yaml.contains("Add to Basket"));
        assert!(yaml.contains("Search"));
        assert!(!yaml.contains("e4 e5"), "static text must be dropped: {yaml}");
    }

    #[test]
    fn filtered_limit_reports_overflow() {
        let doc = sample_doc();
        let aliases = aliased(&doc);
        let yaml = render_filtered(&doc, &aliases, None, &["button".to_string()], false, 1, false);
        assert_eq!(yaml.matches("- button").count(), 1, "got: {yaml}");
        assert!(yaml.contains("1 more matched"), "overflow note expected, got: {yaml}");
    }

    #[test]
    fn filtered_no_match_is_explicit() {
        let doc = sample_doc();
        let aliases = AliasTable::new();
        let yaml = render_filtered(&doc, &aliases, Some("nonexistent"), &[], false, 50, false);
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
