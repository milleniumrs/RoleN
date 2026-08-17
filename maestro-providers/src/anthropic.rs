//! Anthropic Messages API (FR-1.1).

use crate::chat::{ChatRequest, ChatResponse};
use crate::error::ProviderError;
use maestro_core::types::Model;
use serde_json::{json, Value};

pub const DEFAULT_BASE: &str = "https://api.anthropic.com";
const ANTHROPIC_VERSION: &str = "2023-06-01";
/// Required when calling with subscription OAuth tokens instead of API keys.
const OAUTH_BETA: &str = "oauth-2025-04-20";

/// How to authenticate against the Messages API.
#[derive(Debug, Clone)]
pub enum Auth {
    ApiKey(String),
    OAuth(String),
}

fn apply_auth(
    req: reqwest::blocking::RequestBuilder,
    auth: Option<Auth>,
) -> reqwest::blocking::RequestBuilder {
    match auth {
        Some(Auth::ApiKey(k)) => req.header("x-api-key", k),
        Some(Auth::OAuth(t)) => req.bearer_auth(t).header("anthropic-beta", OAUTH_BETA),
        None => req,
    }
}

pub fn chat(
    http: &reqwest::blocking::Client,
    base: &str,
    auth: Option<Auth>,
    req: &ChatRequest,
) -> Result<ChatResponse, ProviderError> {
    let url = format!("{}/v1/messages", base.trim_end_matches('/'));
    let body = json!({
        "model": req.model,
        "max_tokens": req.max_tokens.unwrap_or(256),
        "messages": req.messages,
    });
    let call = apply_auth(
        http.post(&url)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&body),
        auth,
    );
    let started = std::time::Instant::now();
    let resp = call.send()?;
    let latency_ms = started.elapsed().as_millis() as u64;
    let status = resp.status();
    let text = resp.text()?;
    if !status.is_success() {
        return Err(ProviderError::Api(format!(
            "HTTP {status}: {}",
            truncate(&text)
        )));
    }
    let json: Value = serde_json::from_str(&text)
        .map_err(|e| ProviderError::Parse(format!("invalid JSON: {e}")))?;
    let mut out = parse_message(&json)?;
    out.latency_ms = latency_ms;
    Ok(out)
}

pub fn list_models(
    http: &reqwest::blocking::Client,
    base: &str,
    auth: Option<Auth>,
) -> Result<Vec<Model>, ProviderError> {
    let url = format!("{}/v1/models", base.trim_end_matches('/'));
    let call = apply_auth(
        http.get(&url)
            .header("anthropic-version", ANTHROPIC_VERSION),
        auth,
    );
    let resp = call.send()?;
    let status = resp.status();
    let text = resp.text()?;
    if !status.is_success() {
        return Err(ProviderError::Api(format!(
            "HTTP {status}: {}",
            truncate(&text)
        )));
    }
    let json: Value = serde_json::from_str(&text)
        .map_err(|e| ProviderError::Parse(format!("invalid JSON: {e}")))?;
    Ok(parse_models(&json))
}

/// Pure parser — concatenates all text blocks (thinking blocks ignored).
pub fn parse_message(json: &Value) -> Result<ChatResponse, ProviderError> {
    let blocks = json["content"]
        .as_array()
        .ok_or_else(|| ProviderError::Parse("missing content[]".into()))?;
    let text: String = blocks
        .iter()
        .filter(|b| b["type"].as_str() == Some("text"))
        .filter_map(|b| b["text"].as_str())
        .collect();
    if text.is_empty() {
        return Err(ProviderError::Parse("no text block in content[]".into()));
    }
    let tokens_in = json["usage"]["input_tokens"].as_u64().unwrap_or(0);
    let tokens_out = json["usage"]["output_tokens"].as_u64().unwrap_or(0);
    Ok(ChatResponse {
        text,
        tokens_in,
        tokens_out,
        latency_ms: 0,
    })
}

/// Pure parser — unit-tested without network.
pub fn parse_models(json: &Value) -> Vec<Model> {
    json["data"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m["id"].as_str())
                .map(|id| Model {
                    id: id.to_string(),
                    ..Default::default()
                })
                .collect()
        })
        .unwrap_or_default()
}

fn truncate(s: &str) -> String {
    s.chars().take(300).collect()
}

// ------------------------------------------------------------- tool calling

use crate::chat::{HistMsg, StopKind, ToolCall, ToolsChatRequest, ToolsChatResponse};

pub fn chat_tools(
    http: &reqwest::blocking::Client,
    base: &str,
    auth: Option<Auth>,
    req: &ToolsChatRequest,
) -> Result<ToolsChatResponse, ProviderError> {
    let url = format!("{}/v1/messages", base.trim_end_matches('/'));
    let body = build_tools_body(req);
    let call = apply_auth(
        http.post(&url)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&body),
        auth,
    );
    let started = std::time::Instant::now();
    let resp = call.send()?;
    let latency_ms = started.elapsed().as_millis() as u64;
    let status = resp.status();
    let text = resp.text()?;
    if !status.is_success() {
        return Err(ProviderError::Api(format!(
            "HTTP {status}: {}",
            truncate(&text)
        )));
    }
    let json: Value = serde_json::from_str(&text)
        .map_err(|e| ProviderError::Parse(format!("invalid JSON: {e}")))?;
    let mut out = parse_tools_response(&json)?;
    out.latency_ms = latency_ms;
    Ok(out)
}

