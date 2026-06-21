//! OpenAI-compatible chat-completions backend.
//!
//! One implementation covers OpenAI (`https://api.openai.com/v1`), OpenRouter
//! (`https://openrouter.ai/api/v1`), and any local OpenAI-compatible server
//! (llama.cpp `server`, vLLM, LM Studio): only `base_url` + `api_key` + `model`
//! differ. It posts to `{base_url}/chat/completions` with the `tools` parameter
//! and parses `choices[0].message.tool_calls`.

use std::future::Future;
use std::pin::Pin;

use serde_json::{json, Value};

use super::{BackendError, Content, LlmBackend, Msg, Result, Role, ToolCall, ToolSpec, Turn, Usage};

/// An OpenAI-compatible backend (OpenAI / OpenRouter / local servers).
pub struct OpenAiCompatibleBackend {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
    max_tokens: u32,
    label: String,
}

impl OpenAiCompatibleBackend {
    /// Construct from an explicit key. `base_url` is the API root (without the
    /// trailing `/chat/completions`), e.g. `https://api.openai.com/v1`.
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
        max_tokens: u32,
        label: impl Into<String>,
    ) -> Self {
        OpenAiCompatibleBackend {
            client: reqwest::Client::new(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            model: model.into(),
            max_tokens,
            label: label.into(),
        }
    }

    /// Construct, reading the API key from the named environment variable. A local
    /// server with no auth can use an empty/missing key — pass an empty env name
    /// or set the var to any placeholder.
    pub fn from_env(
        base_url: impl Into<String>,
        api_key_env: &str,
        model: impl Into<String>,
        max_tokens: u32,
        label: impl Into<String>,
    ) -> Result<Self> {
        // Local servers often need no key; only error if a name was given and unset.
        let api_key = if api_key_env.is_empty() {
            String::new()
        } else {
            std::env::var(api_key_env).unwrap_or_default()
        };
        Ok(Self::new(base_url, api_key, model, max_tokens, label))
    }

    async fn complete(&self, system: &str, messages: &[Msg], tools: &[ToolSpec]) -> Result<Turn> {
        let url = format!("{}/chat/completions", self.base_url);
        let body = build_request(&self.model, self.max_tokens, system, messages, tools);

        let mut req = self.client.post(&url).json(&body);
        if !self.api_key.is_empty() {
            req = req.bearer_auth(&self.api_key);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| BackendError::Http { backend: "openai", source: e })?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| BackendError::Http { backend: "openai", source: e })?;
        if !status.is_success() {
            return Err(BackendError::Status { backend: "openai", status: status.as_u16(), body: text });
        }
        let v: Value = serde_json::from_str(&text).map_err(|e| BackendError::Parse(format!("{e}: {text}")))?;
        parse_response(&v)
    }
}

impl LlmBackend for OpenAiCompatibleBackend {
    fn complete_with_tools<'a>(
        &'a self,
        system: &'a str,
        messages: &'a [Msg],
        tools: &'a [ToolSpec],
    ) -> Pin<Box<dyn Future<Output = Result<Turn>> + Send + 'a>> {
        Box::pin(self.complete(system, messages, tools))
    }

    fn label(&self) -> &str {
        &self.label
    }
}

/// Build the chat-completions request body.
fn build_request(model: &str, max_tokens: u32, system: &str, messages: &[Msg], tools: &[ToolSpec]) -> Value {
    let mut out: Vec<Value> = Vec::new();
    if !system.is_empty() {
        out.push(json!({ "role": "system", "content": system }));
    }
    for m in messages {
        push_messages(m, &mut out);
    }
    json!({
        "model": model,
        "max_tokens": max_tokens,
        "messages": out,
        "tools": tools.iter().map(tool_to_json).collect::<Vec<_>>(),
        "tool_choice": "auto",
    })
}

