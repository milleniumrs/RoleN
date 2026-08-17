//! Routing context collection (PRD FR-3/FR-4): gathers the live provider
//! states the rule engine evaluates against — health, remaining quota,
//! cycle cost and the model catalog.

use crate::client;
use crate::error::ProviderError;
use crate::quota;
use crate::registry::ProviderRegistry;
use chrono::Datelike;
use maestro_core::ledger::Ledger;
use maestro_core::rules::{EvalContext, ProviderState};
use maestro_core::types::ProviderType;

/// Remaining quota in percent for a provider, if a plan limit is set.
/// Usage is measured from the ledger since the cycle start (or month start).
pub fn remaining_pct(provider_id: &str) -> Option<u8> {
    let subs = quota::load().ok()?;
    let sub = subs.iter().find(|s| s.provider_id == provider_id)?;
    let limit = sub.plan_limit?;
    if limit == 0 {
        return None;
    }
    let since = sub.cycle_start.unwrap_or_else(|| {
        let now = chrono::Utc::now();
        now.date_naive()
            .with_day(1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .map(|t| chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(t, chrono::Utc))
            .unwrap_or(now)
    });
    let ledger = Ledger::open_default().ok()?;
    let used = ledger
        .usage_since(Some(provider_id), &since.to_rfc3339())
        .ok()?;
    let used_total = used.total_tokens();
    let remaining = limit.saturating_sub(used_total);
    Some(((remaining * 100) / limit).min(100) as u8)
}

/// Collect the full evaluation context: every registered provider with
/// health, quota, cycle cost and models. CLI providers are listed but always
/// "unhealthy" for HTTP routing until the M5 adapters land.
pub fn collect(
    task_type: Option<String>,
    project: Option<String>,
) -> Result<EvalContext, ProviderError> {
    let reg = ProviderRegistry::load()?;
    let ledger = Ledger::open_default().ok();
    let month_start = {
        let now = chrono::Utc::now();
        now.date_naive()
            .with_day(1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .map(|t| chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(t, chrono::Utc))
            .unwrap_or(now)
            .to_rfc3339()
    };

    let mut providers = std::collections::HashMap::new();
    for p in reg.list() {
        let (healthy, models) = if p.ptype == ProviderType::Cli {
            (false, Vec::new())
        } else {
            let h = client::health(p);
            (h.ok, p.models.iter().map(|m| m.id.clone()).collect())
        };
        let cost_so_far = ledger
            .as_ref()
            .and_then(|l| l.usage_since(Some(&p.id), &month_start).ok())
            .map(|u| u.cost)
            .unwrap_or(0.0);
        providers.insert(
            p.id.clone(),
            ProviderState {
                id: p.id.clone(),
                healthy,
                quota_remaining_pct: remaining_pct(&p.id),
                cost_so_far,
                models,
            },
        );
    }

    Ok(EvalContext {
        task_type,
        project,
        now: Some(chrono::Utc::now()),
        providers,
    })
}
