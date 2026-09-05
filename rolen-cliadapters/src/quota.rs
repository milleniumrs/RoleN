//! FR-4.2: quota probing for CLI providers. Runs the adapter's configured
//! `quota_args` as a plain subprocess (no PTY needed for a printout) and
//! parses `quota_regex` out of its output.

use crate::spec::CliSpec;
use anyhow::{anyhow, Context};

/// Run the CLI's quota probe and parse `(used, limit)` from its output.
/// Named regex groups `used`/`limit` win; otherwise capture groups 1 and 2.
pub fn cli_quota(provider: &rolen_core::types::Provider) -> anyhow::Result<(u64, Option<u64>)> {
    let spec = CliSpec::for_provider(provider)
        .ok_or_else(|| anyhow!("provider '{}' has no cli_path", provider.id))?;
    let (args, pattern) = match (spec.quota_args, spec.quota_regex) {
        (Some(a), Some(r)) => (a, r),
        _ => {
            return Err(anyhow!(
                "no quota probe configured for '{}' — add quota_args/quota_regex to its adapter in cli-adapters.toml",
                provider.id
            ))
        }
    };
    let out = std::process::Command::new(&spec.program)
        .args(&args)
        .output()
        .with_context(|| format!("running {}", spec.program.display()))?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    parse_quota_output(&text, &pattern)
        .ok_or_else(|| anyhow!("quota output did not match regex '{pattern}':\n{text}"))
}

/// Parse `(used, limit)` from CLI output. Numbers may contain `_` or `,`
/// separators. Public for tests.
pub fn parse_quota_output(text: &str, pattern: &str) -> Option<(u64, Option<u64>)> {
    let re = regex::Regex::new(pattern).ok()?;
    let caps = re.captures(text)?;
    let parse = |s: &str| s.replace(['_', ','], "").parse::<u64>().ok();
    let used = caps
        .name("used")
        .or_else(|| caps.get(1))
        .and_then(|m| parse(m.as_str()));
    let limit = caps
        .name("limit")
        .or_else(|| caps.get(2))
        .and_then(|m| parse(m.as_str()));
    used.map(|u| (u, limit))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_named_groups() {
        let text = "Plan usage: used 12_345 of 100_000 tokens this cycle";
        let (used, limit) =
            parse_quota_output(text, r"used (?P<used>[0-9_]+) of (?P<limit>[0-9_]+)").unwrap();
        assert_eq!(used, 12345);
        assert_eq!(limit, Some(100000));
    }

    #[test]
    fn parses_positional_groups_and_commas() {
        let text = "remaining quota: 98,765 / 200,000";
        let (used, limit) = parse_quota_output(text, r"([0-9,]+) / ([0-9,]+)").unwrap();
        assert_eq!(used, 98765);
        assert_eq!(limit, Some(200000));
    }

    #[test]
    fn no_match_is_none() {
        assert!(parse_quota_output("nothing here", r"used (\d+)").is_none());
        assert!(parse_quota_output("used 5", "(broken").is_none());
    }
}
