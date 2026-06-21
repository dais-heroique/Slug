//! The hybrid agentic loop: observe → reason → act → verify.
//!
//! The brain holds one [`LlmBackend`] (local or cloud), an MCP [`Session`], and
//! the safety rails. Each iteration asks the model what to do with the Slug tools
//! available; executes any tool calls; and — after every action — re-snapshots the
//! focused window so the model can verify expected vs. actual state before
//! continuing. The loop ends when the model returns a final answer, a cap is hit,
//! or the step limit is reached.

use std::sync::Arc;

use serde_json::json;
use slug_mcp::Session;
use tracing::{info, warn};

use crate::backend::{estimate_cost, Content, LlmBackend, Msg, Role};
use crate::config::{Config, Selection};
use crate::hardware::{BackendKind, Report};
use crate::safety::{
    ActionLog, ActionRecord, AllowAll, Budget, CliConfirm, ConfirmHook, DenyAll,
};
use crate::{backend, tools};

const SYSTEM_PROMPT: &str = "\
You are Slug, an agent that operates the desktop (macOS, Windows, or Linux) \
through a semantic accessibility layer — never screenshots. You read the UI as a \
typed tree and act on nodes by their short ref (e.g. b1, e5).

Loop: observe → reason → act → verify.
- Open: if the app you need isn't running, call slug_launch with its name (e.g. \
  {\"name\":\"Spotify\"}), optionally a uri/deep link. Slug only drives running apps.
