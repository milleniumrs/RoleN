//! Tool-calling chat types (PRD FR-12.1/FR-12.2). Provider-agnostic history
//! representation; each client module converts to its wire format.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

impl ChatMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: content.into(),
        }
    }

    /// A reply from the model, so a front-end can feed the conversation back in.
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".into(),
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub max_tokens: Option<u32>,
}

impl ChatRequest {
    /// One-shot probe with a deliberately small cap - used by `provider test`.
    /// Not suitable for a conversation: see [`ChatRequest::conversation`].
    pub fn single(model: impl Into<String>, prompt: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            messages: vec![ChatMessage::user(prompt)],
            max_tokens: Some(256),
        }
    }

    /// A multi-turn request carrying the whole history.
    ///
    /// The caller states its own output cap, because 256 tokens is a sensible
    /// ceiling for a health probe and a useless one for a chat reply.
    pub fn conversation(
        model: impl Into<String>,
        messages: Vec<ChatMessage>,
        max_tokens: u32,
    ) -> Self {
        Self {
            model: model.into(),
            messages,
            max_tokens: Some(max_tokens),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ChatResponse {
    pub text: String,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub latency_ms: u64,
}

// ------------------------------------------------------------- tool calling

/// A tool the agent may call (JSON Schema parameters).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub args: Value,
}

#[derive(Debug, Clone)]
pub struct ToolOutcome {
    pub id: String,
    /// Tool name (required by Ollama tool messages; ignored elsewhere).
    pub name: String,
    pub content: String,
    pub is_error: bool,
}

/// Provider-agnostic conversation history entry.
#[derive(Debug, Clone)]
pub enum HistMsg {
    System(String),
    User(String),
    Assistant {
        text: String,
        tool_calls: Vec<ToolCall>,
    },
    ToolResults(Vec<ToolOutcome>),
}

#[derive(Debug, Clone)]
pub struct ToolsChatRequest {
    pub model: String,
    pub system: Option<String>,
    pub history: Vec<HistMsg>,
    pub tools: Vec<ToolSpec>,
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StopKind {
    Stop,
    ToolUse,
    Length,
    #[default]
    Other,
}

#[derive(Debug, Clone, Default)]
pub struct ToolsChatResponse {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub latency_ms: u64,
    pub stop: StopKind,
}
