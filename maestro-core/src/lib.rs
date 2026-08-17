//! maestro-core — UI-free core of Maestro (PRD NFR-5).
//!
//! Contains the domain types (PRD §8), TOML configuration (FR-14),
//! keychain-backed secrets (FR-2), the SQLite ledger (FR-4.6) and the
//! `config doctor` diagnostics used by both the TUI and the headless CLI.

pub mod config;
pub mod doctor;
pub mod error;
pub mod ledger;
pub mod project;
pub mod rules;
pub mod secrets;
pub mod types;
pub mod vault;

pub use error::CoreError;
