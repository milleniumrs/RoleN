//! Per-model prices in USD per million tokens (FR-1.5).
//!
//! Prices live in their own file rather than on [`crate::types::Model`],
//! because `refresh_models` replaces a provider's whole model vector on every
//! discovery: a price stored there is destroyed by the next Detect, Health
//! Check or "Discover models". Keying on `(provider_id, model_id)` also means
//! a model that disappears from a provider's catalogue keeps its price if it
//! comes back.
//!
//! RoleN ships no price table. Vendors change prices without notice and a
//! stale number compiled into a release is worse than an honest "unknown", so
//! every rate here is one somebody entered.

use crate::config;
use crate::error::CoreError;
use crate::types::{Billing, ProviderType};
use serde::{Deserialize, Serialize};
use std::fs;

/// The token counts of one call, split the way the price list bills them.
///
/// `input` is every prompt token. The three cache figures are *subsets* of it,
/// not extras; whatever is left over is fresh input.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tokens {
    pub input: u64,
    /// Served from cache, billed at the cheap hit rate.
    pub cache_read: u64,
    /// Written into a short-lived cache, billed at a premium.
    pub cache_write_5m: u64,
    /// Written into a long-lived cache, billed at a larger premium.
    pub cache_write_1h: u64,
    pub output: u64,
}

impl Tokens {
    /// Prompt tokens that were neither read from nor written to the cache.
    ///
    /// Clamped, because a provider is free to report counts that do not add
    /// up and a negative here would turn into a credit.
    pub fn fresh_input(self) -> u64 {
        self.input
            .saturating_sub(self.cache_read)
            .saturating_sub(self.cache_write_5m)
            .saturating_sub(self.cache_write_1h)
    }
}

/// What one model costs, per million tokens.
///
/// Only input and output are required. Every cache rate is optional and falls
/// back to the input rate, because most providers have no separate cache
/// pricing and RoleN does not invent multipliers.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rates {
    /// USD per million fresh (uncached) input tokens.
    pub input: f64,
    /// USD per million output (completion) tokens.
    pub output: f64,
    /// USD per million input tokens served from the provider's prompt cache.
    pub cache_read: Option<f64>,
    /// USD per million tokens written to a short-lived (5-minute) cache.
    /// Anthropic charges 1.25x input for this.
    pub cache_write_5m: Option<f64>,
    /// USD per million tokens written to a long-lived (1-hour) cache.
    /// Anthropic charges 2x input for this.
    pub cache_write_1h: Option<f64>,
}

impl Rates {
    /// The simple case: no cache pricing, so every prompt token costs the
    /// same whatever the cache did.
    pub fn flat(input: f64, output: f64) -> Self {
        Self {
            input,
            output,
            ..Default::default()
        }
    }

    fn read_rate(self) -> f64 {
        self.cache_read.unwrap_or(self.input)
    }

    fn write_5m_rate(self) -> f64 {
        self.cache_write_5m.unwrap_or(self.input)
    }

    fn write_1h_rate(self) -> f64 {
        self.cache_write_1h.unwrap_or(self.input)
    }
}

/// A rate somebody entered for one model of one provider.
///
/// Kept flat rather than nesting [`Rates`] so `pricing.toml` stays pleasant to
/// edit by hand.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelPrice {
    pub provider_id: String,
    pub model_id: String,
    pub in_per_mtok: f64,
    pub out_per_mtok: f64,
    #[serde(default)]
    pub cached_in_per_mtok: Option<f64>,
    #[serde(default)]
    pub cache_write_5m_per_mtok: Option<f64>,
    #[serde(default)]
    pub cache_write_1h_per_mtok: Option<f64>,
}

impl ModelPrice {
    pub fn rates(&self) -> Rates {
        Rates {
            input: self.in_per_mtok,
            output: self.out_per_mtok,
            cache_read: self.cached_in_per_mtok,
            cache_write_5m: self.cache_write_5m_per_mtok,
            cache_write_1h: self.cache_write_1h_per_mtok,
        }
    }
}

