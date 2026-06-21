//! Shared, dependency-free destructive-action detection.
//!
//! Lives in `slug-core` (not `slug-brain`) so **both** the autonomous agent and
//! the MCP server can gate the same set of actions without a crate cycle. The
//! agent confirms destructive actions in its own loop; the MCP server enforces a
//! policy at the transport boundary for *external* clients (e.g. Claude Code),
//! which never pass through the agent's confirmation.

/// Keywords that make an action destructive / hard to reverse.
pub const DESTRUCTIVE_KEYWORDS: &[&str] = &[
    "delete", "remove", "trash", "discard", "erase", "destroy", "wipe", "format",
    "send", "submit", "post", "publish", "share",
    "purchase", "buy", "pay", "checkout", "order", "subscribe",
    "uninstall", "deactivate", "disable", "shut down", "shutdown", "log out", "logout",
    "confirm", "overwrite", "replace all",
];

/// Whether an action is destructive, based on the action verb plus any
/// label/args/reasoning text. Case-insensitive substring match against
/// [`DESTRUCTIVE_KEYWORDS`].
pub fn is_destructive(action: &str, context: Option<&str>, args: Option<&str>) -> bool {
    let mut hay = action.to_ascii_lowercase();
    if let Some(c) = context {
        hay.push(' ');
        hay.push_str(&c.to_ascii_lowercase());
    }
    if let Some(a) = args {
        hay.push(' ');
        hay.push_str(&a.to_ascii_lowercase());
    }
    DESTRUCTIVE_KEYWORDS.iter().any(|k| hay.contains(k))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_destructive_verbs_and_labels() {
        assert!(is_destructive("click", Some("Delete account"), None));
        assert!(is_destructive("click", Some("Send email"), None));
        assert!(is_destructive("click", Some("Buy now"), None));
        assert!(is_destructive("submit", None, None));
        // args carry the signal too
        assert!(is_destructive("set_text", Some("compose"), Some("please delete everything")));
    }

    #[test]
    fn leaves_safe_actions_alone() {
        assert!(!is_destructive("click", Some("Cancel"), None));
        assert!(!is_destructive("focus", Some("Search box"), None));
        assert!(!is_destructive("set_value", Some("Volume"), Some("0.5")));
    }
}
