//! OpenAI-compatible API (also covers OpenRouter, Kimi, GLM, … — FR-1.1).

use crate::chat::{ChatRequest, ChatResponse};
use crate::error::ProviderError;
use rolen_core::types::Model;
use serde_json::{json, Value};

pub const DEFAULT_BASE: &str = "https://api.openai.com/v1";

pub fn chat(
    http: &reqwest::blocking::Client,
    base: &str,
    api_key: Option<&str>,
    req: &ChatRequest,
) -> Result<ChatResponse, ProviderError> {
    let url = format!("{}/chat/completions", base.trim_end_matches('/'));
    let body = json!({
        "model": req.model,
        "messages": req.messages,
        "max_tokens": req.max_tokens.unwrap_or(256),
    });
    let mut call = http.post(&url).json(&body);
    if let Some(key) = api_key {
        call = call.bearer_auth(key);
    }
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
    let mut out = parse_chat_response(&json)?;
    out.latency_ms = latency_ms;
    Ok(out)
}

pub fn list_models(
    http: &reqwest::blocking::Client,
    base: &str,
    api_key: Option<&str>,
) -> Result<Vec<Model>, ProviderError> {
    let url = format!("{}/models", base.trim_end_matches('/'));
    let mut call = http.get(&url);
    if let Some(key) = api_key {
        call = call.bearer_auth(key);
    }
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

/// Pure parser — unit-tested without network.
pub fn parse_chat_response(json: &Value) -> Result<ChatResponse, ProviderError> {
    let text = json["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| ProviderError::Parse("missing choices[0].message.content".into()))?
        .to_string();
    let tokens_in = json["usage"]["prompt_tokens"].as_u64().unwrap_or(0);
    let tokens_out = json["usage"]["completion_tokens"].as_u64().unwrap_or(0);
    Ok(ChatResponse {
        text,
        tokens_in,
        tokens_cached: cached_prompt_tokens(json),
        tokens_out,
        latency_ms: 0,
    })
}

/// Cache hits inside `usage.prompt_tokens`.
///
/// `prompt_tokens` already counts cached tokens, so this is a subset, not an
/// extra. Providers that do not cache, or do not report it, leave the field
/// out and get 0. Kimi, DeepSeek and OpenAI all use this shape.
pub fn cached_prompt_tokens(json: &Value) -> u64 {
    let details = &json["usage"]["prompt_tokens_details"];
    let cached = details["cached_tokens"]
        .as_u64()
        // DeepSeek reports the same number under its own name.
        .or_else(|| json["usage"]["prompt_cache_hit_tokens"].as_u64())
        .unwrap_or(0);
    // Never claim more cache hits than there were prompt tokens: the cost
    // split subtracts this and a bad number would bill negative fresh input.
    cached.min(json["usage"]["prompt_tokens"].as_u64().unwrap_or(0))
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
    api_key: Option<&str>,
    req: &ToolsChatRequest,
) -> Result<ToolsChatResponse, ProviderError> {
    let url = format!("{}/chat/completions", base.trim_end_matches('/'));
    let body = build_tools_body(req);
    let mut call = http.post(&url).json(&body);
    if let Some(key) = api_key {
        call = call.bearer_auth(key);
    }
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

/// Wire-format flavor for tool calling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolWireStyle {
    /// arguments as JSON string; tool messages keyed by tool_call_id.
    OpenAI,
    /// arguments as JSON object; tool messages carry the tool name.
    Ollama,
}

/// Pure builder — unit-tested without network.
pub fn build_tools_body(req: &ToolsChatRequest) -> Value {
    build_tools_body_styled(req, ToolWireStyle::OpenAI)
}

pub(crate) fn build_tools_body_styled(req: &ToolsChatRequest, style: ToolWireStyle) -> Value {
    let mut messages: Vec<Value> = Vec::new();
    for m in &req.history {
        match m {
            HistMsg::System(s) => messages.push(json!({"role": "system", "content": s})),
            HistMsg::User(s) => messages.push(json!({"role": "user", "content": s})),
            HistMsg::Assistant { text, tool_calls } => {
                let calls: Vec<Value> = tool_calls
                    .iter()
                    .map(|c| {
                        let arguments = match style {
                            ToolWireStyle::OpenAI => json!(c.args.to_string()),
                            ToolWireStyle::Ollama => c.args.clone(),
                        };
                        json!({
                            "id": c.id,
                            "type": "function",
                            "function": {"name": c.name, "arguments": arguments}
                        })
                    })
                    .collect();
                if calls.is_empty() {
                    // some providers (kimi) reject empty assistant content
                    messages.push(json!({"role": "assistant", "content": if text.is_empty() { "…" } else { text.as_str() }}));
                } else {
                    messages.push(json!({
                        "role": "assistant",
                        "content": if text.is_empty() { Value::Null } else { json!(text) },
                        "tool_calls": calls
                    }));
                }
            }
            HistMsg::ToolResults(results) => {
                for r in results {
                    let mut msg = json!({
                        "role": "tool",
                        "tool_call_id": r.id,
                        "content": r.content
                    });
                    if style == ToolWireStyle::Ollama {
                        msg["name"] = json!(r.name);
                    }
                    messages.push(msg);
                }
            }
        }
    }
    let tools: Vec<Value> = req
        .tools
        .iter()
        .map(|t| json!({
            "type": "function",
            "function": {"name": t.name, "description": t.description, "parameters": t.parameters}
        }))
        .collect();
    json!({
        "model": req.model,
        "messages": messages,
        "tools": tools,
        "tool_choice": "auto",
        "max_tokens": req.max_tokens.unwrap_or(4096),
    })
}

/// Pure parser — unit-tested without network.
pub fn parse_tools_response(json: &Value) -> Result<ToolsChatResponse, ProviderError> {
    let msg = &json["choices"][0]["message"];
    if msg.is_null() {
        return Err(ProviderError::Parse("missing choices[0].message".into()));
    }
    let text = msg["content"].as_str().unwrap_or("").to_string();
    let tool_calls = parse_openai_tool_calls(&msg["tool_calls"]);
    let stop = match json["choices"][0]["finish_reason"].as_str() {
        Some("stop") => StopKind::Stop,
        Some("tool_calls") => StopKind::ToolUse,
        Some("length") => StopKind::Length,
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
        tokens_in: json["usage"]["prompt_tokens"].as_u64().unwrap_or(0),
        tokens_cached: cached_prompt_tokens(json),
        tokens_out: json["usage"]["completion_tokens"].as_u64().unwrap_or(0),
        latency_ms: 0,
        stop,
    })
}

/// Shared with Ollama (same wire shape, except `arguments` may be an object).
pub(crate) fn parse_openai_tool_calls(calls: &Value) -> Vec<ToolCall> {
    calls
        .as_array()
        .map(|arr| {
            arr.iter()
                .enumerate()
                .filter_map(|(i, c)| {
                    let name = c["function"]["name"].as_str()?.to_string();
                    let raw_args = &c["function"]["arguments"];
                    let args = match raw_args {
                        Value::String(s) => serde_json::from_str(s).unwrap_or(json!({})),
                        other => other.clone(),
                    };
                    Some(ToolCall {
                        id: c["id"]
                            .as_str()
                            .map(String::from)
                            .unwrap_or_else(|| format!("call_{i}")),
                        name,
                        args,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_chat_response() {
        let json = json!({
            "choices": [{"message": {"role": "assistant", "content": "OK"}}],
            "usage": {"prompt_tokens": 12, "completion_tokens": 3, "total_tokens": 15}
        });
        let r = parse_chat_response(&json).unwrap();
        assert_eq!(r.text, "OK");
        assert_eq!(r.tokens_in, 12);
        assert_eq!(r.tokens_out, 3);
        // no cache reported by this provider
        assert_eq!(r.tokens_cached, 0);
    }

    #[test]
    fn reads_cache_hits_as_a_subset_of_prompt_tokens() {
        let json = json!({
            "choices": [{"message": {"role": "assistant", "content": "OK"}}],
            "usage": {
                "prompt_tokens": 1000,
                "completion_tokens": 10,
                "prompt_tokens_details": {"cached_tokens": 800}
            }
        });
        let r = parse_chat_response(&json).unwrap();
        // prompt_tokens already includes the cached ones
        assert_eq!(r.tokens_in, 1000);
        assert_eq!(r.tokens_cached, 800);
    }

    #[test]
    fn accepts_the_deepseek_spelling_of_cache_hits() {
        let json = json!({
            "usage": {"prompt_tokens": 500, "prompt_cache_hit_tokens": 128}
        });
        assert_eq!(cached_prompt_tokens(&json), 128);
    }

    #[test]
    fn a_cache_count_larger_than_the_prompt_is_clamped() {
        let json = json!({
            "usage": {"prompt_tokens": 100, "prompt_tokens_details": {"cached_tokens": 999}}
        });
        assert_eq!(cached_prompt_tokens(&json), 100);
    }

    #[test]
    fn a_missing_usage_block_reports_no_cache() {
        assert_eq!(cached_prompt_tokens(&json!({})), 0);
        assert_eq!(cached_prompt_tokens(&json!({"usage": {}})), 0);
    }

    #[test]
    fn parses_models() {
        let json = json!({"data": [{"id": "gpt-5"}, {"id": "kimi-k3"}, {"no_id": true}]});
        let models = parse_models(&json);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "gpt-5");
    }

    #[test]
    fn rejects_bad_shape() {
        assert!(parse_chat_response(&json!({"unexpected": true})).is_err());
    }

    #[test]
    fn builds_tools_body_with_history() {
        let req = ToolsChatRequest {
            model: "m".into(),
            system: Some("sys".into()),
            history: vec![
                HistMsg::System("sys".into()),
                HistMsg::User("hi".into()),
                HistMsg::Assistant {
                    text: String::new(),
                    tool_calls: vec![ToolCall {
                        id: "c1".into(),
                        name: "read_file".into(),
                        args: json!({"path": "a.txt"}),
                    }],
                },
                HistMsg::ToolResults(vec![crate::chat::ToolOutcome {
                    id: "c1".into(),
                    name: "read_file".into(),
                    content: "data".into(),
                    is_error: false,
                }]),
            ],
            tools: vec![crate::chat::ToolSpec {
                name: "read_file".into(),
                description: "d".into(),
                parameters: json!({"type": "object"}),
            }],
            max_tokens: None,
        };
        let body = build_tools_body(&req);
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[1]["role"], "user");
        assert_eq!(msgs[2]["role"], "assistant");
        assert_eq!(msgs[2]["tool_calls"][0]["function"]["name"], "read_file");
        // arguments must be a *string* on the OpenAI wire format
        assert!(msgs[2]["tool_calls"][0]["function"]["arguments"].is_string());
        assert_eq!(msgs[3]["role"], "tool");
        assert_eq!(msgs[3]["tool_call_id"], "c1");
        assert_eq!(body["tools"][0]["function"]["name"], "read_file");
    }

    #[test]
    fn parses_tools_response_with_string_arguments() {
        let json = json!({
            "choices": [{"finish_reason": "tool_calls", "message": {
                "content": null,
                "tool_calls": [{"id": "c1", "type": "function",
                    "function": {"name": "run_shell", "arguments": "{\"command\":\"ls\"}"}}]
            }}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5}
        });
        let r = parse_tools_response(&json).unwrap();
        assert_eq!(r.stop, StopKind::ToolUse);
        assert_eq!(r.tool_calls.len(), 1);
        assert_eq!(r.tool_calls[0].args["command"], "ls");
        assert_eq!(r.tokens_in, 10);
    }
}
