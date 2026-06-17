//! The LLM backend abstraction.
//!
//! [`LlmBackend`] hides the difference between the local Ollama model and the
//! Anthropic Claude API behind one method, [`LlmBackend::complete_with_tools`].
//! Both backends are driven with the *same* tool schemas ([`ToolSpec`]) and the
//! same provider-neutral conversation ([`Msg`]); each backend serializes them to
//! its own wire format.
//!
//! The trait returns a boxed future so it stays `dyn`-compatible (the brain holds
//! a `Box<dyn LlmBackend>`), avoiding an `async-trait` dependency.

pub mod claude;
pub mod ollama;

use std::future::Future;
use std::pin::Pin;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use claude::ClaudeBackend;
pub use ollama::OllamaBackend;

/// A tool definition presented to the model. `input_schema` is JSON Schema and is
/// shared verbatim by both backends (each wraps it in its own envelope).
#[derive(Clone, Debug)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// Conversation role.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

/// A single content block within a message.
#[derive(Clone, Debug)]
pub enum Content {
    /// Plain text.
    Text(String),
    /// An assistant tool invocation.
    ToolUse { id: String, name: String, input: Value },
    /// A result for a previously-requested tool call (sent in a user turn).
    ToolResult { tool_use_id: String, content: String, is_error: bool },
}

/// A provider-neutral message.
#[derive(Clone, Debug)]
pub struct Msg {
    pub role: Role,
    pub content: Vec<Content>,
}

impl Msg {
    pub fn user_text(text: impl Into<String>) -> Msg {
        Msg { role: Role::User, content: vec![Content::Text(text.into())] }
    }
}

/// A single tool call the model wants executed.
#[derive(Clone, Debug)]
pub struct ToolCall {
    /// Stable id used to correlate the result. (Ollama has no ids; we synthesize one.)
    pub id: String,
    pub name: String,
    pub input: Value,
}

/// Token usage for one completion, used by the safety cost cap.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// The model's response for one turn: optional preamble text plus any tool calls.
/// When `tool_calls` is empty the turn is final and `text` is the answer.
#[derive(Clone, Debug, Default)]
pub struct Turn {
    pub text: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Usage,
}

impl Turn {
    pub fn is_final(&self) -> bool {
        self.tool_calls.is_empty()
    }
}

/// Errors from a backend.
#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error("http error talking to {backend}: {source}")]
    Http { backend: &'static str, source: reqwest::Error },
    #[error("{backend} returned status {status}: {body}")]
    Status { backend: &'static str, status: u16, body: String },
    #[error("could not parse backend response: {0}")]
    Parse(String),
    #[error("missing API key (set {0})")]
    MissingApiKey(String),
}

pub type Result<T> = std::result::Result<T, BackendError>;

/// The unified backend interface.
pub trait LlmBackend: Send + Sync {
    /// Run one model turn with tools available. Returns the model's text and/or
    /// the tool calls it wants executed.
    fn complete_with_tools<'a>(
        &'a self,
        system: &'a str,
        messages: &'a [Msg],
        tools: &'a [ToolSpec],
    ) -> Pin<Box<dyn Future<Output = Result<Turn>> + Send + 'a>>;

    /// Per-1M-token (input, output) price in USD, for cost estimation. `(0.0, 0.0)`
    /// for local models.
    fn price_per_mtok(&self) -> (f64, f64) {
        (0.0, 0.0)
    }

    /// Human-readable label for logs.
    fn label(&self) -> &str;
}

/// Estimate the USD cost of a usage figure given a backend's pricing.
pub fn estimate_cost(usage: Usage, backend: &dyn LlmBackend) -> f64 {
    let (inp, out) = backend.price_per_mtok();
    (usage.input_tokens as f64 / 1_000_000.0) * inp
        + (usage.output_tokens as f64 / 1_000_000.0) * out
}
