//! rolen-runtime — M2 (PRD FR-12).
//!
//! The built-in agent loop for API/Ollama providers. Tools are sandboxed to a
//! workdir; there is deliberately NO write_file tool — all writes go through
//! `WriteSink` (M2: direct atomic write; M3: orchestrator write queue).

pub mod agent;
pub mod error;
pub mod sink;
pub mod tools;

pub use agent::{AgentEvent, AgentOptions, RunReport};
pub use error::RuntimeError;
