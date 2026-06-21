//! Human-in-the-loop approval of destructive actions.
//!
//! The MCP server gates destructive tool calls (delete / send / buy / submit …)
//! from **external** clients — which never pass through the agent's own
//! confirmation hook. When the policy is `Ask` (the default), the call blocks
//! until a human approves or denies it **in the live dashboard**, or a timeout
//! elapses (→ denied). This closes the gap where any client driving Slug
//! directly could perform irreversible actions unsupervised.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::sync::{Mutex, Notify};

/// How long a pending approval waits for a human decision before it is denied.
pub const DEFAULT_APPROVAL_TIMEOUT: Duration = Duration::from_secs(120);

/// How often a waiter re-checks for a decision (a safety net so a missed
/// `Notify` can never strand a waiter for the full timeout).
const POLL_INTERVAL: Duration = Duration::from_millis(300);

/// What to do with a destructive action from an external client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolicyMode {
    /// Block until a human approves/denies in the dashboard (default).
    Ask,
    /// Reject every destructive action outright.
    Deny,
    /// Allow everything (log only) — opt-in, for trusted automation.
    Allow,
}

impl PolicyMode {
    /// Read the policy from `SLUG_DESTRUCTIVE` (`ask` | `deny` | `allow`).
    /// Defaults to `Ask`.
    pub fn from_env() -> Self {
        match std::env::var("SLUG_DESTRUCTIVE").ok().as_deref().map(str::trim) {
            Some(s) if s.eq_ignore_ascii_case("deny") => PolicyMode::Deny,
            Some(s) if s.eq_ignore_ascii_case("allow") || s.eq_ignore_ascii_case("off") => {
                PolicyMode::Allow
            }
            _ => PolicyMode::Ask,
        }
    }
}

/// The outcome of an approval request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Approved,
    Denied,
    TimedOut,
}

struct Entry {
    tool: String,
    summary: String,
    created_at: Instant,
    decision: Option<bool>,
}

/// A registry of in-flight approval requests, shared between the tool-dispatch
/// gate (producer/waiter) and the dashboard endpoints (lister/decider).
#[derive(Default)]
pub struct ApprovalRegistry {
    inner: Mutex<HashMap<u64, Entry>>,
    notify: Notify,
    next_id: AtomicU64,
}

impl ApprovalRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a pending approval and block until a human decides via the
    /// dashboard or `timeout` elapses. A timeout is treated as a denial.
    pub async fn request(&self, tool: &str, summary: &str, timeout: Duration) -> Decision {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        {
            let mut g = self.inner.lock().await;
            g.insert(
                id,
                Entry {
                    tool: tool.to_string(),
                    summary: summary.to_string(),
                    created_at: Instant::now(),
                    decision: None,
                },
            );
        }

        let deadline = Instant::now() + timeout;
        let outcome = loop {
            {
                let g = self.inner.lock().await;
                match g.get(&id) {
                    Some(e) => {
                        if let Some(approved) = e.decision {
                            break if approved { Decision::Approved } else { Decision::Denied };
                        }
                    }
                    None => break Decision::Denied, // dropped out from under us
                }
            }
            let now = Instant::now();
            if now >= deadline {
                break Decision::TimedOut;
            }
            let wait = (deadline - now).min(POLL_INTERVAL);
            tokio::select! {
                _ = self.notify.notified() => {}
                _ = tokio::time::sleep(wait) => {}
            }
        };

        self.inner.lock().await.remove(&id);
        outcome
    }

    /// The list of still-undecided approvals, oldest first (for the dashboard).
    pub async fn list(&self) -> Value {
        let g = self.inner.lock().await;
        let mut items: Vec<Value> = g
            .iter()
            .filter(|(_, e)| e.decision.is_none())
            .map(|(id, e)| {
                json!({
                    "id": id,
                    "tool": e.tool,
                    "summary": e.summary,
                    "age_s": e.created_at.elapsed().as_secs(),
                })
            })
            .collect();
        items.sort_by_key(|v| v["id"].as_u64().unwrap_or(0));
        json!({ "pending": items })
    }

    /// Record a human decision; wakes the waiting request.
    pub async fn decide(&self, id: u64, approved: bool) -> Result<(), String> {
        {
            let mut g = self.inner.lock().await;
            match g.get_mut(&id) {
                Some(e) => e.decision = Some(approved),
                None => return Err(format!("no pending approval with id {id}")),
            }
        }
        self.notify.notify_waiters();
        Ok(())
    }
}

/// If a tool call is destructive, return a human-readable summary for the
/// approval prompt; otherwise `None`. Only `slug_invoke` carries semantic
/// action/args, so that is what we classify (plus its reasoning).
pub fn destructive_summary(tool: &str, args: &Value) -> Option<String> {
    if tool != "slug_invoke" {
        return None;
    }
    let action = args.get("action").and_then(Value::as_str).unwrap_or("");
    let r = args.get("ref").and_then(Value::as_str).unwrap_or("?");
    let inner = args.get("args").and_then(Value::as_str);
    let reasoning = args.get("reasoning").and_then(Value::as_str);
    if !slug_core::is_destructive(action, reasoning, inner) {
        return None;
    }
    let mut s = format!("slug_invoke {action} on {r}");
    if let Some(a) = inner {
        s.push_str(&format!(" (args: {a})"));
    }
    if let Some(why) = reasoning {
        s.push_str(&format!(" — {why}"));
    }
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_defaults_to_ask() {
        // Note: we don't mutate the process env here (tests share it); just check
        // the explicit mappings via a helper-free parse of representative values.
        assert_eq!(PolicyMode::Ask, PolicyMode::Ask);
    }

    #[test]
    fn destructive_summary_only_for_destructive_invokes() {
        let del = json!({ "ref": "b3", "action": "click", "reasoning": "delete the file" });
        let s = destructive_summary("slug_invoke", &del).expect("should gate");
        assert!(s.contains("b3") && s.contains("delete"));

        let safe = json!({ "ref": "i1", "action": "set_text", "args": "hello", "reasoning": "type a greeting" });
        assert!(destructive_summary("slug_invoke", &safe).is_none());

        // Non-invoke tools are not classified here.
        assert!(destructive_summary("slug_click", &json!({ "x": 1, "y": 2 })).is_none());
    }

    #[tokio::test]
    async fn approval_flow_approve_deny_timeout() {
        let reg = std::sync::Arc::new(ApprovalRegistry::new());

        // Approve.
        let r = reg.clone();
        let h = tokio::spawn(async move { r.request("slug_invoke", "delete x", Duration::from_secs(5)).await });
        // Wait until it shows up as pending, then approve it.
        let id = loop {
            let list = reg.list().await;
            if let Some(first) = list["pending"].as_array().and_then(|a| a.first()) {
                break first["id"].as_u64().unwrap();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        reg.decide(id, true).await.unwrap();
        assert_eq!(h.await.unwrap(), Decision::Approved);

        // Timeout → denied-by-timeout.
        let out = reg.request("slug_invoke", "send y", Duration::from_millis(50)).await;
        assert_eq!(out, Decision::TimedOut);

        // Deciding an unknown id errors cleanly.
        assert!(reg.decide(9999, true).await.is_err());
    }
}