/// What RoleN can honestly say about one model's price.
///
/// The distinction that matters is between [`Price::Unknown`], where a rate
/// exists and nobody has entered it, and [`Price::Plan`], where no per-token
/// rate exists at all. Both cost 0.0 by default, but only the first is a gap
/// somebody can close.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Price {
    /// Runs on hardware you already pay for; there is no per-token charge.
    Free,
    /// Metered, and rates have been entered.
    Known(Rates),
    /// Billed by subscription: the provider publishes no per-token rate and
    /// meters an allowance instead. Any rates here are the user's own
    /// estimate for budgeting, not a quoted price, and are billed when set.
    Plan(Option<Rates>),
    /// Metered, but nobody has told RoleN the rate. Cost is not estimated.
    Unknown,
}

impl Price {
    /// USD for a call of this size.
    ///
    /// Each bucket of [`Tokens`] is billed at its own rate, with every cache
    /// rate falling back to the input rate when it was not entered.
    /// [`Price::Unknown`] deliberately yields 0.0: guessing a rate would put
    /// invented money in the ledger.
    pub fn cost(self, t: Tokens) -> f64 {
        match self.rates() {
            None => 0.0,
            Some(r) => {
                (t.fresh_input() as f64 * r.input
                    + t.cache_read.min(t.input) as f64 * r.read_rate()
                    + t.cache_write_5m.min(t.input) as f64 * r.write_5m_rate()
                    + t.cache_write_1h.min(t.input) as f64 * r.write_1h_rate()
                    + t.output as f64 * r.output)
                    / 1_000_000.0
            }
        }
    }

    /// The rates to bill at, if there are any.
    pub fn rates(self) -> Option<Rates> {
        match self {
            Price::Known(r) | Price::Plan(Some(r)) => Some(r),
            Price::Free | Price::Plan(None) | Price::Unknown => None,
        }
    }

    /// True when RoleN can account for a call: it has a rate, or it knows
    /// there is nothing to charge. Only [`Price::Unknown`] is a real gap.
    pub fn is_known(self) -> bool {
        !matches!(self, Price::Unknown)
    }

    /// True when the rates are the user's own guess rather than a quoted
    /// price, so the UI can mark the numbers as approximate.
    pub fn is_estimate(self) -> bool {
        matches!(self, Price::Plan(Some(_)))
    }

    pub fn input_label(self) -> String {
        self.label(|r| r.input)
    }

    pub fn output_label(self) -> String {
        self.label(|r| r.output)
    }

    /// The cached-hit rate. A model with no explicit cached rate shows the
    /// input rate it falls back to — that is known, not unknown.
    pub fn cached_label(self) -> String {
        self.label(|r| r.read_rate())
    }

    /// The two cache-write rates as one cell, "6.25/10.00" style, since they
    /// are only ever interesting next to each other.
    pub fn cache_write_label(self) -> String {
        match (self, self.rates()) {
            (Price::Free, _) => "free".to_string(),
            (Price::Plan(None), _) => "plan".to_string(),
            (_, None) => "unknown".to_string(),
            (_, Some(r)) => format!(
                "{}{}/{}",
                if self.is_estimate() { "~" } else { "" },
                fmt_rate(r.write_5m_rate()),
                fmt_rate(r.write_1h_rate())
            ),
        }
    }

    fn label(self, pick: fn(Rates) -> f64) -> String {
        match (self, self.rates()) {
            (Price::Free, _) => "free".to_string(),
            // No per-token rate exists, so there is nothing to show and
            // nothing anyone could enter to make it appear.
            (Price::Plan(None), _) => "plan".to_string(),
            (_, None) => "unknown".to_string(),
            // A tilde marks a number the user estimated, not one a vendor
            // published.
            (_, Some(r)) => format!(
                "{}${}",
                if self.is_estimate() { "~" } else { "" },
                fmt_rate(pick(r))
            ),
        }
    }
}

