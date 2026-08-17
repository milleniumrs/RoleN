//! Quota / subscription helpers (PRD FR-4.4): persisted to
//! `subscriptions.toml`; usage numbers come from the ledger.

use crate::error::ProviderError;
use maestro_core::config;
use maestro_core::types::{QuotaSource, Subscription};
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