/// Pure builder — Anthropic wire format (system is top-level; tool results
/// are tool_result blocks inside a user message).
pub fn build_tools_body(req: &ToolsChatRequest) -> Value {
    let mut system_parts: Vec<String> = Vec::new();
    let mut messages: Vec<Value> = Vec::new();
    for m in &req.history {
        match m {
            HistMsg::System(s) => system_parts.push(s.clone()),
            HistMsg::User(s) => messages.push(json!({"role": "user", "content": s})),
            HistMsg::Assistant { text, tool_calls } => {
                let mut blocks: Vec<Value> = Vec::new();
                if !text.is_empty() {
                    blocks.push(json!({"type": "text", "text": text}));
                }
                for c in tool_calls {
                    blocks.push(json!({
                        "type": "tool_use", "id": c.id, "name": c.name, "input": c.args
                    }));
                }
                if blocks.is_empty() {
                    blocks.push(json!({"type": "text", "text": ""}));
                }
                messages.push(json!({"role": "assistant", "content": blocks}));
            }
            HistMsg::ToolResults(results) => {
                let blocks: Vec<Value> = results
                    .iter()
                    .map(|r| {
                        json!({
                            "type": "tool_result",
                            "tool_use_id": r.id,
                            "content": r.content,
                            "is_error": r.is_error
                        })
                    })
                    .collect();
                messages.push(json!({"role": "user", "content": blocks}));
            }
        }
    }
    let tools: Vec<Value> = req
        .tools
        .iter()
        .map(
            |t| json!({"name": t.name, "description": t.description, "input_schema": t.parameters}),
        )
        .collect();
    let mut body = json!({
        "model": req.model,
        "max_tokens": req.max_tokens.unwrap_or(4096),
        "messages": messages,
        "tools": tools,
    });
    if !system_parts.is_empty() {
        body["system"] = json!(system_parts.join("\n\n"));
    }
    body
}

/// Pure parser — unit-tested without network.
pub fn parse_tools_response(json: &Value) -> Result<ToolsChatResponse, ProviderError> {
    let blocks = json["content"]
        .as_array()
        .ok_or_else(|| ProviderError::Parse("missing content[]".into()))?;
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    for b in blocks {
        match b["type"].as_str() {
            Some("text") => text.push_str(b["text"].as_str().unwrap_or("")),
            Some("tool_use") => {
                if let Some(name) = b["name"].as_str() {
                    tool_calls.push(ToolCall {
                        id: b["id"].as_str().unwrap_or("").to_string(),
                        name: name.to_string(),
                        args: b["input"].clone(),
                    });
                }
            }
            _ => {}
        }
    }
    let stop = match json["stop_reason"].as_str() {
        Some("end_turn") => StopKind::Stop,
        Some("tool_use") => StopKind::ToolUse,
        Some("max_tokens") => StopKind::Length,
        _ => {
            if tool_calls.is_empty() {
                StopKind::Other
            } else {
                StopKind::ToolUse
            }
        }
    };
    Ok(ToolsChatResponse {
        text,
        tool_calls,
        tokens_in: json["usage"]["input_tokens"].as_u64().unwrap_or(0),
        tokens_out: json["usage"]["output_tokens"].as_u64().unwrap_or(0),
        latency_ms: 0,
        stop,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_message_with_thinking_block() {
        let json = json!({
            "content": [
                {"type": "thinking", "thinking": "hmm"},
                {"type": "text", "text": "OK"}
            ],
            "usage": {"input_tokens": 20, "output_tokens": 4}
        });
        let r = parse_message(&json).unwrap();
        assert_eq!(r.text, "OK");
        assert_eq!(r.tokens_in, 20);
        assert_eq!(r.tokens_out, 4);
    }

    #[test]
    fn parses_models() {
        let json = json!({"data": [{"id": "claude-opus-5"}, {"id": "claude-sonnet-5"}]});
        assert_eq!(parse_models(&json).len(), 2);
    }

    #[test]
    fn builds_and_parses_tool_use() {
        let req = ToolsChatRequest {
            model: "claude".into(),
            system: Some("sys".into()),
            history: vec![
                HistMsg::System("sys".into()),
                HistMsg::User("hi".into()),
                HistMsg::Assistant {
                    text: "checking".into(),
                    tool_calls: vec![ToolCall {
                        id: "t1".into(),
                        name: "search".into(),
                        args: json!({"q": "x"}),
                    }],
                },
                HistMsg::ToolResults(vec![crate::chat::ToolOutcome {
                    id: "t1".into(),
                    name: "search".into(),
                    content: "found".into(),
                    is_error: false,
                }]),
            ],
            tools: vec![crate::chat::ToolSpec {
                name: "search".into(),
                description: "d".into(),
                parameters: json!({"type": "object"}),
            }],
            max_tokens: None,
        };
        let body = build_tools_body(&req);
        assert_eq!(body["system"], "sys");
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs[1]["role"], "assistant");
        assert_eq!(msgs[1]["content"][1]["type"], "tool_use");
        assert_eq!(msgs[2]["role"], "user");
        assert_eq!(msgs[2]["content"][0]["type"], "tool_result");
        assert_eq!(body["tools"][0]["input_schema"]["type"], "object");

        let resp = json!({
            "content": [
                {"type": "text", "text": "let me check"},
                {"type": "tool_use", "id": "t1", "name": "search", "input": {"q": "abc"}}
            ],
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 7, "output_tokens": 9}
        });
        let r = parse_tools_response(&resp).unwrap();
        assert_eq!(r.text, "let me check");
        assert_eq!(r.stop, StopKind::ToolUse);
        assert_eq!(r.tool_calls[0].name, "search");
        assert_eq!(r.tokens_out, 9);
    }
}
