//! Safety rails: per-session token/cost caps, destructive-action confirmation,
//! and a structured action log with undo of the last action.

use serde::Serialize;

use crate::backend::Usage;

/// Cumulative per-session budget. Tracks tokens and (cloud) cost against caps.
#[derive(Debug, Clone)]
pub struct Budget {
    pub max_tokens: u64,
    pub max_cost_usd: f64,
    pub used_tokens: u64,
    pub used_cost_usd: f64,
}

impl Budget {
    pub fn new(max_tokens: u64, max_cost_usd: f64) -> Self {
        Budget { max_tokens, max_cost_usd, used_tokens: 0, used_cost_usd: 0.0 }
    }

    /// Record a completion's usage and cost.
    pub fn record(&mut self, usage: Usage, cost_usd: f64) {
        self.used_tokens += usage.input_tokens + usage.output_tokens;
        self.used_cost_usd += cost_usd;
    }

    /// If a cap has been exceeded, return a human-readable reason.
    pub fn exceeded(&self) -> Option<String> {
        if self.max_tokens > 0 && self.used_tokens >= self.max_tokens {
            return Some(format!(
                "token cap reached: {} / {} tokens",
                self.used_tokens, self.max_tokens
            ));
        }
        if self.max_cost_usd > 0.0 && self.used_cost_usd >= self.max_cost_usd {
            return Some(format!(
                "cost cap reached: ${:.4} / ${:.2}",
                self.used_cost_usd, self.max_cost_usd
            ));
        }
        None
    }
}

/// Keywords that make an action destructive / hard to reverse.
const DESTRUCTIVE_KEYWORDS: &[&str] = &[
    "delete", "remove", "trash", "discard", "erase", "destroy", "wipe", "format",
    "send", "submit", "post", "publish", "share",
    "purchase", "buy", "pay", "checkout", "order", "subscribe",
    "uninstall", "deactivate", "disable", "shut down", "shutdown", "log out", "logout",
    "confirm", "overwrite", "replace all",
];

/// Whether an action on a node is destructive, based on the action verb and the
/// node's label/args text. Pattern-matched per the task brief.
pub fn is_destructive(action: &str, target_label: Option<&str>, args: Option<&str>) -> bool {
    let mut hay = action.to_ascii_lowercase();
    if let Some(l) = target_label {
        hay.push(' ');
        hay.push_str(&l.to_ascii_lowercase());
    }
    if let Some(a) = args {
        hay.push(' ');
        hay.push_str(&a.to_ascii_lowercase());
    }
    DESTRUCTIVE_KEYWORDS.iter().any(|k| hay.contains(k))
}

/// A confirmation gate for destructive actions.
pub trait ConfirmHook: Send + Sync {
    /// Return `true` to allow the action described by `summary`.
    fn confirm(&self, summary: &str) -> bool;
}

/// Allow everything (non-interactive / trusted).
pub struct AllowAll;
impl ConfirmHook for AllowAll {
    fn confirm(&self, _summary: &str) -> bool {
        true
    }
}

/// Deny everything (dry-run / non-interactive safe default).
pub struct DenyAll;
impl ConfirmHook for DenyAll {
    fn confirm(&self, summary: &str) -> bool {
        tracing::warn!(%summary, "destructive action auto-denied (non-interactive)");
        false
    }
}

/// Prompt on the terminal (`y/N`).
pub struct CliConfirm;
impl ConfirmHook for CliConfirm {
    fn confirm(&self, summary: &str) -> bool {
        use std::io::Write;
        eprint!("\n[confirm] {summary}\nProceed? [y/N] ");
        let _ = std::io::stderr().flush();
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_err() {
            return false;
        }
        matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
    }
}

/// One recorded action.
#[derive(Clone, Debug, Serialize)]
pub struct ActionRecord {
    pub tool: String,
    pub args: serde_json::Value,
    pub reasoning: String,
    pub result: String,
    pub is_error: bool,
    /// An inverse action that would undo this one, if known.
    pub undo: Option<UndoAction>,
}

