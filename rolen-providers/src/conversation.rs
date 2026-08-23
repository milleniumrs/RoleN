//! Multi-turn chat with the ledger bookkeeping that goes with it.
//!
//! Both front-ends need the same three things per turn: send the whole
//! history, record a ledger entry, and update one session row for the whole
//! conversation. Keeping that in one place is the point - when it lived only
//! in the TUI it drifted into sending a fresh single-message request every
//! time, so the model never saw what had already been said.

use crate::chat::{ChatMessage, ChatRequest};
use crate::error::ProviderError;
use crate::registry::ProviderRegistry;
use rolen_core::ledger::Ledger;
use rolen_core::types::{LedgerEntry, Session, SessionState};

/// Running totals for the conversation so far, excluding the turn about to be
/// sent. The session row is replaced on each turn, so it has to be told what
/// came before or it would only ever show the last exchange.
#[derive(Debug, Clone, Copy, Default)]
pub struct Totals {
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost: f64,
}

/// What one turn produced.
#[derive(Debug, Clone)]
pub struct Turn {
    pub text: String,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost: f64,
    pub latency_ms: u64,
}

/// Send `history` (which must already end with the new user message) and
/// record the result.
///
/// Ledger writes are best effort: a chat reply that cannot be accounted for is
/// still a reply worth showing.
pub fn send(
    provider_id: &str,
    model: &str,
    history: Vec<ChatMessage>,
    session_id: &str,
    prior: Totals,
    max_tokens: u32,
) -> Result<Turn, ProviderError> {
    let reg = ProviderRegistry::load()?;
    let provider = reg
        .get(provider_id)
        .ok_or_else(|| ProviderError::NotFound(provider_id.to_string()))?
        .clone();

    let request = ChatRequest::conversation(model, history, max_tokens);
    let started = std::time::Instant::now();
    let response = crate::client::chat(&provider, &request)?;
    let latency_ms = started.elapsed().as_millis() as u64;

    let cost =
        crate::test::estimate_cost(&provider, model, response.tokens_in, response.tokens_out);

    if let Ok(ledger) = Ledger::open_default() {
        let now = chrono::Utc::now();
        let _ = ledger.record(&LedgerEntry {
            id: format!("le-{}", now.timestamp_nanos_opt().unwrap_or(0)),
            session_id: session_id.to_string(),
            provider_id: provider_id.to_string(),
            tokens_in: response.tokens_in,
            tokens_out: response.tokens_out,
            cost,
            latency_ms: Some(response.latency_ms),
            ts: now,
        });
        let _ = ledger.upsert_session(&Session {
            id: session_id.to_string(),
            task_id: None,
            provider_id: provider_id.to_string(),
            model: model.to_string(),
            state: SessionState::Done,
            tokens_in: prior.tokens_in + response.tokens_in,
            tokens_out: prior.tokens_out + response.tokens_out,
            cost: prior.cost + cost,
            started: now,
            transcript_path: None,
        });
    }

    Ok(Turn {
        text: response.text,
        tokens_in: response.tokens_in,
        tokens_out: response.tokens_out,
        cost,
        latency_ms,
    })
}

/// A conversation id, stable for the life of one chat window.
pub fn new_session_id() -> String {
    format!(
        "qc-{}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
    )
}
