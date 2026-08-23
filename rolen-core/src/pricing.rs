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
use crate::types::ProviderType;
use serde::{Deserialize, Serialize};
use std::fs;

/// A rate somebody entered for one model of one provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelPrice {
    pub provider_id: String,
    pub model_id: String,
    /// USD per million fresh (uncached) input tokens.
    pub in_per_mtok: f64,
    /// USD per million output (completion) tokens.
    pub out_per_mtok: f64,
    /// USD per million input tokens served from the provider's prompt cache.
    /// `None` means the provider does not price cache hits separately, so
    /// they are billed at `in_per_mtok`.
    #[serde(default)]
    pub cached_in_per_mtok: Option<f64>,
}

/// What RoleN can honestly say about one model's price.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Price {
    /// Runs on hardware you already pay for; there is no per-token charge.
    Free,
    /// Metered, and a rate has been entered.
    Known {
        in_per_mtok: f64,
        out_per_mtok: f64,
        cached_in_per_mtok: Option<f64>,
    },
    /// Metered, but nobody has told RoleN the rate. Cost is not estimated.
    Unknown,
}

impl Price {
    /// USD for a call of this size.
    ///
    /// `tokens_in` is every prompt token; `tokens_cached` is the subset that
    /// was a cache hit, and is billed at the cached rate instead of the input
    /// rate. [`Price::Unknown`] deliberately yields 0.0: guessing a rate would
    /// put invented money in the ledger.
    pub fn cost(self, tokens_in: u64, tokens_cached: u64, tokens_out: u64) -> f64 {
        match self {
            Price::Free | Price::Unknown => 0.0,
            Price::Known {
                in_per_mtok,
                out_per_mtok,
                cached_in_per_mtok,
            } => {
                // A provider that reports more cache hits than prompt tokens
                // would otherwise bill negative fresh input.
                let cached = tokens_cached.min(tokens_in);
                let fresh = tokens_in - cached;
                let cached_rate = cached_in_per_mtok.unwrap_or(in_per_mtok);
                (fresh as f64 * in_per_mtok
                    + cached as f64 * cached_rate
                    + tokens_out as f64 * out_per_mtok)
                    / 1_000_000.0
            }
        }
    }

    /// The cached-input rate as shown in the price grid: "free" for unmetered
    /// providers, "unknown" when no rate is entered, and otherwise either the
    /// explicit cached rate or the input rate it falls back to.
    pub fn cached_label(self) -> String {
        match self {
            Price::Free => "free".to_string(),
            Price::Unknown => "unknown".to_string(),
            Price::Known {
                in_per_mtok,
                cached_in_per_mtok,
                ..
            } => match cached_in_per_mtok {
                Some(v) => format!("${}", fmt_rate(v)),
                // Not "unknown": it is known, it is just the same as input.
                None => format!("${}", fmt_rate(in_per_mtok)),
            },
        }
    }

    /// True when a call through this model can be costed at all.
    pub fn is_known(self) -> bool {
        !matches!(self, Price::Unknown)
    }

    pub fn input_label(self) -> String {
        self.label(|p| p.0)
    }

    pub fn output_label(self) -> String {
        self.label(|p| p.1)
    }

