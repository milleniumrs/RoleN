//! maestro-cliadapters — M5 (PRD FR-13).
//!
//! Wraps external CLI agents (`claude`, `codex`, `gemini`, …) as PTY
//! subprocesses via `portable-pty` (D7), streams their output into Maestro
//! session events/transcripts, and enforces the single-writer guarantee by
//! running them in an overlay directory whose diff is harvested and
//! re-applied through the orchestrator write queue (D3).

pub mod error;
pub mod overlay;
pub mod pty;
pub mod session;
pub mod spec;

pub use error::AdapterError;
pub use session::{run_cli_session, CliEvent, CliSessionReport};
pub use spec::CliSpec;