- Observe: call slug_snapshot (scope \"focused\" by default) to read the current \
  window. Each interactive node shows a [ref=…]; opaque surfaces may show @x,y. \
  Prefer focused/window scope to keep context small; use desktop scope only to \
  find another app.
  - FAST PATH: real pages are huge. To find a specific control, narrow the \
    snapshot server-side with filter (substring on the name), roles (e.g. \
    [\"button\"], [\"entry\",\"combo_box\"], [\"static_text\"]) and/or \
    interactive_only:true. You then get a compact flat list of just the matching \
    nodes, each already carrying its [ref=…] AND a centre @x,y. Use this instead of \
    reading the whole tree — it is far faster and cheaper.
- Act, in order of preference:
  - slug_invoke with the node's ref and an action (click, focus, set_text, \
    set_value, toggle, …) — this is the precise, preferred way to act on controls.
  - slug_key for the focused app: a key chord ({\"keys\":\"cmd+s\"}) or literal text \
    ({\"keys\":\"hello\",\"mode\":\"text\"}) — works even on apps with no tree.
  - slug_click {\"x\",\"y\"} only when there is no node to target (e.g. inside a \
    canvas), using an @x,y hint from the snapshot.
  - slug_scroll {\"x\",\"y\",\"dy\"} to reveal content that isn't visible yet \
    (negative dy scrolls down) — scroll over the relevant list/grid then re-snapshot.
  Always pass a short `reasoning` explaining why.
- If a target you expect isn't in the snapshot, it may be off-screen or behind a \
  search box: scroll the relevant area, or focus the app's search field (or press \
  its search shortcut) and type the name, before giving up.
- Verify: after each action a fresh post-action snapshot is returned to you — \
  check the state changed as expected before the next step. If not, re-observe.

Be decisive: when you can act, act. Don't re-snapshot needlessly. When the task \
is complete, stop and reply with a one or two sentence summary of what you did. \
If you are blocked or uncertain, say so plainly rather than guessing.";

/// Errors from the brain loop.
#[derive(Debug, thiserror::Error)]
pub enum BrainError {
    #[error(transparent)]
    Backend(#[from] backend::BackendError),
}

/// Outcome of a completed task run.
pub struct Outcome {
    /// The model's final answer (or the reason it stopped).
    pub answer: String,
    /// Number of model turns taken.
    pub steps: u32,
    /// Cumulative tokens used.
    pub tokens: u64,
    /// Cumulative estimated cost (USD; 0 for local).
    pub cost_usd: f64,
    /// Whether the run stopped because a cap/limit was hit rather than finishing.
    pub escalated: bool,
}

/// An observer notified of each reasoning/action step as a JSON event (used by
/// `slug-agent --jsonl` to stream a live log to the MCP dashboard).
pub type StepObserver = Box<dyn Fn(serde_json::Value) + Send + Sync>;

/// The agent.
pub struct Brain {
    backend: Box<dyn LlmBackend>,
    session: Arc<Session>,
    confirm: Box<dyn ConfirmHook>,
    budget: Budget,
    log: ActionLog,
    max_steps: u32,
    confirm_destructive: bool,
    observer: Option<StepObserver>,
}

impl Brain {
    /// Build a brain from config + hardware report, choosing the backend per the
    /// selection policy. `interactive` controls the destructive-action confirm hook.
    pub fn from_config(cfg: &Config, report: &Report, interactive: bool) -> anyhow::Result<Brain> {
        let backend = build_backend(cfg, report)?;
        info!(backend = backend.label(), "selected inference backend");

        let confirm: Box<dyn ConfirmHook> =
            if interactive { Box::new(CliConfirm) } else { Box::new(DenyAll) };

        Ok(Brain {
            backend,
            session: Session::new(),
            confirm,
            budget: Budget::new(cfg.caps.max_tokens_per_session, cfg.caps.max_cost_usd),
            log: ActionLog::new(),
            max_steps: cfg.caps.max_steps,
            confirm_destructive: cfg.safety.confirm_destructive,
            observer: None,
        })
    }

    /// Override the confirmation hook (e.g. [`AllowAll`] for trusted automation).
    pub fn with_confirm(mut self, hook: Box<dyn ConfirmHook>) -> Self {
        self.confirm = hook;
        self
    }

    /// Allow tests / callers to inject a backend and session directly.
    pub fn with_backend(backend: Box<dyn LlmBackend>, session: Arc<Session>, cfg: &Config) -> Brain {
        Brain {
            backend,
            session,
            confirm: Box::new(AllowAll),
            budget: Budget::new(cfg.caps.max_tokens_per_session, cfg.caps.max_cost_usd),
            log: ActionLog::new(),
            max_steps: cfg.caps.max_steps,
            confirm_destructive: cfg.safety.confirm_destructive,
            observer: None,
        }
    }

    /// Attach a step observer (each reasoning/action step is reported as a JSON
    /// event). Used by `slug-agent --jsonl`.
    pub fn with_observer(mut self, observer: StepObserver) -> Self {
        self.observer = Some(observer);
        self
    }

    fn emit(&self, event: serde_json::Value) {
        if let Some(o) = &self.observer {
            o(event);
        }
    }

    /// The action log (for inspection / undo).
    pub fn log(&self) -> &ActionLog {
        &self.log
    }

    /// Run a task to completion.
    pub async fn run(&mut self, task: &str) -> std::result::Result<Outcome, BrainError> {
        let tools = tools::tool_specs();
        let mut messages: Vec<Msg> = vec![Msg::user_text(task)];
        let mut steps = 0u32;

        while steps < self.max_steps {
            if let Some(reason) = self.budget.exceeded() {
                warn!(%reason, "budget cap hit — escalating to human");
                return Ok(self.escalate(format!("Stopped: {reason}. Escalating to a human."), steps));
            }

            let turn = self.backend.complete_with_tools(SYSTEM_PROMPT, &messages, &tools).await?;
            steps += 1;
            let cost = estimate_cost(turn.usage, self.backend.as_ref());
            self.budget.record(turn.usage, cost);

            // Assemble the assistant message (preamble text + any tool_use blocks).
            let mut assistant: Vec<Content> = Vec::new();
            if let Some(text) = &turn.text {
                if !text.is_empty() {
                    assistant.push(Content::Text(text.clone()));
                }
            }
            for tc in &turn.tool_calls {
                assistant.push(Content::ToolUse {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    input: tc.input.clone(),
                });
            }

            if turn.is_final() {
                let answer = turn.text.unwrap_or_default();
                info!(steps, tokens = self.budget.used_tokens, "task complete");
                self.emit(json!({
                    "kind": "final", "step": steps, "answer": answer,
                    "tokens": self.budget.used_tokens, "cost_usd": self.budget.used_cost_usd,
                }));
                return Ok(Outcome {
                    answer,
                    steps,
                    tokens: self.budget.used_tokens,
                    cost_usd: self.budget.used_cost_usd,
                    escalated: false,
                });
            }

            messages.push(Msg { role: Role::Assistant, content: assistant });

            // Execute each requested tool, gating destructive invokes.
            let mut results: Vec<Content> = Vec::new();
            for tc in &turn.tool_calls {
                let reasoning =
                    tc.input.get("reasoning").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let result = self.run_tool_call(tc).await;
                if let Content::ToolResult { content, is_error, .. } = &result {
                    let snippet: String = content.chars().take(400).collect();
                    self.emit(json!({
                        "kind": "step",
                        "step": steps,
                        "reasoning": reasoning,
                        "tool": tc.name,
                        "args": tc.input,
                        "result": snippet,
                        "is_error": is_error,
                        "tokens": self.budget.used_tokens,
                        "cost_usd": self.budget.used_cost_usd,
                    }));
                }
                results.push(result);
            }
            messages.push(Msg { role: Role::User, content: results });
        }

        Ok(self.escalate("Reached the step limit without completing the task.".into(), steps))
    }

    /// Execute a single tool call (with the destructive gate + verify snapshot)
    /// and return its `tool_result` content block.
    async fn run_tool_call(&mut self, tc: &backend::ToolCall) -> Content {
        let reasoning = tc
            .input
            .get("reasoning")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // Destructive-action confirmation for slug_invoke. We lack the node label
        // at invoke time, so we judge intent from the action verb, its argument,
        // and the model's stated reasoning.
        if self.confirm_destructive && tc.name == "slug_invoke" {
            let action = tc.input.get("action").and_then(|v| v.as_str()).unwrap_or("");
            let args = tc.input.get("args").and_then(|v| v.as_str());
            let refname = tc.input.get("ref").and_then(|v| v.as_str()).unwrap_or("?");
            if crate::safety::is_destructive(action, Some(&reasoning), args) {
                let summary = format!(
                    "{action} on {refname}{}{}",
                    args.map(|a| format!(" \"{a}\"")).unwrap_or_default(),
                    if reasoning.is_empty() { String::new() } else { format!(" — {reasoning}") },
                );
                if !self.confirm.confirm(&summary) {
                    return Content::ToolResult {
                        tool_use_id: tc.id.clone(),
                        content: "Action cancelled: the user declined this destructive action.".into(),
                        is_error: true,
                    };
                }
            }
        }

        let out = tools::execute(&self.session, &tc.name, &tc.input).await;

        // Log the action (with a best-effort undo for invokes).
        let undo = if tc.name == "slug_invoke" {
            let r = tc.input.get("ref").and_then(|v| v.as_str()).unwrap_or("");
            let a = tc.input.get("action").and_then(|v| v.as_str()).unwrap_or("");
            ActionLog::infer_undo(r, a, None)
        } else {
            None
        };
        self.log.push(ActionRecord {
            tool: tc.name.clone(),
            args: tc.input.clone(),
            reasoning,
            result: out.text.clone(),
            is_error: out.is_error,
            undo,
        });

        // Verify: after a successful action, attach a fresh focused snapshot.
        let mut content = out.text;
        if tc.name == "slug_invoke" && !out.is_error {
            let verify = tools::execute(&self.session, "slug_snapshot", &json!({"scope":"focused"})).await;
            content.push_str("\n\n# post-action snapshot (verify expected vs actual):\n");
            content.push_str(&verify.text);
        }

        Content::ToolResult { tool_use_id: tc.id.clone(), content, is_error: out.is_error }
    }

    fn escalate(&self, answer: String, steps: u32) -> Outcome {
        Outcome {
            answer,
            steps,
            tokens: self.budget.used_tokens,
            cost_usd: self.budget.used_cost_usd,
            escalated: true,
        }
    }
}

/// Choose and construct the backend per `[brain] provider` (falling back to the
/// legacy `[backend] selection` / hardware tier when provider is `auto`).
fn build_backend(cfg: &Config, report: &Report) -> anyhow::Result<Box<dyn LlmBackend>> {
    use crate::backend::{ClaudeBackend, GeminiBackend, OllamaBackend, OpenAiCompatibleBackend};
    use crate::config::Provider;

    // Resolve `auto` to a concrete provider.
    let provider = match cfg.brain.provider {
        Provider::Auto => match cfg.backend.selection {
            Selection::Cloud => Provider::Claude,
            Selection::Local => Provider::Ollama,
            Selection::Auto => {
                if report.backend == BackendKind::Cloud {
                    Provider::Claude
                } else {
                    Provider::Ollama
                }
            }
        },
        p => p,
    };

    let model_or_report = |m: &str| if m.is_empty() { report.model.clone() } else { m.to_string() };
    let max_tokens = cfg.cloud.max_tokens;
    let p = cfg.resolved_provider(provider);

    match provider {
        Provider::Claude => {
            Ok(Box::new(ClaudeBackend::from_env(&p.api_key_env, model_or_report(&p.model), max_tokens)?))
        }
        Provider::Openai => Ok(Box::new(OpenAiCompatibleBackend::from_env(
            &p.base_url,
            &p.api_key_env,
            model_or_report(&p.model),
            max_tokens,
            "openai",
        )?)),
        Provider::Openrouter => Ok(Box::new(OpenAiCompatibleBackend::from_env(
            &p.base_url,
            &p.api_key_env,
            model_or_report(&p.model),
            max_tokens,
            "openrouter",
        )?)),
        Provider::Gemini => Ok(Box::new(GeminiBackend::from_env(
            &p.base_url,
            &p.api_key_env,
            model_or_report(&p.model),
            max_tokens,
        )?)),
        Provider::Ollama => {
            let host = if p.base_url.is_empty() { cfg.local.ollama_host.clone() } else { p.base_url };
            let model = if !p.model.is_empty() {
                p.model
            } else if !cfg.local.model.is_empty() {
                cfg.local.model.clone()
            } else {
                report.model.clone()
            };
            Ok(Box::new(OllamaBackend::new(host, model, cfg.local.num_ctx)))
        }
        Provider::Auto => unreachable!("auto resolved above"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{Result as BackendResult, ToolCall, ToolSpec, Turn, Usage};
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Mutex;

    /// A scripted backend: returns a queued sequence of turns, ignoring input.
    struct ScriptedBackend {
        turns: Mutex<std::collections::VecDeque<Turn>>,
    }

    impl LlmBackend for ScriptedBackend {
        fn complete_with_tools<'a>(
            &'a self,
            _system: &'a str,
            _messages: &'a [Msg],
            _tools: &'a [ToolSpec],
        ) -> Pin<Box<dyn Future<Output = BackendResult<Turn>> + Send + 'a>> {
            let next = self.turns.lock().unwrap().pop_front().unwrap_or_default();
            Box::pin(async move { Ok(next) })
        }
        fn label(&self) -> &str {
            "scripted"
        }
    }

    fn cfg() -> Config {
        Config::default()
    }

    #[tokio::test]
    async fn returns_final_answer_without_tools() {
        let turns = [Turn {
            text: Some("Hello, done.".into()),
            tool_calls: vec![],
            usage: Usage { input_tokens: 10, output_tokens: 2 },
        }];
        let backend = Box::new(ScriptedBackend {
            turns: Mutex::new(turns.into_iter().collect()),
        });
        let mut brain = Brain::with_backend(backend, Session::new(), &cfg());
        let out = brain.run("say hi").await.unwrap();
        assert!(!out.escalated);
        assert_eq!(out.answer, "Hello, done.");
        assert_eq!(out.steps, 1);
    }

    #[tokio::test]
    async fn executes_tool_then_finishes() {
        // Turn 1: call a tool (fails without a bus, but the loop must continue).
        // Turn 2: final answer.
        let t1 = Turn {
            text: Some("Listing apps.".into()),
            tool_calls: vec![ToolCall {
                id: "c1".into(),
                name: "slug_list_apps".into(),
                input: json!({}),
            }],
            usage: Usage { input_tokens: 50, output_tokens: 5 },
        };
        let t2 = Turn {
            text: Some("No apps available; stopping.".into()),
            tool_calls: vec![],
            usage: Usage { input_tokens: 60, output_tokens: 8 },
        };
        let backend = Box::new(ScriptedBackend {
            turns: Mutex::new([t1, t2].into_iter().collect()),
        });
        let mut brain = Brain::with_backend(backend, Session::new(), &cfg());
        let out = brain.run("list apps").await.unwrap();
        assert_eq!(out.steps, 2);
        assert_eq!(out.answer, "No apps available; stopping.");
        // The tool call was logged.
        assert_eq!(brain.log().entries().len(), 1);
        assert_eq!(brain.log().entries()[0].tool, "slug_list_apps");
    }

    #[tokio::test]
    async fn token_cap_escalates() {
        let mut c = cfg();
        c.caps.max_tokens_per_session = 5; // tiny cap
        c.caps.max_steps = 10;
        // First turn uses 100 tokens via a tool call; loop then re-checks cap.
        let t1 = Turn {
            text: None,
            tool_calls: vec![ToolCall { id: "c1".into(), name: "slug_list_apps".into(), input: json!({}) }],
            usage: Usage { input_tokens: 100, output_tokens: 0 },
        };
        let backend = Box::new(ScriptedBackend { turns: Mutex::new([t1].into_iter().collect()) });
        let mut brain = Brain::with_backend(backend, Session::new(), &c);
        let out = brain.run("do something big").await.unwrap();
        assert!(out.escalated);
        assert!(out.answer.contains("cap reached"));
    }

    #[tokio::test]
    async fn destructive_action_denied_by_hook() {
        let t1 = Turn {
            text: None,
            tool_calls: vec![ToolCall {
                id: "c1".into(),
                name: "slug_invoke".into(),
                input: json!({"ref":"b1","action":"click","reasoning":"delete the file"}),
            }],
            usage: Usage::default(),
        };
        let t2 = Turn { text: Some("Aborted.".into()), tool_calls: vec![], usage: Usage::default() };
        let backend = Box::new(ScriptedBackend { turns: Mutex::new([t1, t2].into_iter().collect()) });
        // confirm_destructive defaults true; DenyAll hook denies.
        let mut brain = Brain::with_backend(backend, Session::new(), &cfg())
            .with_confirm(Box::new(DenyAll));
        // Make the action itself destructive via args text.
        let out = brain.run("delete it").await.unwrap();
        assert_eq!(out.answer, "Aborted.");
        // The denied action is still logged? No — denial short-circuits before execute.
        // The loop recorded the tool_result as an error, but no ActionRecord push.
        assert!(brain.log().entries().is_empty());
    }
}
