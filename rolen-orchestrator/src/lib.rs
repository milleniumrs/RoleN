//! rolen-orchestrator — M3 (PRD FR-7/FR-8).
//!
//! The orchestrator is the ONLY component that writes project files. Agents
//! submit write tickets; the global queue applies them with per-path FIFO
//! serialization, concurrent application across disjoint paths, optimistic
//! base-hash concurrency, atomic temp+rename writes, and SQLite journaling.

pub mod daggen;
pub mod git;
pub mod queue;
pub mod scheduler;

pub use queue::{QueuedWriteSink, TicketHandle, WriteQueue};
pub use scheduler::{run_batch, BatchEvent, BatchOptions, BatchReport, BatchSpec, TaskSpec};
