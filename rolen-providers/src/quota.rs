//! Quota / subscription helpers (PRD FR-4.4): persisted to
//! `subscriptions.toml`; usage numbers come from the ledger.

use crate::error::ProviderError;
use rolen_core::config;
use rolen_core::types::{QuotaSource, Subscription};
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize)]
struct SubscriptionsFile {
    #[serde(default)]
    subscriptions: Vec<Subscription>,
}

pub fn load() -> Result<Vec<Subscription>, ProviderError> {
    let file = config::subscriptions_file()?;
    if !file.exists() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(&file)?;
    Ok(toml::from_str::<SubscriptionsFile>(&text)?.subscriptions)
}

pub fn save(subs: &[Subscription]) -> Result<(), ProviderError> {
    config::ensure_dirs()?;
    let text = toml::to_string_pretty(&SubscriptionsFile {
        subscriptions: subs.to_vec(),
    })?;
    std::fs::write(config::subscriptions_file()?, text)?;
    Ok(())
}

/// Create or update the manual-budget subscription for a provider (FR-4.2).
pub fn set_manual_budget(provider_id: &str, plan_limit: u64) -> Result<(), ProviderError> {
    let mut subs = load()?;
    if let Some(s) = subs.iter_mut().find(|s| s.provider_id == provider_id) {
        s.plan_limit = Some(plan_limit);
        s.source = QuotaSource::Manual;
    } else {
        subs.push(Subscription {
            provider_id: provider_id.to_string(),
            plan_limit: Some(plan_limit),
            used: 0,
            cycle_start: None,
            renewal: None,
            source: QuotaSource::Manual,
        });
    }
    save(&subs)
}

/// Remove any budget/subscription for a provider: its quota becomes "unknown"
/// again, which silences threshold alerts and makes rules treat it
/// optimistically. Returns true when an entry was removed.
pub fn clear_budget(provider_id: &str) -> Result<bool, ProviderError> {
    let mut subs = load()?;
    let before = subs.len();
    subs.retain(|s| s.provider_id != provider_id);
    let removed = subs.len() != before;
    if removed {
        save(&subs)?;
    }
    Ok(removed)
}

/// The configured plan limit for a provider, if any (used to explain alerts).
pub fn plan_limit(provider_id: &str) -> Option<(u64, QuotaSource)> {
    load()
        .ok()?
        .into_iter()
        .find(|s| s.provider_id == provider_id)
        .and_then(|s| s.plan_limit.map(|l| (l, s.source)))
}

/// FR-4.4: set the billing-cycle dates of a provider's subscription
/// (`renewal` also anchors the exhaustion forecast).
pub fn set_cycle(
    provider_id: &str,
    cycle_start: Option<chrono::DateTime<chrono::Utc>>,
    renewal: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<(), ProviderError> {
    let mut subs = load()?;
    if let Some(s) = subs.iter_mut().find(|s| s.provider_id == provider_id) {
        if cycle_start.is_some() {
            s.cycle_start = cycle_start;
        }
        if renewal.is_some() {
            s.renewal = renewal;
        }
    } else {
        subs.push(Subscription {
            provider_id: provider_id.to_string(),
            plan_limit: None,
            used: 0,
            cycle_start,
            renewal,
            source: QuotaSource::Manual,
        });
    }
    save(&subs)
}

/// FR-4.1/4.2: record quota numbers synced from a provider source (billing
/// endpoint or parsed CLI output).
pub fn record_synced(
    provider_id: &str,
    used: u64,
    limit: Option<u64>,
    source: QuotaSource,
) -> Result<(), ProviderError> {
    let mut subs = load()?;
    if let Some(s) = subs.iter_mut().find(|s| s.provider_id == provider_id) {
        s.used = used;
        if let Some(l) = limit {
            s.plan_limit = Some(l);
        }
        s.source = source;
    } else {
        subs.push(Subscription {
            provider_id: provider_id.to_string(),
            plan_limit: limit,
            used,
            cycle_start: None,
            renewal: None,
            source,
        });
    }
    save(&subs)
}
