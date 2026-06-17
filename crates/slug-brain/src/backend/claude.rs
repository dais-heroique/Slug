//! Anthropic Claude API backend (Messages API, raw HTTP).
//!
//! Rust has no official Anthropic SDK, so this calls `POST /v1/messages`
//! directly. It implements the documented tool-use turn: send the `tools` array,
//! and when the response `stop_reason` is `tool_use`, surface the `tool_use`
//! blocks so the brain can execute them and append `tool_result` blocks on the
//! next turn. Default model is `claude-sonnet-4-6` (the `cloud_model` from
//! `docs/HARDWARE-TIERING.md`); override in `slug.toml`.

use std::future::Future;
use std::pin::Pin;

use serde_json::{json, Value};

use super::{BackendError, Content, LlmBackend, Msg, Result, Role, ToolCall, ToolSpec, Turn, Usage};

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";

/// Claude API backend.
pub struct ClaudeBackend {
    client: reqwest::Client,
    api_key: String,
    model: String,
    max_tokens: u32,
    price_in: f64,
    price_out: f64,
}

impl ClaudeBackend {
    /// Construct from an explicit key. Prefer [`ClaudeBackend::from_env`].
    pub fn new(api_key: impl Into<String>, model: impl Into<String>, max_tokens: u32) -> Self {
        let model = model.into();
        let (price_in, price_out) = price_for(&model);
        ClaudeBackend {
            client: reqwest::Client::new(),
            api_key: api_key.into(),
            model,
            max_tokens,
            price_in,
            price_out,
        }
    }

    /// Construct, reading the API key from the named environment variable.
    pub fn from_env(api_key_env: &str, model: impl Into<String>, max_tokens: u32) -> Result<Self> {
        let key = std::env::var(api_key_env)
            .map_err(|_| BackendError::MissingApiKey(api_key_env.to_string()))?;
        Ok(Self::new(key, model, max_tokens))
    }

    async fn complete(
        &self,
        system: &str,
        messages: &[Msg],
        tools: &[ToolSpec],
    ) -> Result<Turn> {
        let body = json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "system": system,
            "messages": messages.iter().map(msg_to_json).collect::<Vec<_>>(),
            "tools": tools.iter().map(tool_to_json).collect::<Vec<_>>(),
        });

        let resp = self
            .client
            .post(API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| BackendError::Http { backend: "claude", source: e })?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| BackendError::Http { backend: "claude", source: e })?;
        if !status.is_success() {
            return Err(BackendError::Status { backend: "claude", status: status.as_u16(), body: text });
        }

        let v: Value = serde_json::from_str(&text)
            .map_err(|e| BackendError::Parse(format!("{e}: {text}")))?;
        parse_response(&v)
    }
}

impl LlmBackend for ClaudeBackend {
    fn complete_with_tools<'a>(
        &'a self,
        system: &'a str,
        messages: &'a [Msg],
        tools: &'a [ToolSpec],
    ) -> Pin<Box<dyn Future<Output = Result<Turn>> + Send + 'a>> {
        Box::pin(self.complete(system, messages, tools))
    }

    fn price_per_mtok(&self) -> (f64, f64) {
        (self.price_in, self.price_out)
    }

    fn label(&self) -> &str {
        &self.model
    }
}

/// Published per-1M-token prices (USD) for the cloud-capable models.
fn price_for(model: &str) -> (f64, f64) {
    match model {
        m if m.starts_with("claude-opus") => (5.0, 25.0),
        m if m.starts_with("claude-sonnet") => (3.0, 15.0),
        m if m.starts_with("claude-haiku") => (1.0, 5.0),
        _ => (3.0, 15.0),
    }
}

fn tool_to_json(t: &ToolSpec) -> Value {
    json!({ "name": t.name, "description": t.description, "input_schema": t.input_schema })
}

fn msg_to_json(m: &Msg) -> Value {
    let role = match m.role {
        Role::User => "user",
        Role::Assistant => "assistant",
    };
    let blocks: Vec<Value> = m
        .content
        .iter()
        .map(|c| match c {
            Content::Text(t) => json!({ "type": "text", "text": t }),
            Content::ToolUse { id, name, input } => {
                json!({ "type": "tool_use", "id": id, "name": name, "input": input })
            }
            Content::ToolResult { tool_use_id, content, is_error } => json!({
                "type": "tool_result",
                "tool_use_id": tool_use_id,
                "content": content,
                "is_error": is_error,
            }),
        })
        .collect();
    json!({ "role": role, "content": blocks })
}

fn parse_response(v: &Value) -> Result<Turn> {
    let mut turn = Turn::default();
    let mut text = String::new();

    let blocks = v
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| BackendError::Parse("missing content array".into()))?;
    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(t) = block.get("text").and_then(Value::as_str) {
                    text.push_str(t);
                }
            }
            Some("tool_use") => {
                let id = block.get("id").and_then(Value::as_str).unwrap_or("").to_string();
                let name = block.get("name").and_then(Value::as_str).unwrap_or("").to_string();
                let input = block.get("input").cloned().unwrap_or_else(|| json!({}));
                turn.tool_calls.push(ToolCall { id, name, input });
            }
            _ => {}
        }
    }
    if !text.is_empty() {
        turn.text = Some(text);
    }
    if let Some(u) = v.get("usage") {
        turn.usage = Usage {
            input_tokens: u.get("input_tokens").and_then(Value::as_u64).unwrap_or(0),
            output_tokens: u.get("output_tokens").and_then(Value::as_u64).unwrap_or(0),
        };
    }
    Ok(turn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tool_use_response() {
        let v = json!({
            "content": [
                { "type": "text", "text": "Let me click that." },
                { "type": "tool_use", "id": "toolu_1", "name": "slug_invoke",
                  "input": { "ref": "b1", "action": "click" } }
            ],
            "stop_reason": "tool_use",
            "usage": { "input_tokens": 1200, "output_tokens": 45 }
        });
        let turn = parse_response(&v).unwrap();
        assert_eq!(turn.text.as_deref(), Some("Let me click that."));
        assert_eq!(turn.tool_calls.len(), 1);
        assert_eq!(turn.tool_calls[0].name, "slug_invoke");
        assert_eq!(turn.tool_calls[0].input["ref"], "b1");
        assert_eq!(turn.usage.input_tokens, 1200);
        assert!(!turn.is_final());
    }

    #[test]
    fn parses_final_text_response() {
        let v = json!({
            "content": [ { "type": "text", "text": "Done." } ],
            "stop_reason": "end_turn",
            "usage": { "input_tokens": 800, "output_tokens": 3 }
        });
        let turn = parse_response(&v).unwrap();
        assert!(turn.is_final());
        assert_eq!(turn.text.as_deref(), Some("Done."));
    }

    #[test]
    fn serializes_tool_result_round_trip() {
        let m = Msg {
            role: Role::User,
            content: vec![Content::ToolResult {
                tool_use_id: "toolu_1".into(),
                content: "ok".into(),
                is_error: false,
            }],
        };
        let j = msg_to_json(&m);
        assert_eq!(j["role"], "user");
        assert_eq!(j["content"][0]["type"], "tool_result");
        assert_eq!(j["content"][0]["tool_use_id"], "toolu_1");
    }

    #[test]
    fn pricing_lookup() {
        assert_eq!(price_for("claude-sonnet-4-6"), (3.0, 15.0));
        assert_eq!(price_for("claude-opus-4-8"), (5.0, 25.0));
    }
}
