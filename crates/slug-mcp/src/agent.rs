//! Agent control: supervise a `slug-brain` run through the *same* MCP transport.
//!
//! To avoid a crate cycle (`slug-brain` depends on `slug-mcp`), the controller
//! drives the `slug-agent` binary as a child process with `--jsonl`, parsing its
//! JSON-lines event stream (status/step/final) into a live log. The MCP tools
//! `slug_agent_start_task` / `slug_agent_status` / `slug_agent_pause` /
//! `slug_agent_resume` / `slug_agent_stop` expose it to humans and clients alike.

use std::collections::VecDeque;
use std::process::Stdio;
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

/// How many recent log lines to retain.
const LOG_CAP: usize = 200;
/// How many to return from `status`.
const STATUS_LINES: usize = 20;

#[derive(Default)]
struct AgentState {
    child: Option<Child>,
    task: Option<String>,
    provider: Option<String>,
    tier: Option<String>,
    model: Option<String>,
    status: String,
    paused: bool,
    steps: u64,
    tokens: u64,
    cost_usd: f64,
    started_at: Option<std::time::Instant>,
    log: VecDeque<String>,
    /// Last few stderr lines from the child, surfaced if it dies unexpectedly so
    /// the user sees *why* a task did nothing instead of a silent "done".
    err_tail: VecDeque<String>,
}

impl AgentState {
    fn push_log(&mut self, line: impl Into<String>) {
        self.log.push_back(line.into());
        while self.log.len() > LOG_CAP {
            self.log.pop_front();
        }
    }
}

/// Supervises one `slug-agent` child at a time.
pub struct AgentController {
    state: Arc<Mutex<AgentState>>,
    agent_bin: String,
    approvals: Arc<crate::approval::ApprovalRegistry>,
}

impl AgentController {
    /// Build a controller, locating the `slug-agent` binary (`$SLUG_AGENT_BIN`,
    /// else next to the current executable, else on `PATH`).
    pub fn new() -> Arc<Self> {
        let agent_bin = locate_agent_bin();
        debug!(agent_bin, "agent controller ready");
        Arc::new(AgentController {
            state: Arc::new(Mutex::new(AgentState { status: "idle".into(), ..Default::default() })),
            agent_bin,
            approvals: Arc::new(crate::approval::ApprovalRegistry::new()),
        })
    }

    /// The shared registry of pending destructive-action approvals (the
    /// dashboard lists/decides them; the tool gate waits on them).
    pub fn approvals(&self) -> Arc<crate::approval::ApprovalRegistry> {
        self.approvals.clone()
    }

    /// Start a new task. Errors if one is already running.
    pub async fn start_task(&self, description: &str) -> Result<String, String> {
        let mut st = self.state.lock().await;
        if st.status == "running" || st.status == "paused" {
            return Err("an agent task is already running; stop it first".into());
        }

        // Pre-flight: the built-in agent needs an AI provider + key. If none is
        // configured we'd otherwise spawn a child that exits instantly and shows a
        // misleading "done". Surface the real reason up front instead.
        if let Err(hint) = crate::dashboard_api::brain_ready() {
            st.task = Some(description.to_string());
            st.status = "error".into();
            st.paused = false;
            st.started_at = Some(std::time::Instant::now());
            st.log.clear();
            st.push_log(format!("▶ task: {description}"));
            st.push_log(format!("✗ {hint}"));
            return Ok("brain not ready — see the Brain tab".into());
        }

        let mut cmd = Command::new(&self.agent_bin);
        cmd.arg("--jsonl").arg(description);
        if let Ok(cfg) = std::env::var("SLUG_CONFIG") {
            cmd.arg("--config").arg(cfg);
        }
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("failed to launch '{}': {e}", self.agent_bin))?;
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");

        // Reset state for the new run.
        st.task = Some(description.to_string());
        st.status = "running".into();
        st.paused = false;
        st.provider = None;
        st.tier = None;
        st.model = None;
        st.steps = 0;
        st.tokens = 0;
        st.cost_usd = 0.0;
        st.started_at = Some(std::time::Instant::now());
        st.log.clear();
        st.push_log(format!("▶ task: {description}"));
        st.child = Some(child);
        drop(st);