/// Expand one neutral [`Msg`] into one or more OpenAI messages (tool results are
/// their own `role:"tool"` messages; assistant tool calls go in `tool_calls`).
fn push_messages(m: &Msg, out: &mut Vec<Value>) {
    match m.role {
        Role::User => {
            let mut text = String::new();
            let mut tool_results: Vec<Value> = Vec::new();
            for c in &m.content {
                match c {
                    Content::Text(t) => text.push_str(t),
                    Content::ToolResult { tool_use_id, content, .. } => {
                        tool_results.push(json!({
                            "role": "tool",
                            "tool_call_id": tool_use_id,
                            "content": content,
                        }));
                    }
                    Content::ToolUse { .. } => {}
                }
            }
            if !text.is_empty() {
                out.push(json!({ "role": "user", "content": text }));
            }
            out.extend(tool_results);
        }
        Role::Assistant => {
            let mut text = String::new();
            let mut tool_calls: Vec<Value> = Vec::new();
            for c in &m.content {
                match c {
                    Content::Text(t) => text.push_str(t),
                    Content::ToolUse { id, name, input } => {
                        tool_calls.push(json!({
                            "id": id,
                            "type": "function",
                            "function": { "name": name, "arguments": input.to_string() },
                        }));
                    }
                    Content::ToolResult { .. } => {}
                }
            }
            let mut msg = json!({ "role": "assistant" });
            msg["content"] = if text.is_empty() { Value::Null } else { Value::String(text) };
            if !tool_calls.is_empty() {
                msg["tool_calls"] = Value::Array(tool_calls);
            }
            out.push(msg);
        }
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

fn parse_response(v: &Value) -> Result<Turn> {
    let mut turn = Turn::default();
    let message = v
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|c| c.first())
        .and_then(|c| c.get("message"))
        .ok_or_else(|| BackendError::Parse("missing choices[0].message".into()))?;

    if let Some(t) = message.get("content").and_then(Value::as_str) {
        if !t.is_empty() {
            turn.text = Some(t.to_string());
        }
    }
    if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
        for call in calls {
            let id = call.get("id").and_then(Value::as_str).unwrap_or("").to_string();
            let func = call.get("function");
            let name = func
                .and_then(|f| f.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            // `arguments` is a JSON-encoded string.
            let input = func
                .and_then(|f| f.get("arguments"))
                .and_then(Value::as_str)
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or_else(|| json!({}));
            turn.tool_calls.push(ToolCall { id, name, input });
        }
    }
    if let Some(u) = v.get("usage") {
        turn.usage = Usage {
            input_tokens: u.get("prompt_tokens").and_then(Value::as_u64).unwrap_or(0),
            output_tokens: u.get("completion_tokens").and_then(Value::as_u64).unwrap_or(0),
        };
    }
    Ok(turn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_tools_and_messages() {
        let tools = vec![ToolSpec {
            name: "slug_invoke".into(),
            description: "act".into(),
            input_schema: json!({"type":"object"}),
        }];
        let msgs = vec![
            Msg::user_text("click save"),
            Msg {
                role: Role::Assistant,
                content: vec![Content::ToolUse {
                    id: "call_1".into(),
                    name: "slug_invoke".into(),
                    input: json!({"ref":"b1","action":"click"}),
                }],
            },
            Msg {
                role: Role::User,
                content: vec![Content::ToolResult {
                    tool_use_id: "call_1".into(),
                    content: "ok".into(),
                    is_error: false,
                }],
            },
        ];
        let body = build_request("gpt-4o", 1024, "be brief", &msgs, &tools);
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["function"]["name"], "slug_invoke");
        let m = body["messages"].as_array().unwrap();
        assert_eq!(m[0]["role"], "system");
        assert_eq!(m[1]["role"], "user");
        assert_eq!(m[2]["role"], "assistant");
        assert_eq!(m[2]["tool_calls"][0]["function"]["name"], "slug_invoke");
        // arguments must be a JSON-encoded *string*.
        assert!(m[2]["tool_calls"][0]["function"]["arguments"].is_string());
        assert_eq!(m[3]["role"], "tool");
        assert_eq!(m[3]["tool_call_id"], "call_1");
    }

    #[test]
    fn parses_tool_call_response() {
        let v = json!({
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_abc",
                        "type": "function",
                        "function": { "name": "slug_invoke", "arguments": "{\"ref\":\"b1\",\"action\":\"click\"}" }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": { "prompt_tokens": 321, "completion_tokens": 12 }
        });
        let turn = parse_response(&v).unwrap();
        assert_eq!(turn.tool_calls.len(), 1);
        assert_eq!(turn.tool_calls[0].name, "slug_invoke");
        assert_eq!(turn.tool_calls[0].input["ref"], "b1");
        assert_eq!(turn.usage.input_tokens, 321);
        assert!(!turn.is_final());
    }

    #[test]
    fn parses_final_text_response() {
        let v = json!({
            "choices": [{ "message": { "role": "assistant", "content": "All done." }, "finish_reason": "stop" }],
            "usage": { "prompt_tokens": 50, "completion_tokens": 4 }
        });
        let turn = parse_response(&v).unwrap();
        assert!(turn.is_final());
        assert_eq!(turn.text.as_deref(), Some("All done."));
    }
}
