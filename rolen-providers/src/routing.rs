//! Routing context collection (PRD FR-3/FR-4): gathers the live provider
//! states the rule engine evaluates against — health, remaining quota,
//! cycle cost and the model catalog.

use crate::client;
use crate::error::ProviderError;
use crate::quota;
use crate::registry::ProviderRegistry;
use chrono::Datelike;
use rolen_core::ledger::Ledger;
use rolen_core::rules::{EvalContext, ProviderState};
use rolen_core::types::ProviderType;

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
    // FR-4.1/4.2: numbers synced from a billing endpoint or parsed CLI
    // output are authoritative; ledger estimates are the fallback
    let used_total = if matches!(
        sub.source,
        rolen_core::types::QuotaSource::Api | rolen_core::types::QuotaSource::Parsed
    ) && sub.used > 0
    {
        sub.used
    } else {
        ledger
            .usage_since(Some(provider_id), &since.to_rfc3339())
            .ok()?
            .total_tokens()
    };
    let remaining = limit.saturating_sub(used_total);
    Some(((remaining * 100) / limit).min(100) as u8)
}

/// Burn rate and exhaustion forecast (FR-9.2): `(tokens_per_day, days_left)`
/// for providers with a plan limit; None when no limit is set or nothing has
/// been used yet this cycle.
pub fn burn_rate(provider_id: &str) -> Option<(u64, f64)> {
    let subs = quota::load().ok()?;
    let sub = subs.iter().find(|s| s.provider_id == provider_id)?;
    let limit = sub.plan_limit?;
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
        .ok()?
        .total_tokens();
    if used == 0 {
        return None;
    }
    let days_elapsed = (chrono::Utc::now() - since).num_seconds().max(0) as f64 / 86400.0;
    let per_day = (used as f64 / days_elapsed.max(0.04)) as u64; // min ~1h window
    if per_day == 0 {
        return None;
    }
    let days_left = limit.saturating_sub(used) as f64 / per_day as f64;
    Some((per_day, days_left))
}

/// Collect the full evaluation context: every registered provider with
/// health, quota, cycle cost and models. CLI providers are listed but always
/// "unhealthy" for HTTP routing until the M5 adapters land.
///
/// This health-checks providers one at a time at a 30 s timeout each, so with
/// several unreachable endpoints it can block for minutes. Callers with a user
/// waiting should use [`collect_cancellable`].
pub fn collect(
    task_type: Option<String>,
    project: Option<String>,
) -> Result<EvalContext, ProviderError> {
    // Never cancelled, so the option is always Some.
    let ctx = collect_cancellable(task_type, project, &|| false, &mut |_, _, _| {})?;
    Ok(ctx.expect("collect_cancellable only yields None when cancelled"))
}

/// [`collect`] with a way out.
///
/// `cancelled` is polled before each provider is contacted and once at the
/// end; `Ok(None)` means the caller asked to stop. `on_provider` is called
/// with `(done_so_far, total, provider_id)` before each health check, which is
/// the only progress a caller can get - a single `client::health` call cannot
/// be interrupted once it is in flight, so the worst case after cancelling is
/// the remaining timeout of the provider already being probed, not the sum of
/// all of them.
pub fn collect_cancellable(
    task_type: Option<String>,
    project: Option<String>,
    cancelled: &dyn Fn() -> bool,
    on_provider: &mut dyn FnMut(usize, usize, &str),
) -> Result<Option<EvalContext>, ProviderError> {
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
    let total = reg.list().len();
    for (done, p) in reg.list().iter().enumerate() {
        if cancelled() {
            return Ok(None);
        }
        on_provider(done, total, &p.id);
        let (healthy, models) = if p.suspended {
            // FR-4.5: suspended providers are skipped by rule routing
            (false, p.models.iter().map(|m| m.id.clone()).collect())
        } else if p.ptype == ProviderType::Cli {
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

    if cancelled() {
        return Ok(None);
    }
    Ok(Some(EvalContext {
        task_type,
        project,
        now: Some(chrono::Utc::now()),
        providers,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Cancelling before the first provider must contact nobody. This is what
    /// makes the GUI's Cancel button worth having: without the early check the
    /// caller would still wait out every remaining 30 s timeout.
    #[test]
    fn cancelling_up_front_contacts_no_provider() {
        let seen = AtomicUsize::new(0);
        let mut on_provider = |_: usize, _: usize, _: &str| {
            seen.fetch_add(1, Ordering::Relaxed);
        };
        let out = collect_cancellable(None, None, &|| true, &mut on_provider)
            .expect("registry load should succeed");
        assert!(
            out.is_none(),
            "cancelled collection must not return a context"
        );
        assert_eq!(
            seen.load(Ordering::Relaxed),
            0,
            "no provider should have been probed"
        );
    }
}