    fn label(self, pick: fn((f64, f64)) -> f64) -> String {
        match self {
            Price::Free => "free".to_string(),
            Price::Unknown => "unknown".to_string(),
            Price::Known {
                in_per_mtok,
                out_per_mtok,
                ..
            } => format!("${}", fmt_rate(pick((in_per_mtok, out_per_mtok)))),
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

    /// Insert or overwrite the rates for one model. `cached_in_per_mtok` of
    /// `None` means cache hits are billed at the input rate.
    pub fn set(
        &mut self,
        provider_id: &str,
        model_id: &str,
        in_per_mtok: f64,
        out_per_mtok: f64,
        cached_in_per_mtok: Option<f64>,
    ) {
        match self
            .entries
            .iter_mut()
            .find(|e| e.provider_id == provider_id && e.model_id == model_id)
        {
            Some(e) => {
                e.in_per_mtok = in_per_mtok;
                e.out_per_mtok = out_per_mtok;
                e.cached_in_per_mtok = cached_in_per_mtok;
            }
            None => self.entries.push(ModelPrice {
                provider_id: provider_id.to_string(),
                model_id: model_id.to_string(),
                in_per_mtok,
                out_per_mtok,
                cached_in_per_mtok,
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
    /// An unmetered provider is [`Price::Free`] even if a rate was entered:
    /// the hardware is yours either way, and a stray entry must not start
    /// billing a local model.
    pub fn resolve(&self, ptype: ProviderType, provider_id: &str, model_id: &str) -> Price {
        if !ptype.is_metered() {
            return Price::Free;
        }
        match self.get(provider_id, model_id) {
            Some(e) => Price::Known {
                in_per_mtok: e.in_per_mtok,
                out_per_mtok: e.out_per_mtok,
                cached_in_per_mtok: e.cached_in_per_mtok,
            },
            None => Price::Unknown,
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
    fn ollama_cloud_is_metered_like_any_api() {
        let p = Pricing::default();
        assert_eq!(
            p.resolve(ProviderType::OllamaCloud, "ollama-cloud", "kimi-k2"),
            Price::Unknown
        );
        assert_eq!(
            p.resolve(ProviderType::Api, "kimi", "kimi-for-coding"),
            Price::Unknown
        );
    }

    #[test]
    fn a_rate_on_an_unmetered_provider_stays_free() {
        let mut p = Pricing::default();
        p.set("ollama-local", "qwen3:30b", 99.0, 99.0, Some(99.0));
        assert_eq!(
            p.resolve(ProviderType::OllamaLocal, "ollama-local", "qwen3:30b"),
            Price::Free
        );
    }

    #[test]
    fn set_then_resolve_returns_the_rate() {
        let mut p = Pricing::default();
        p.set("kimi", "kimi-for-coding", 3.0, 15.0, Some(0.30));
        assert_eq!(
            p.resolve(ProviderType::Api, "kimi", "kimi-for-coding"),
            Price::Known {
                in_per_mtok: 3.0,
                out_per_mtok: 15.0,
                cached_in_per_mtok: Some(0.30)
            }
        );
    }

    #[test]
    fn set_overwrites_rather_than_duplicating() {
        let mut p = Pricing::default();
        p.set("kimi", "k2", 3.0, 15.0, Some(0.30));
        p.set("kimi", "k2", 1.0, 2.0, None);
        assert_eq!(p.len(), 1);
        assert_eq!(p.get("kimi", "k2").unwrap().in_per_mtok, 1.0);
        // the cached rate is part of the overwrite, not merged with the old one
        assert_eq!(p.get("kimi", "k2").unwrap().cached_in_per_mtok, None);
    }

    #[test]
    fn prices_are_scoped_to_one_provider() {
        let mut p = Pricing::default();
        p.set("kimi", "shared-name", 3.0, 15.0, None);
        assert_eq!(
            p.resolve(ProviderType::Api, "other", "shared-name"),
            Price::Unknown
        );
    }

    #[test]
    fn clear_reports_whether_it_removed_anything() {
        let mut p = Pricing::default();
        p.set("kimi", "k2", 3.0, 15.0, None);
        assert!(p.clear("kimi", "k2"));
        assert!(!p.clear("kimi", "k2"));
        assert!(p.is_empty());
    }

    /// The rates on a Kimi K3 subscription page.
    fn k3() -> Price {
        Price::Known {
            in_per_mtok: 3.00,
            out_per_mtok: 15.00,
            cached_in_per_mtok: Some(0.30),
        }
    }

    #[test]
    fn cost_is_per_million_tokens() {
        let price = k3();
        assert_eq!(price.cost(1_000_000, 0, 0), 3.0);
        assert_eq!(price.cost(0, 0, 1_000_000), 15.0);
        assert_eq!(price.cost(500_000, 0, 100_000), 1.5 + 1.5);
    }

    #[test]
    fn cache_hits_bill_at_the_cached_rate() {
        // 1M prompt tokens, 800k of them cache hits, 100k output:
        //   200k fresh  @ $3.00  = $0.60
        //   800k cached @ $0.30  = $0.24
        //   100k out    @ $15.00 = $1.50
        let cost = k3().cost(1_000_000, 800_000, 100_000);
        assert!((cost - 2.34).abs() < 1e-9, "got {cost}");
    }

    #[test]
    fn a_fully_cached_prompt_costs_the_cached_rate_only() {
        assert_eq!(k3().cost(1_000_000, 1_000_000, 0), 0.30);
    }

    #[test]
    fn cached_tokens_are_a_subset_not_an_extra() {
        // Billing must never exceed the no-cache price for the same prompt.
        let no_cache = k3().cost(1_000_000, 0, 0);
        let cached = k3().cost(1_000_000, 400_000, 0);
        assert!(cached < no_cache);
    }

    #[test]
    fn an_overreported_cache_count_cannot_bill_negative_input() {
        // A provider claiming more cache hits than prompt tokens must not
        // produce a credit.
        let cost = k3().cost(1_000, 10_000, 0);
        assert!(cost > 0.0);
        assert_eq!(cost, k3().cost(1_000, 1_000, 0));
    }

    #[test]
    fn without_a_cached_rate_hits_bill_as_normal_input() {
        let p = Price::Known {
            in_per_mtok: 3.0,
            out_per_mtok: 15.0,
            cached_in_per_mtok: None,
        };
        assert_eq!(p.cost(1_000_000, 750_000, 0), p.cost(1_000_000, 0, 0));
    }

    #[test]
    fn free_and_unknown_both_cost_nothing() {
        assert_eq!(Price::Free.cost(1_000_000, 500_000, 1_000_000), 0.0);
        assert_eq!(Price::Unknown.cost(1_000_000, 500_000, 1_000_000), 0.0);
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
        assert_eq!(Price::Unknown.output_label(), "unknown");
        assert_eq!(Price::Unknown.cached_label(), "unknown");
        let k = Price::Known {
            in_per_mtok: 3.0,
            out_per_mtok: 0.075,
            cached_in_per_mtok: Some(0.30),
        };
        assert_eq!(k.input_label(), "$3.00");
        assert_eq!(k.output_label(), "$0.075");
        assert_eq!(k.cached_label(), "$0.30");
    }

    #[test]
    fn no_cached_rate_shows_the_input_rate_it_falls_back_to() {
        let k = Price::Known {
            in_per_mtok: 3.0,
            out_per_mtok: 15.0,
            cached_in_per_mtok: None,
        };
        assert_eq!(k.cached_label(), "$3.00");
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
        p.set("kimi", "k2", 3.0, 15.0, Some(0.19));
        p.set("openai", "gpt-4o", 2.5, 10.0, None);
        let text = toml::to_string_pretty(&PricingFile {
            prices: p.entries.clone(),
        })
        .unwrap();
        let back: PricingFile = toml::from_str(&text).unwrap();
        assert_eq!(back.prices, p.entries);
    }

    #[test]
    fn a_price_file_without_a_cached_rate_still_loads() {
        // Files written before cached pricing existed have no such key.
        let text = "[[prices]]\nprovider_id = \"kimi\"\nmodel_id = \"k2\"\nin_per_mtok = 3.0\nout_per_mtok = 15.0\n";
        let back: PricingFile = toml::from_str(text).unwrap();
        assert_eq!(back.prices[0].cached_in_per_mtok, None);
    }
}