/// A best-effort inverse for [`ActionRecord`].
#[derive(Clone, Debug, Serialize)]
pub struct UndoAction {
    pub slug_ref: String,
    pub action: String,
    pub args: Option<String>,
}

/// An append-only log of executed actions, supporting undo of the last one.
#[derive(Debug, Default)]
pub struct ActionLog {
    entries: Vec<ActionRecord>,
}

impl ActionLog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, record: ActionRecord) {
        self.entries.push(record);
    }

    pub fn entries(&self) -> &[ActionRecord] {
        &self.entries
    }

    pub fn last(&self) -> Option<&ActionRecord> {
        self.entries.last()
    }

    /// Pop the last action and return its undo plan (if any). The caller executes
    /// the undo against the bridge/session.
    pub fn take_undo(&mut self) -> Option<UndoAction> {
        let rec = self.entries.pop()?;
        rec.undo
    }

    /// Compute a best-effort inverse for a slug_invoke action.
    ///
    /// * `set_text` → restore the previously-observed text (`prior`).
    /// * `set_value` → restore the previous numeric value.
    /// * `toggle`/`check`/`uncheck` → re-toggle.
    /// * everything else (click, focus) → no automatic undo.
    pub fn infer_undo(
        slug_ref: &str,
        action: &str,
        prior: Option<&str>,
    ) -> Option<UndoAction> {
        match action.to_ascii_lowercase().as_str() {
            "set_text" => Some(UndoAction {
                slug_ref: slug_ref.into(),
                action: "set_text".into(),
                args: Some(prior.unwrap_or("").to_string()),
            }),
            "set_value" => prior.map(|p| UndoAction {
                slug_ref: slug_ref.into(),
                action: "set_value".into(),
                args: Some(p.to_string()),
            }),
            "toggle" | "check" | "uncheck" => Some(UndoAction {
                slug_ref: slug_ref.into(),
                action: "toggle".into(),
                args: None,
            }),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_caps() {
        let mut b = Budget::new(100, 0.0);
        assert!(b.exceeded().is_none());
        b.record(Usage { input_tokens: 60, output_tokens: 50 }, 0.0);
        assert!(b.exceeded().is_some(), "110 >= 100 tokens");
    }

    #[test]
    fn cost_cap() {
        let mut b = Budget::new(0, 0.50);
        b.record(Usage { input_tokens: 1, output_tokens: 1 }, 0.49);
        assert!(b.exceeded().is_none());
        b.record(Usage::default(), 0.02);
        assert!(b.exceeded().is_some());
    }

    #[test]
    fn destructive_detection() {
        assert!(is_destructive("click", Some("Delete account"), None));
        assert!(is_destructive("click", Some("Send email"), None));
        assert!(is_destructive("click", Some("Buy now"), None));
        assert!(!is_destructive("click", Some("Cancel"), None));
        assert!(!is_destructive("focus", Some("Search box"), None));
        assert!(is_destructive("set_text", Some("Recipient"), Some("please delete everything")));
    }

    #[test]
    fn undo_inference() {
        let u = ActionLog::infer_undo("b1", "set_text", Some("old")).unwrap();
        assert_eq!(u.action, "set_text");
        assert_eq!(u.args.as_deref(), Some("old"));
        assert!(ActionLog::infer_undo("b1", "click", None).is_none());
    }

    #[test]
    fn log_take_undo() {
        let mut log = ActionLog::new();
        log.push(ActionRecord {
            tool: "slug_invoke".into(),
            args: serde_json::json!({"ref":"b1","action":"toggle"}),
            reasoning: "test".into(),
            result: "ok".into(),
            is_error: false,
            undo: ActionLog::infer_undo("b1", "toggle", None),
        });
        let undo = log.take_undo().unwrap();
        assert_eq!(undo.action, "toggle");
        assert!(log.last().is_none());
    }
}