/// Render a rate with two to four decimals: enough for $0.075 without
/// printing $3.0000 for a round one.
pub fn fmt_rate(v: f64) -> String {
    let mut s = format!("{v:.4}");
    if let Some(dot) = s.find('.') {
        while s.len() - dot - 1 > 2 && s.ends_with('0') {
            s.pop();
        }
    }
    s
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct PricingFile {
    #[serde(default)]
    prices: Vec<ModelPrice>,
}

/// The contents of `pricing.toml`. Everything except [`Pricing::load`] and
/// [`Pricing::save`] is pure, so the rules below are unit-tested without
/// touching the real config directory.
#[derive(Debug, Default, Clone)]
pub struct Pricing {
    entries: Vec<ModelPrice>,
}

impl Pricing {
    /// Read `pricing.toml`. A missing file is an empty price list, not an
    /// error: no prices entered yet is the normal starting state.
    pub fn load() -> Result<Self, CoreError> {
        let path = config::pricing_file()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = fs::read_to_string(path)?;
        let file: PricingFile = toml::from_str(&text)?;
        Ok(Self {
            entries: file.prices,
        })
    }

    pub fn save(&self) -> Result<(), CoreError> {
        config::ensure_dirs()?;
        let mut file = PricingFile {
            prices: self.entries.clone(),
        };
        // Stable order, so hand-edits and rewrites do not shuffle the file.
        file.prices
            .sort_by(|a, b| (&a.provider_id, &a.model_id).cmp(&(&b.provider_id, &b.model_id)));
        let text = toml::to_string_pretty(&file)?;
        fs::write(config::pricing_file()?, text)?;
        Ok(())
    }

    pub fn get(&self, provider_id: &str, model_id: &str) -> Option<&ModelPrice> {
        self.entries
            .iter()
            .find(|e| e.provider_id == provider_id && e.model_id == model_id)
    }

    /// Insert or overwrite the rates for one model. Any cache rate left as
    /// `None` is billed at the input rate.
    pub fn set(&mut self, provider_id: &str, model_id: &str, r: Rates) {
        match self
            .entries
            .iter_mut()
            .find(|e| e.provider_id == provider_id && e.model_id == model_id)
        {
            Some(e) => {
                e.in_per_mtok = r.input;
                e.out_per_mtok = r.output;
                e.cached_in_per_mtok = r.cache_read;
                e.cache_write_5m_per_mtok = r.cache_write_5m;
                e.cache_write_1h_per_mtok = r.cache_write_1h;
            }
            None => self.entries.push(ModelPrice {
                provider_id: provider_id.to_string(),
                model_id: model_id.to_string(),
                in_per_mtok: r.input,
                out_per_mtok: r.output,
                cached_in_per_mtok: r.cache_read,
                cache_write_5m_per_mtok: r.cache_write_5m,
                cache_write_1h_per_mtok: r.cache_write_1h,
            }),
        }
    }

    /// Forget the rate for one model. Returns whether anything was removed.
    pub fn clear(&mut self, provider_id: &str, model_id: &str) -> bool {
        let before = self.entries.len();
        self.entries
            .retain(|e| !(e.provider_id == provider_id && e.model_id == model_id));
        self.entries.len() != before
    }

    /// What to show, and charge, for one model.
    ///
    /// A free provider is [`Price::Free`] even if a rate was entered: the
    /// hardware is yours either way, and a stray entry must not start billing
    /// a local model.
    pub fn resolve(&self, ptype: ProviderType, provider_id: &str, model_id: &str) -> Price {
        let entered = || self.get(provider_id, model_id).map(|e| e.rates());
        match ptype.billing() {
            Billing::Free => Price::Free,
            Billing::Subscription => Price::Plan(entered()),
            Billing::PerToken => match entered() {
                Some(r) => Price::Known(r),
                None => Price::Unknown,
            },
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_and_tunnelled_ollama_are_free() {
        let p = Pricing::default();
        assert_eq!(
            p.resolve(ProviderType::OllamaLocal, "ollama-local", "qwen3:30b"),
            Price::Free
        );
        assert_eq!(
            p.resolve(ProviderType::OllamaRemote, "ollama-box", "qwen3:30b"),
            Price::Free
        );
    }

    #[test]
    fn a_subscription_provider_is_on_a_plan_not_unknown() {
        // "unknown" means a rate exists and nobody entered it. A wrapped CLI
        // agent and Ollama Cloud publish no per-token rate at all, so there is
        // nothing anyone could enter to resolve it.
        let p = Pricing::default();
        for t in [ProviderType::Cli, ProviderType::OllamaCloud] {
            let price = p.resolve(t, "cli-claude", "claude-sonnet-5");
            assert_eq!(price, Price::Plan(None), "{t:?}");
            assert_eq!(price.input_label(), "plan");
            assert_eq!(price.cache_write_label(), "plan");
            // Costed as zero, but not because the rate is missing.
            assert_eq!(price.cost(prompt(1_000_000, 1_000_000)), 0.0);
            assert!(price.is_known());
            assert!(!price.is_estimate());
        }
    }

    #[test]
    fn an_estimate_on_a_plan_provider_is_billed_and_marked() {
        let mut p = Pricing::default();
        p.set("cli-claude", "claude-sonnet-5", Rates::flat(2.0, 10.0));
        let price = p.resolve(ProviderType::Cli, "cli-claude", "claude-sonnet-5");
        assert_eq!(price, Price::Plan(Some(Rates::flat(2.0, 10.0))));
        assert!(price.is_estimate());
        // The tilde is what tells the user this is their guess, not a quote.
        assert_eq!(price.input_label(), "~$2.00");
        assert_eq!(price.output_label(), "~$10.00");
        assert_eq!(price.cost(prompt(1_000_000, 0)), 2.0);
    }

    #[test]
    fn a_per_token_rate_is_not_marked_as_an_estimate() {
        assert!(!k3().is_estimate());
        assert_eq!(k3().input_label(), "$3.00");
    }

    #[test]
    fn only_unknown_is_a_gap_somebody_can_close() {
        assert!(Price::Free.is_known());
        assert!(Price::Plan(None).is_known());
        assert!(k3().is_known());
        assert!(!Price::Unknown.is_known());
    }

    #[test]
    fn an_api_provider_without_a_rate_is_unknown() {
        // The one case where "unknown" is right: a metered API whose rate is
        // published somewhere, just not entered here yet.
        let p = Pricing::default();
        assert_eq!(
            p.resolve(ProviderType::Api, "kimi", "kimi-for-coding"),
            Price::Unknown
        );
        assert_eq!(
            p.resolve(ProviderType::Api, "kimi", "x").input_label(),
            "unknown"
        );
    }

    #[test]
    fn ollama_cloud_is_not_metered_like_an_api() {
        // It sells a subscription against an opaque compute allowance and
        // publishes no per-token rate, so it must not be reported as a
        // metered provider with a missing number.
        let p = Pricing::default();
        assert_eq!(
            p.resolve(ProviderType::OllamaCloud, "ollama-cloud", "gpt-oss:20b"),
            Price::Plan(None)
        );
    }

    #[test]
    fn a_rate_on_an_unmetered_provider_stays_free() {
        let mut p = Pricing::default();
        p.set("ollama-local", "qwen3:30b", Rates::flat(99.0, 99.0));
        assert_eq!(
            p.resolve(ProviderType::OllamaLocal, "ollama-local", "qwen3:30b"),
            Price::Free
        );
    }

    #[test]
    fn set_then_resolve_returns_the_rate() {
        let mut p = Pricing::default();
        p.set("kimi", "kimi-for-coding", k3_rates());
        assert_eq!(
            p.resolve(ProviderType::Api, "kimi", "kimi-for-coding"),
            Price::Known(k3_rates())
        );
    }

    #[test]
    fn set_overwrites_rather_than_duplicating() {
        let mut p = Pricing::default();
        p.set("kimi", "k2", k3_rates());
        p.set("kimi", "k2", Rates::flat(1.0, 2.0));
        assert_eq!(p.len(), 1);
        assert_eq!(p.get("kimi", "k2").unwrap().in_per_mtok, 1.0);
        // the cached rate is part of the overwrite, not merged with the old one
        assert_eq!(p.get("kimi", "k2").unwrap().cached_in_per_mtok, None);
    }

    #[test]
    fn prices_are_scoped_to_one_provider() {
        let mut p = Pricing::default();
        p.set("kimi", "shared-name", Rates::flat(3.0, 15.0));
        assert_eq!(
            p.resolve(ProviderType::Api, "other", "shared-name"),
            Price::Unknown
        );
    }

    #[test]
    fn clear_reports_whether_it_removed_anything() {
        let mut p = Pricing::default();
        p.set("kimi", "k2", Rates::flat(3.0, 15.0));
        assert!(p.clear("kimi", "k2"));
        assert!(!p.clear("kimi", "k2"));
        assert!(p.is_empty());
    }

    /// The rates on a Kimi K3 subscription page: no separate cache-write
    /// pricing, so writes fall back to the input rate.
    fn k3_rates() -> Rates {
        Rates {
            input: 3.00,
            output: 15.00,
            cache_read: Some(0.30),
            ..Default::default()
        }
    }

    fn k3() -> Price {
        Price::Known(k3_rates())
    }

    /// Claude Opus 5: $5 in, $25 out, $0.50 hit, $6.25 5m write, $10 1h write.
    fn opus5() -> Price {
        Price::Known(Rates {
            input: 5.00,
            output: 25.00,
            cache_read: Some(0.50),
            cache_write_5m: Some(6.25),
            cache_write_1h: Some(10.00),
        })
    }

    fn prompt(input: u64, output: u64) -> Tokens {
        Tokens {
            input,
            output,
            ..Default::default()
        }
    }

    #[test]
    fn cost_is_per_million_tokens() {
        assert_eq!(k3().cost(prompt(1_000_000, 0)), 3.0);
        assert_eq!(k3().cost(prompt(0, 1_000_000)), 15.0);
        assert_eq!(k3().cost(prompt(500_000, 100_000)), 1.5 + 1.5);
    }

    #[test]
    fn cache_hits_bill_at_the_cached_rate() {
        // 1M prompt tokens, 800k of them cache hits, 100k output:
        //   200k fresh  @ $3.00  = $0.60
        //   800k cached @ $0.30  = $0.24
        //   100k out    @ $15.00 = $1.50
        let cost = k3().cost(Tokens {
            input: 1_000_000,
            cache_read: 800_000,
            output: 100_000,
            ..Default::default()
        });
        assert!((cost - 2.34).abs() < 1e-9, "got {cost}");
    }

    #[test]
    fn cache_writes_bill_above_the_input_rate() {
        // Opus 5: 1M tokens all written to a 1h cache is $10, double the $5
        // input rate. Billing writes as plain input would say $5.
        let w1h = opus5().cost(Tokens {
            input: 1_000_000,
            cache_write_1h: 1_000_000,
            ..Default::default()
        });
        assert_eq!(w1h, 10.0);

        let w5m = opus5().cost(Tokens {
            input: 1_000_000,
            cache_write_5m: 1_000_000,
            ..Default::default()
        });
        assert_eq!(w5m, 6.25);

        assert!(w5m > opus5().cost(prompt(1_000_000, 0)));
    }

    #[test]
    fn a_realistic_opus_turn_bills_every_bucket_at_its_own_rate() {
        //   100k fresh   @ $5.00  = $0.50
        //   500k hits    @ $0.50  = $0.25
        //   300k 5m writ @ $6.25  = $1.875
        //   100k 1h writ @ $10.00 = $1.00
        //    20k out     @ $25.00 = $0.50
        let cost = opus5().cost(Tokens {
            input: 1_000_000,
            cache_read: 500_000,
            cache_write_5m: 300_000,
            cache_write_1h: 100_000,
            output: 20_000,
        });
        assert!((cost - 4.125).abs() < 1e-9, "got {cost}");
    }

    #[test]
    fn fresh_input_is_whatever_the_cache_buckets_did_not_claim() {
        let t = Tokens {
            input: 1_000,
            cache_read: 400,
            cache_write_5m: 300,
            cache_write_1h: 100,
            output: 0,
        };
        assert_eq!(t.fresh_input(), 200);
    }

    #[test]
    fn a_fully_cached_prompt_costs_the_cached_rate_only() {
        assert_eq!(
            k3().cost(Tokens {
                input: 1_000_000,
                cache_read: 1_000_000,
                ..Default::default()
            }),
            0.30
        );
    }

    #[test]
    fn cached_tokens_are_a_subset_not_an_extra() {
        // Billing must never exceed the no-cache price for the same prompt.
        let no_cache = k3().cost(prompt(1_000_000, 0));
        let cached = k3().cost(Tokens {
            input: 1_000_000,
            cache_read: 400_000,
            ..Default::default()
        });
        assert!(cached < no_cache);
    }

    #[test]
    fn overreported_cache_counts_cannot_bill_negative_input() {
        // A provider whose buckets exceed its own prompt total must not
        // produce a credit.
        let t = Tokens {
            input: 1_000,
            cache_read: 10_000,
            cache_write_5m: 10_000,
            cache_write_1h: 10_000,
            output: 0,
        };
        assert_eq!(t.fresh_input(), 0);
        assert!(opus5().cost(t) > 0.0);
    }

    #[test]
    fn without_cache_rates_every_bucket_bills_as_normal_input() {
        let p = Price::Known(Rates::flat(3.0, 15.0));
        let mixed = p.cost(Tokens {
            input: 1_000_000,
            cache_read: 300_000,
            cache_write_5m: 300_000,
            cache_write_1h: 150_000,
            ..Default::default()
        });
        assert_eq!(mixed, p.cost(prompt(1_000_000, 0)));
    }

    #[test]
    fn free_and_unknown_both_cost_nothing() {
        let t = Tokens {
            input: 1_000_000,
            cache_read: 500_000,
            cache_write_1h: 100_000,
            output: 1_000_000,
            ..Default::default()
        };
        assert_eq!(Price::Free.cost(t), 0.0);
        assert_eq!(Price::Unknown.cost(t), 0.0);
    }

    #[test]
    fn unknown_is_the_only_uncostable_price() {
        assert!(Price::Free.is_known());
        assert!(k3().is_known());
        assert!(!Price::Unknown.is_known());
    }

    #[test]
    fn labels_say_free_and_unknown_in_words() {
        assert_eq!(Price::Free.input_label(), "free");
        assert_eq!(Price::Free.cached_label(), "free");
        assert_eq!(Price::Free.cache_write_label(), "free");
        assert_eq!(Price::Unknown.output_label(), "unknown");
        assert_eq!(Price::Unknown.cached_label(), "unknown");
        assert_eq!(Price::Unknown.cache_write_label(), "unknown");
        assert_eq!(opus5().input_label(), "$5.00");
        assert_eq!(opus5().output_label(), "$25.00");
        assert_eq!(opus5().cached_label(), "$0.50");
        assert_eq!(opus5().cache_write_label(), "6.25/10.00");
    }

    #[test]
    fn missing_rates_show_the_input_rate_they_fall_back_to() {
        assert_eq!(k3().cache_write_label(), "3.00/3.00");
        assert_eq!(Price::Known(Rates::flat(3.0, 15.0)).cached_label(), "$3.00");
    }

    #[test]
    fn rates_keep_two_to_four_decimals() {
        assert_eq!(fmt_rate(3.0), "3.00");
        assert_eq!(fmt_rate(100.0), "100.00");
        assert_eq!(fmt_rate(0.075), "0.075");
        assert_eq!(fmt_rate(0.15), "0.15");
        assert_eq!(fmt_rate(0.0001), "0.0001");
        assert_eq!(fmt_rate(12.34), "12.34");
    }

    #[test]
    fn the_file_round_trips() {
        let mut p = Pricing::default();
        p.set("kimi", "k2", k3_rates());
        p.set("openai", "gpt-4o", Rates::flat(2.5, 10.0));
        let text = toml::to_string_pretty(&PricingFile {
            prices: p.entries.clone(),
        })
        .unwrap();
        let back: PricingFile = toml::from_str(&text).unwrap();
        assert_eq!(back.prices, p.entries);
    }

    #[test]
    fn a_hand_written_file_with_every_rate_loads() {
        // Exactly the shape written into pricing.toml by hand, so a typo in
        // the field names shows up here rather than as a silently ignored
        // rate that bills at the input price.
        let text = r#"
[[prices]]
provider_id = "claude-sub"
model_id = "claude-opus-5"
in_per_mtok = 5.0
out_per_mtok = 25.0
cached_in_per_mtok = 0.5
cache_write_5m_per_mtok = 6.25
cache_write_1h_per_mtok = 10.0
"#;
        let back: PricingFile = toml::from_str(text).unwrap();
        let r = back.prices[0].rates();
        assert_eq!(r.input, 5.0);
        assert_eq!(r.output, 25.0);
        assert_eq!(r.cache_read, Some(0.5));
        assert_eq!(r.cache_write_5m, Some(6.25));
        assert_eq!(r.cache_write_1h, Some(10.0));
    }

    #[test]
    fn a_price_file_without_a_cached_rate_still_loads() {
        // Files written before cached pricing existed have no such key.
        let text = "[[prices]]\nprovider_id = \"kimi\"\nmodel_id = \"k2\"\nin_per_mtok = 3.0\nout_per_mtok = 15.0\n";
        let back: PricingFile = toml::from_str(text).unwrap();
        assert_eq!(back.prices[0].cached_in_per_mtok, None);
    }
}
