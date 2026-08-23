//! Ollama local + cloud (FR-1.1). Same API shape; cloud uses
//! https://ollama.com with a bearer key.

use crate::chat::{ChatRequest, ChatResponse};
use crate::error::ProviderError;
use rolen_core::types::Model;
use serde_json::{json, Value};

pub const DEFAULT_LOCAL_BASE: &str = "http://localhost:11434";
pub const DEFAULT_CLOUD_BASE: &str = "https://ollama.com";

pub fn chat(
    http: &reqwest::blocking::Client,
    base: &str,
    api_key: Option<&str>,
    req: &ChatRequest,
) -> Result<ChatResponse, ProviderError> {
    let url = format!("{}/api/chat", base.trim_end_matches('/'));
    let body = json!({
        "model": req.model,
        "messages": req.messages,
        "stream": false,
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
    let mut out = parse_chat(&json)?;
    out.latency_ms = latency_ms;
    Ok(out)
}

pub fn list_models(
    http: &reqwest::blocking::Client,
    base: &str,
    api_key: Option<&str>,
) -> Result<Vec<Model>, ProviderError> {
    let url = format!("{}/api/tags", base.trim_end_matches('/'));
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
    Ok(parse_tags(&json))
}

/// Pure parser — unit-tested without network.
pub fn parse_chat(json: &Value) -> Result<ChatResponse, ProviderError> {
    let text = json["message"]["content"]
        .as_str()
        .ok_or_else(|| ProviderError::Parse("missing message.content".into()))?
        .to_string();
    let tokens_in = json["prompt_eval_count"].as_u64().unwrap_or(0);
    let tokens_out = json["eval_count"].as_u64().unwrap_or(0);
    Ok(ChatResponse {
        text,
        tokens_in,
        tokens_out,
        latency_ms: 0,
    })
}

/// Pure parser — maps /api/tags entries to the capability matrix (FR-1.4),
/// tolerating older Ollama versions without the `capabilities` field.
pub fn parse_tags(json: &Value) -> Vec<Model> {
    json["models"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    let name = m["name"].as_str()?.to_string();
                    let caps: Vec<&str> = m["capabilities"]
                        .as_array()
                        .map(|c| c.iter().filter_map(|v| v.as_str()).collect())
                        .unwrap_or_default();
                    let context = m["details"]["context_length"].as_u64().map(|v| v as u32);
                    // fallback heuristics for older servers
                    let lname = name.to_lowercase();
                    let vision = caps.contains(&"vision")
                        || (caps.is_empty()
                            && (lname.contains("vision")
                                || lname.contains("-vl")
                                || lname.contains("minicpm-v")));
                    let tools = caps.contains(&"tools")
                        || (caps.is_empty()
                            && (lname.contains("qwen")
                                || lname.contains("mistral")
                                || lname.contains("llama3")));
                    Some(Model {
                        id: name,
                        context_tokens: context,
                        vision,
                        tools,
                        streaming: true,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn truncate(s: &str) -> String {
    s.chars().take(300).collect()
}

// ------------------------------------------------------------- tool calling

use crate::chat::{StopKind, ToolsChatRequest, ToolsChatResponse};
use crate::openai::{build_tools_body_styled, parse_openai_tool_calls, ToolWireStyle};

pub fn chat_tools(
    http: &reqwest::blocking::Client,
    base: &str,
    api_key: Option<&str>,
    req: &ToolsChatRequest,
) -> Result<ToolsChatResponse, ProviderError> {
    let url = format!("{}/api/chat", base.trim_end_matches('/'));
    // Ollama speaks the OpenAI message/tools shape at /api/chat, except:
    // arguments are JSON *objects* and tool messages carry the tool name.
    let mut body = build_tools_body_styled(req, ToolWireStyle::Ollama);
    body["stream"] = json!(false);
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
    let mut out = parse_tools_chat(&json)?;
    out.latency_ms = latency_ms;
    Ok(out)
}

/// Pure parser — Ollama wraps everything in `message` and counts tokens as
/// prompt_eval_count / eval_count.
pub fn parse_tools_chat(json: &Value) -> Result<ToolsChatResponse, ProviderError> {
    let msg = &json["message"];
    if msg.is_null() {
        return Err(ProviderError::Parse("missing message".into()));
    }
    let text = msg["content"].as_str().unwrap_or("").to_string();
    let tool_calls = parse_openai_tool_calls(&msg["tool_calls"]);
    Ok(ToolsChatResponse {
        text,
        stop: if tool_calls.is_empty() {
            StopKind::Stop
        } else {
            StopKind::ToolUse
        },
        tool_calls,
        tokens_in: json["prompt_eval_count"].as_u64().unwrap_or(0),
        tokens_out: json["eval_count"].as_u64().unwrap_or(0),
        latency_ms: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_chat() {
        let json = json!({
            "message": {"role": "assistant", "content": "OK"},
            "prompt_eval_count": 26,
            "eval_count": 5
        });
        let r = parse_chat(&json).unwrap();
        assert_eq!(r.text, "OK");
        assert_eq!(r.tokens_in, 26);
        assert_eq!(r.tokens_out, 5);
    }

    #[test]
    fn parses_tags_with_capabilities() {
        let json = json!({"models": [{
            "name": "mistral-small3.2:latest",
            "details": {"context_length": 131072},
            "capabilities": ["vision", "completion", "tools"]
        }]});
        let models = parse_tags(&json);
        assert_eq!(models.len(), 1);
        assert!(models[0].vision);
        assert!(models[0].tools);
        assert_eq!(models[0].context_tokens, Some(131072));
    }

    #[test]
    fn parses_tags_legacy_without_capabilities() {
        let json = json!({"models": [
            {"name": "qwen3:30b", "details": {"context_length": 262144}},
            {"name": "llava-vl:latest", "details": {}}
        ]});
        let models = parse_tags(&json);
        assert_eq!(models.len(), 2);
        assert!(models[0].tools); // qwen heuristic
        assert!(models[1].vision); // -vl heuristic
    }

    #[test]
    fn parses_tools_chat_with_object_arguments() {
        // Ollama returns tool call arguments as an *object* (not a string)
        let json = json!({
            "message": {"role": "assistant", "content": "",
                "tool_calls": [{"function": {"name": "submit_write",
                    "arguments": {"path": "a.txt", "content": "hi"}}}]},
            "prompt_eval_count": 33,
            "eval_count": 12
        });
        let r = parse_tools_chat(&json).unwrap();
        assert_eq!(r.stop, StopKind::ToolUse);
        assert_eq!(r.tool_calls[0].name, "submit_write");
        assert_eq!(r.tool_calls[0].args["path"], "a.txt");
        assert_eq!(r.tokens_in, 33);
    }
}