        // Capture stderr (human logs) into a small ring so we can explain a crash,
        // and echo ERROR/WARN lines into the visible activity log.
        let state = self.state.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let mut st = state.lock().await;
                st.err_tail.push_back(line.clone());
                while st.err_tail.len() > 12 {
                    st.err_tail.pop_front();
                }
                if line.contains("ERROR") || line.contains("WARN") {
                    st.push_log(format!("· {line}"));
                }
            }
        });

        // Parse the JSONL event stream on stdout. When it closes, reap the child
        // and decide the terminal state from its exit code — a non-zero exit (or a
        // silent immediate exit with no `final`) becomes a visible *error*, never a
        // misleading "done".
        let state = self.state.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            let mut saw_final = false;
            while let Ok(Some(line)) = lines.next_line().await {
                let mut st = state.lock().await;
                if ingest_event(&mut st, &line) {
                    saw_final = true;
                }
            }
            // Stream closed: reap the child for its exit status (unless the user
            // already stopped it, which takes the handle).
            let child = state.lock().await.child.take();
            let exit_ok = match child {
                Some(mut c) => c.wait().await.map(|s| s.success()).unwrap_or(false),
                None => true,
            };
            let mut st = state.lock().await;
            match st.status.as_str() {
                "stopped" | "error" => {}
                _ if saw_final => st.status = "done".into(),
                _ if exit_ok => {
                    st.status = "done".into();
                    st.push_log("✓ finished");
                }
                _ => {
                    st.status = "error".into();
                    let tail: Vec<String> = st.err_tail.iter().cloned().collect();
                    if tail.is_empty() {
                        st.push_log("✗ the agent exited immediately without doing anything.");
                    } else {
                        for l in tail {
                            st.push_log(format!("✗ {l}"));
                        }
                    }
                    st.push_log(
                        "✗ Hint: the built-in agent needs an AI provider + API key — open the \
                         Brain tab and Connect one. (A connected MCP client like Claude Code \
                         drives Slug directly and does NOT need this.)",
                    );
                }
            }
            info!("agent task stream ended ({})", st.status);
        });

        Ok("task started".into())
    }

    /// A status snapshot (task, last lines, provider/tier).
    pub async fn status(&self) -> Value {
        let st = self.state.lock().await;
        let recent: Vec<&String> = st.log.iter().rev().take(STATUS_LINES).collect();
        let recent: Vec<&String> = recent.into_iter().rev().collect();
        let elapsed_s = st.started_at.map(|t| t.elapsed().as_secs()).unwrap_or(0);
        json!({
            "status": st.status,
            "paused": st.paused,
            "task": st.task,
            "provider": st.provider,
            "tier": st.tier,
            "model": st.model,
            "steps": st.steps,
            "tokens": st.tokens,
            "cost_usd": st.cost_usd,
            "elapsed_s": elapsed_s,
            "log": recent,
        })
    }

    pub async fn pause(&self) -> Result<String, String> {
        let pid = self.running_pid().await?;
        signal(pid, "-STOP").await?;
        let mut st = self.state.lock().await;
        st.paused = true;
        st.status = "paused".into();
        st.push_log("⏸ paused");
        Ok("paused".into())
    }

    pub async fn resume(&self) -> Result<String, String> {
        let pid = self.running_pid().await?;
        signal(pid, "-CONT").await?;
        let mut st = self.state.lock().await;
        st.paused = false;
        st.status = "running".into();
        st.push_log("▶ resumed");
        Ok("resumed".into())
    }

    pub async fn stop(&self) -> Result<String, String> {
        let mut st = self.state.lock().await;
        match st.child.take() {
            Some(mut child) => {
                let _ = child.start_kill();
                st.status = "stopped".into();
                st.paused = false;
                st.push_log("■ stopped");
                Ok("stopped".into())
            }
            None => Err("no running task".into()),
        }
    }

    async fn running_pid(&self) -> Result<u32, String> {
        let st = self.state.lock().await;
        st.child
            .as_ref()
            .and_then(|c| c.id())
            .ok_or_else(|| "no running task".to_string())
    }
}

