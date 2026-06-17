//! Local Ollama backend (`POST /api/chat`, function-calling).
//!
//! Sends the same tool schemas as the Claude backend, wrapped in Ollama's
//! `{type:"function", function:{...}}` envelope. Ollama tool calls carry no ids,
//! so we synthesize stable ones for correlation; tool results are sent back as
//! `role:"tool"` messages.

use std::future::Future;
use std::pin::Pin;

use serde_json::{json, Value};

use super::{BackendError, Content, LlmBackend, Msg, Result, Role, ToolCall, ToolSpec, Turn, Usage};

/// Local Ollama backend.
pub struct OllamaBackend {
    client: reqwest::Client,
    host: String,
    model: String,
    num_ctx: u32,
}

impl OllamaBackend {
    /// `host` like `http://127.0.0.1:11434`.
    pub fn new(host: impl Into<String>, model: impl Into<String>, num_ctx: u32) -> Self {
        OllamaBackend {
            client: reqwest::Client::new(),
            host: host.into(),
            model: model.into(),
            num_ctx,
        }
    }

    async fn complete(
        &self,
        system: &str,
        messages: &[Msg],
        tools: &[ToolSpec],
    ) -> Result<Turn> {
        let mut wire: Vec<Value> = Vec::with_capacity(messages.len() + 1);
        if !system.is_empty() {
            wire.push(json!({ "role": "system", "content": system }));
        }
        for m in messages {
            wire.extend(msg_to_ollama(m));
        }

        let body = json!({
            "model": self.model,
            "messages": wire,
            "tools": tools.iter().map(tool_to_json).collect::<Vec<_>>(),
            "stream": false,
            "options": { "num_ctx": self.num_ctx },
        });

        let url = format!("{}/api/chat", self.host.trim_end_matches('/'));
        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| BackendError::Http { backend: "ollama", source: e })?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| BackendError::Http { backend: "ollama", source: e })?;
        if !status.is_success() {
            return Err(BackendError::Status { backend: "ollama", status: status.as_u16(), body: text });
        }

        let v: Value = serde_json::from_str(&text)
            .map_err(|e| BackendError::Parse(format!("{e}: {text}")))?;
        parse_response(&v)
    }
}

impl LlmBackend for OllamaBackend {
    fn complete_with_tools<'a>(
        &'a self,
        system: &'a str,
        messages: &'a [Msg],
        tools: &'a [ToolSpec],
    ) -> Pin<Box<dyn Future<Output = Result<Turn>> + Send + 'a>> {
        Box::pin(self.complete(system, messages, tools))
    }

    // Local inference is free; default (0.0, 0.0) pricing applies.

    fn label(&self) -> &str {
        &self.model
    }
}

fn tool_to_json(t: &ToolSpec) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": t.name,
            "description": t.description,
            "parameters": t.input_schema,
        }
    })
}

/// Convert one neutral message into one or more Ollama messages.
fn msg_to_ollama(m: &Msg) -> Vec<Value> {
    match m.role {
        Role::Assistant => {
            let mut text = String::new();
            let mut tool_calls: Vec<Value> = Vec::new();
            for c in &m.content {
                match c {
                    Content::Text(t) => text.push_str(t),
                    Content::ToolUse { name, input, .. } => {
                        tool_calls.push(json!({ "function": { "name": name, "arguments": input } }));
                    }
                    Content::ToolResult { .. } => {}
                }
            }
            let mut msg = json!({ "role": "assistant", "content": text });
            if !tool_calls.is_empty() {
                msg["tool_calls"] = Value::Array(tool_calls);
            }
            vec![msg]
        }
        Role::User => {
            let mut out = Vec::new();
            let mut text = String::new();
            for c in &m.content {
                match c {
                    Content::Text(t) => text.push_str(t),
                    Content::ToolResult { content, .. } => {
                        // Ollama matches tool results positionally / by name.
                        out.push(json!({ "role": "tool", "content": content }));
                    }
                    Content::ToolUse { .. } => {}
                }
            }
            if !text.is_empty() {
                out.insert(0, json!({ "role": "user", "content": text }));
            }
            out
        }
    }
}

fn parse_response(v: &Value) -> Result<Turn> {
    let msg = v
        .get("message")
        .ok_or_else(|| BackendError::Parse("missing message".into()))?;
    let mut turn = Turn::default();

    if let Some(t) = msg.get("content").and_then(Value::as_str) {
        if !t.is_empty() {
            turn.text = Some(t.to_string());
        }
    }
    if let Some(calls) = msg.get("tool_calls").and_then(Value::as_array) {
        for (i, call) in calls.iter().enumerate() {
            let func = call.get("function");
            let name = func
                .and_then(|f| f.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let input = func
                .and_then(|f| f.get("arguments"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            turn.tool_calls.push(ToolCall { id: format!("ollama-{i}-{name}"), name, input });
        }
    }
    turn.usage = Usage {
        input_tokens: v.get("prompt_eval_count").and_then(Value::as_u64).unwrap_or(0),
        output_tokens: v.get("eval_count").and_then(Value::as_u64).unwrap_or(0),
    };
    Ok(turn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tool_call() {
        let v = json!({
            "message": {
                "role": "assistant",
                "content": "",
                "tool_calls": [
                    { "function": { "name": "slug_snapshot", "arguments": { "scope": "window" } } }
                ]
            },
            "prompt_eval_count": 500,
            "eval_count": 20,
            "done": true
        });
        let turn = parse_response(&v).unwrap();
        assert_eq!(turn.tool_calls.len(), 1);
        assert_eq!(turn.tool_calls[0].name, "slug_snapshot");
        assert_eq!(turn.tool_calls[0].input["scope"], "window");
        assert!(!turn.is_final());
        assert_eq!(turn.usage.input_tokens, 500);
    }

    #[test]
    fn parses_final_text() {
        let v = json!({
            "message": { "role": "assistant", "content": "All set." },
            "done": true
        });
        let turn = parse_response(&v).unwrap();
        assert!(turn.is_final());
        assert_eq!(turn.text.as_deref(), Some("All set."));
    }

    #[test]
    fn tool_schema_is_wrapped_as_function() {
        let spec = ToolSpec {
            name: "slug_invoke".into(),
            description: "do".into(),
            input_schema: json!({ "type": "object" }),
        };
        let j = tool_to_json(&spec);
        assert_eq!(j["type"], "function");
        assert_eq!(j["function"]["name"], "slug_invoke");
    }
}
