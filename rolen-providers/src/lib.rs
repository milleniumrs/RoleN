//! rolen-providers — M1 (PRD FR-1/FR-4).
//!
//! Provider registry persisted to `providers.toml`, HTTP clients for
//! OpenAI-compatible, Anthropic and Ollama (local + cloud) APIs, model
//! discovery, health checks and the `test_prompt` flow that feeds the ledger.

pub mod anthropic;
pub mod chat;
pub mod client;
pub mod conversation;
pub mod detect;
pub mod error;
pub mod generate;
pub mod oauth;
pub mod ollama;
pub mod openai;
pub mod quota;
pub mod registry;
pub mod routing;
pub mod test;
pub mod tunnel;

pub use chat::{ChatMessage, ChatRequest, ChatResponse};
pub use client::Health;
pub use error::ProviderError;
pub use registry::ProviderRegistry;
pub use test::TestResult;
