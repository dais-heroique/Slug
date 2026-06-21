//! Google Gemini backend (`generateContent`).
//!
//! Maps Slug's neutral tool schemas to Gemini's `function_declarations` and
//! parses `functionCall` parts from the response. Endpoint:
//! `{base_url}/v1beta/models/{model}:generateContent?key=API_KEY`.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use serde_json::{json, Value};

use super::{BackendError, Content, LlmBackend, Msg, Result, Role, ToolCall, ToolSpec, Turn, Usage};

const DEFAULT_BASE: &str = "https://generativelanguage.googleapis.com";

/// Google Gemini backend.
pub struct GeminiBackend {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
    max_tokens: u32,
}

impl GeminiBackend {
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
        max_tokens: u32,
    ) -> Self {
        let base = base_url.into();
        let base = if base.is_empty() { DEFAULT_BASE.to_string() } else { base.trim_end_matches('/').to_string() };
        GeminiBackend {
            client: reqwest::Client::new(),
            base_url: base,
            api_key: api_key.into(),
            model: model.into(),
            max_tokens,
        }
    }

    pub fn from_env(
        base_url: impl Into<String>,
        api_key_env: &str,
        model: impl Into<String>,
        max_tokens: u32,
    ) -> Result<Self> {
        let key = std::env::var(api_key_env)
            .map_err(|_| BackendError::MissingApiKey(api_key_env.to_string()))?;
        Ok(Self::new(base_url, key, model, max_tokens))
    }

    async fn complete(&self, system: &str, messages: &[Msg], tools: &[ToolSpec]) -> Result<Turn> {
        let url = format!(
            "{}/v1beta/models/{}:generateContent?key={}",
            self.base_url, self.model, self.api_key
        );
        let body = build_request(self.max_tokens, system, messages, tools);
        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| BackendError::Http { backend: "gemini", source: e })?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| BackendError::Http { backend: "gemini", source: e })?;
        if !status.is_success() {
            return Err(BackendError::Status { backend: "gemini", status: status.as_u16(), body: text });
        }
        let v: Value = serde_json::from_str(&text).map_err(|e| BackendError::Parse(format!("{e}: {text}")))?;
        parse_response(&v)
    }
}

impl LlmBackend for GeminiBackend {
    fn complete_with_tools<'a>(
        &'a self,
        system: &'a str,
        messages: &'a [Msg],
        tools: &'a [ToolSpec],
    ) -> Pin<Box<dyn Future<Output = Result<Turn>> + Send + 'a>> {
        Box::pin(self.complete(system, messages, tools))
    }

    fn label(&self) -> &str {
        &self.model
    }
}

/// Build the `generateContent` request body.
fn build_request(max_tokens: u32, system: &str, messages: &[Msg], tools: &[ToolSpec]) -> Value {
    let mut body = json!({
        "contents": to_contents(messages),
        "generationConfig": { "maxOutputTokens": max_tokens },
    });
    if !system.is_empty() {
        body["systemInstruction"] = json!({ "parts": [ { "text": system } ] });
    }
    if !tools.is_empty() {
        body["tools"] = json!([{
            "function_declarations": tools.iter().map(tool_to_json).collect::<Vec<_>>()
        }]);
    }
    body
}

fn tool_to_json(t: &ToolSpec) -> Value {
    json!({ "name": t.name, "description": t.description, "parameters": t.input_schema })
}

/// Convert neutral messages to Gemini `contents`. Gemini's `functionResponse`
/// needs the function *name*, which our `ToolResult` only references by id, so we
/// track id→name from `tool_use` parts in a forward pass.
fn to_contents(messages: &[Msg]) -> Vec<Value> {
    let mut id_to_name: HashMap<String, String> = HashMap::new();
    let mut contents: Vec<Value> = Vec::new();

    for m in messages {
        let role = match m.role {
            Role::User => "user",
            Role::Assistant => "model",
        };
        let mut parts: Vec<Value> = Vec::new();
        for c in &m.content {
            match c {
                Content::Text(t) => parts.push(json!({ "text": t })),
                Content::ToolUse { id, name, input } => {
                    id_to_name.insert(id.clone(), name.clone());
                    parts.push(json!({ "functionCall": { "name": name, "args": input } }));
                }
                Content::ToolResult { tool_use_id, content, .. } => {
                    let name = id_to_name.get(tool_use_id).cloned().unwrap_or_else(|| tool_use_id.clone());
                    parts.push(json!({
                        "functionResponse": {
                            "name": name,
                            "response": { "result": content }
                        }
                    }));
                }
            }
        }
        if !parts.is_empty() {
            contents.push(json!({ "role": role, "parts": parts }));
        }
    }
    contents
}