/// Translate one JSONL event line into log/state updates. Returns `true` when the
/// line was the terminal `final` event (so the caller knows the run completed
/// cleanly rather than crashing).
fn ingest_event(st: &mut AgentState, line: &str) -> bool {
    let v: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => {
            st.push_log(line.to_string());
            return false;
        }
    };
    match v.get("kind").and_then(Value::as_str) {
        Some("status") => {
            st.provider = v.get("provider").and_then(Value::as_str).map(str::to_string);
            st.tier = v.get("tier").and_then(Value::as_str).map(str::to_string);
            st.model = v.get("model").and_then(Value::as_str).map(str::to_string);
            st.push_log(format!(
                "● provider={} tier={} model={}",
                st.provider.as_deref().unwrap_or("?"),
                st.tier.as_deref().unwrap_or("?"),
                st.model.as_deref().unwrap_or("?"),
            ));
        }
        Some("task") => {
            // already logged at start; ignore duplicate
        }
        Some("step") => {
            let step = v.get("step").and_then(Value::as_u64).unwrap_or(0);
            let tool = v.get("tool").and_then(Value::as_str).unwrap_or("");
            let reasoning = v.get("reasoning").and_then(Value::as_str).unwrap_or("");
            let result = v.get("result").and_then(Value::as_str).unwrap_or("");
            let first = result.lines().next().unwrap_or("");
            let err = if v.get("is_error").and_then(Value::as_bool).unwrap_or(false) {
                " [ERROR]"
            } else {
                ""
            };
            st.push_log(format!("→ [{step}] {tool}: {reasoning} ⇒ {first}{err}"));
        }
        Some("final") => {
            let answer = v.get("answer").and_then(Value::as_str).unwrap_or("");
            st.push_log(format!("✓ {answer}"));
            st.status = "done".into();
            return true;
        }
        _ => st.push_log(line.to_string()),
    }
    false
}

/// Send a job-control signal to a pid via `kill` (portable across macOS/Linux).
async fn signal(pid: u32, sig: &str) -> Result<(), String> {
    let ok = Command::new("kill")
        .arg(sig)
        .arg(pid.to_string())
        .status()
        .await
        .map_err(|e| format!("kill {sig} {pid}: {e}"))?
        .success();
    if ok {
        Ok(())
    } else {
        Err(format!("kill {sig} {pid} failed"))
    }
}

fn locate_agent_bin() -> String {
    if let Ok(p) = std::env::var("SLUG_AGENT_BIN") {
        return p;
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let cand = dir.join("slug-agent");
            if cand.exists() {
                return cand.to_string_lossy().into_owned();
            }
        }
    }
    warn!("slug-agent not found next to slug-mcp; relying on PATH");
    "slug-agent".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fresh_controller_is_idle() {
        let ctrl = AgentController::new();
        let s = ctrl.status().await;
        assert_eq!(s["status"], "idle");
        assert!(s["task"].is_null());
        // Controlling with no task errors cleanly (never panics).
        assert!(ctrl.pause().await.is_err());
        assert!(ctrl.stop().await.is_err());
    }

    #[test]
    fn ingest_parses_jsonl_events() {
        let mut st = AgentState { status: "running".into(), ..Default::default() };
        ingest_event(&mut st, r#"{"kind":"status","provider":"openai","tier":"TIER_CLOUD","model":"gpt-4o"}"#);
        assert_eq!(st.provider.as_deref(), Some("openai"));
        assert_eq!(st.tier.as_deref(), Some("TIER_CLOUD"));

        ingest_event(&mut st, r#"{"kind":"step","step":1,"tool":"slug_invoke","reasoning":"click save","result":"ok","is_error":false}"#);
        assert!(st.log.iter().any(|l| l.contains("slug_invoke") && l.contains("click save")));

        ingest_event(&mut st, r#"{"kind":"final","answer":"done","step":2}"#);
        assert_eq!(st.status, "done");
        assert!(st.log.iter().any(|l| l.contains("✓ done")));

        // Non-JSON lines are kept verbatim, never panic.
        ingest_event(&mut st, "raw line");
        assert!(st.log.iter().any(|l| l == "raw line"));
    }
}
