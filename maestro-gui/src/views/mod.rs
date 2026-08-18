//! Screens. Each view is a plain function over `(&mut MaestroApp, &mut Ui)` so
//! the app owns all state and the views stay free of their own caches - the
//! snapshot is the single source of truth for anything read from disk.

pub mod dashboard;
pub mod projects;
pub mod providers;
pub mod rules;

/// Compact token counts: 45 / 12.3k / 1.5M.
pub fn fmt_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// `$0.0000` for small amounts, `$12.34` once it is worth rounding.
pub fn fmt_cost(cost: f64) -> String {
    if cost > 0.0 && cost < 0.01 {
        format!("${cost:.4}")
    } else {
        format!("${cost:.2}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_switch_units_at_the_right_boundaries() {
        assert_eq!(fmt_tokens(0), "0");
        assert_eq!(fmt_tokens(999), "999");
        assert_eq!(fmt_tokens(1_000), "1.0k");
        assert_eq!(fmt_tokens(12_345), "12.3k");
        assert_eq!(fmt_tokens(999_999), "1000.0k");
        assert_eq!(fmt_tokens(1_000_000), "1.0M");
        assert_eq!(fmt_tokens(2_500_000), "2.5M");
    }

    /// Sub-cent spend is the normal case for a single request; rounding it to
    /// `$0.00` would make the dashboard look broken.
    #[test]
    fn small_costs_keep_their_precision() {
        assert_eq!(fmt_cost(0.0), "$0.00");
        assert_eq!(fmt_cost(0.0042), "$0.0042");
        assert_eq!(fmt_cost(0.01), "$0.01");
        assert_eq!(fmt_cost(12.345), "$12.35");
    }
}
