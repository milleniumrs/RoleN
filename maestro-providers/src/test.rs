//! `test_prompt` — the M1 exit-criteria flow: send a prompt through a
//! provider, compute cost from the capability matrix, and ledger the usage
//! (FR-4.1/FR-4.6).

use crate::chat::ChatRequest;
use crate::client;
use crate::error::ProviderError;
use crate::registry::ProviderRegistry;
use maestro_core::ledger::Ledger;
use maestro_core::types::{LedgerEntry, Provider};

#[derive(Debug, Clone)]
pub struct TestResult {
    pub provider_id: String,
    pub model: String,
    pub text: String,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost: f64,
    pub latency_ms: u64,
}

/// Estimate cost in USD from the provider's per-model cost table (FR-1.5).
pub fn estimate_cost(provider: &Provider, model_id: &str, tokens_in: u64, tokens_out: u64) -> f64 {
    let Some(m) = provider.models.iter().find(|m| m.id == model_id) else {
        return 0.0;
    };
    let ci = m.cost_in_per_mtok.unwrap_or(0.0);
    let co = m.cost_out_per_mtok.unwrap_or(0.0);
    (tokens_in as f64 * ci + tokens_out as f64 * co) / 1_000_000.0
}

pub fn test_prompt(
    provider_id: &str,
    model: Option<&str>,
    prompt: &str,
) -> Result<TestResult, ProviderError> {
    let reg = ProviderRegistry::load()?;
    let provider = reg
        .get(provider_id)
        .ok_or_else(|| ProviderError::NotFound(provider_id.into()))?
        .clone();

    let model_id = match model {
        Some(m) => m.to_string(),
        None => provider
            .models
            .first()
            .map(|m| m.id.clone())
            .ok_or_else(|| ProviderError::Api(format!(
                "provider '{provider_id}' has no models; run `maestro provider models {provider_id}` first"
            )))?,
    };

    let req = ChatRequest::single(model_id.clone(), prompt);
    let resp = client::chat(&provider, &req)?;
    let cost = estimate_cost(&provider, &model_id, resp.tokens_in, resp.tokens_out);

    // ledger the usage (FR-4.6)
    let ledger = Ledger::open_default()?;
    ledger.record(&LedgerEntry {
        id: format!(
            "test-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ),
        session_id: format!("manual:{provider_id}"),
        provider_id: provider_id.to_string(),
        tokens_in: resp.tokens_in,
        tokens_out: resp.tokens_out,
        cost,
        latency_ms: Some(resp.latency_ms),
        ts: chrono::Utc::now(),
    })?;

    Ok(TestResult {
        provider_id: provider_id.to_string(),
        model: model_id,
        text: resp.text,
        tokens_in: resp.tokens_in,
        tokens_out: resp.tokens_out,
        cost,
        latency_ms: resp.latency_ms,
    })
}