fn parse_response(v: &Value) -> Result<Turn> {
    let mut turn = Turn::default();
    let mut text = String::new();

    let parts = v
        .get("candidates")
        .and_then(Value::as_array)
        .and_then(|c| c.first())
        .and_then(|c| c.get("content"))
        .and_then(|c| c.get("parts"))
        .and_then(Value::as_array);

    if let Some(parts) = parts {
        for (i, part) in parts.iter().enumerate() {
            if let Some(t) = part.get("text").and_then(Value::as_str) {
                text.push_str(t);
            }
            if let Some(fc) = part.get("functionCall") {
                let name = fc.get("name").and_then(Value::as_str).unwrap_or("").to_string();
                let input = fc.get("args").cloned().unwrap_or_else(|| json!({}));
                // Gemini gives no call id; synthesize a stable one.
                turn.tool_calls.push(ToolCall { id: format!("gemini-{i}-{name}"), name, input });
            }
        }
    }
    if !text.is_empty() {
        turn.text = Some(text);
    }
    if let Some(u) = v.get("usageMetadata") {
        turn.usage = Usage {
            input_tokens: u.get("promptTokenCount").and_then(Value::as_u64).unwrap_or(0),
            output_tokens: u.get("candidatesTokenCount").and_then(Value::as_u64).unwrap_or(0),
        };
    }
    Ok(turn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_tools_to_function_declarations() {
        let tools = vec![ToolSpec {
            name: "slug_snapshot".into(),
            description: "read ui".into(),
            input_schema: json!({"type":"object","properties":{}}),
        }];
        let body = build_request(2048, "be brief", &[Msg::user_text("hi")], &tools);
        assert_eq!(body["tools"][0]["function_declarations"][0]["name"], "slug_snapshot");
        assert_eq!(body["systemInstruction"]["parts"][0]["text"], "be brief");
        assert_eq!(body["contents"][0]["role"], "user");
        assert_eq!(body["contents"][0]["parts"][0]["text"], "hi");
    }

    #[test]
    fn round_trips_tool_call_and_result_names() {
        // assistant emits a functionCall (id X → name), then a tool result by id.
        let msgs = vec![
            Msg {
                role: Role::Assistant,
                content: vec![Content::ToolUse {
                    id: "x1".into(),
                    name: "slug_invoke".into(),
                    input: json!({"ref":"b1"}),
                }],
            },
            Msg {
                role: Role::User,
                content: vec![Content::ToolResult {
                    tool_use_id: "x1".into(),
                    content: "ok".into(),
                    is_error: false,
                }],
            },
        ];
        let contents = to_contents(&msgs);
        assert_eq!(contents[0]["role"], "model");
        assert_eq!(contents[0]["parts"][0]["functionCall"]["name"], "slug_invoke");
        // The functionResponse recovers the name from the id.
        assert_eq!(contents[1]["parts"][0]["functionResponse"]["name"], "slug_invoke");
    }

    #[test]
    fn parses_function_call_part() {
        let v = json!({
            "candidates": [{
                "content": { "role": "model", "parts": [
                    { "text": "clicking" },
                    { "functionCall": { "name": "slug_invoke", "args": { "ref": "b1", "action": "click" } } }
                ]}
            }],
            "usageMetadata": { "promptTokenCount": 200, "candidatesTokenCount": 8 }
        });
        let turn = parse_response(&v).unwrap();
        assert_eq!(turn.text.as_deref(), Some("clicking"));
        assert_eq!(turn.tool_calls.len(), 1);
        assert_eq!(turn.tool_calls[0].name, "slug_invoke");
        assert_eq!(turn.tool_calls[0].input["action"], "click");
        assert_eq!(turn.usage.input_tokens, 200);
    }
}
