//! Rule engine (PRD FR-3): YAML-canonical rules, condition evaluation and
//! quota-aware fallback-chain walking. Pure and UI-free — context collection
//! (health, quotas, ledger) lives in `rolen-providers::routing`.

use crate::config;
use crate::error::CoreError;
use crate::types::{CmpOp, Condition, ConditionField, Rule};
use chrono::{DateTime, Timelike, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Built-in role catalog (FR-3.1). Users may define additional roles freely.
pub const BUILT_IN_ROLES: &[&str] = &[
    "planner",
    "summarizer",
    "coder",
    "tool-runner",
    "image-reader",
    "image-writer",
    "doc-reader",
    "doc-writer",
    "reviewer",
    "interrogator",
];

// --------------------------------------------------------------- RuleSet

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct RuleSet {
    #[serde(default)]
    pub rules: Vec<Rule>,
}

impl RuleSet {
    pub fn load() -> Result<Self, CoreError> {
        let file = config::rules_file()?;
        if !file.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(&file)?;
        serde_yaml::from_str(&text).map_err(|e| CoreError::Vault(format!("rules.yaml parse: {e}")))
    }

    pub fn save(&self) -> Result<(), CoreError> {
        config::ensure_dirs()?;
        let text = serde_yaml::to_string(self)
            .map_err(|e| CoreError::Vault(format!("rules.yaml serialize: {e}")))?;
        std::fs::write(config::rules_file()?, text)?;
        Ok(())
    }

    pub fn for_role(&self, role: &str) -> Vec<&Rule> {
        let mut v: Vec<&Rule> = self.rules.iter().filter(|r| r.role == role).collect();
        v.sort_by_key(|r| -r.priority);
        v
    }
}

// --------------------------------------------------------------- context

/// Live state of one provider, collected by `rolen-providers::routing`.
#[derive(Debug, Clone, Default)]
pub struct ProviderState {
    pub id: String,
    pub healthy: bool,
    /// Remaining quota in percent, if a plan limit is known (FR-4).
    pub quota_remaining_pct: Option<u8>,
    /// Cost accumulated in the current billing cycle.
    pub cost_so_far: f64,
    /// Discovered model ids (empty = unknown/CLI).
    pub models: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct EvalContext {
    pub task_type: Option<String>,
    pub project: Option<String>,
    pub now: Option<DateTime<Utc>>,
    pub providers: HashMap<String, ProviderState>,
}

// ----------------------------------------------------------- paused roles

/// FR-4.5 pause-role alert action: paused roles fail dispatch until resumed
/// (`rolen rule resume --role X`), instead of being rerouted to fallbacks.
mod paused {
    use crate::config;
    use crate::error::CoreError;
    use std::path::PathBuf;

    fn file() -> Result<PathBuf, CoreError> {
        Ok(config::data_dir()?.join("paused_roles.yaml"))
    }

    pub fn load() -> Vec<String> {
        let Ok(f) = file() else { return Vec::new() };
        std::fs::read_to_string(f)
            .ok()
            .and_then(|t| serde_yaml::from_str::<Vec<String>>(&t).ok())
            .unwrap_or_default()
    }

    pub fn is_paused(role: &str) -> bool {
        load().iter().any(|r| r == role)
    }

    pub fn set(role: &str, paused: bool) -> Result<(), CoreError> {
        let mut roles = load();
        roles.retain(|r| r != role);
        if paused {
            roles.push(role.to_string());
        }
        roles.sort();
        let text = serde_yaml::to_string(&roles)
            .map_err(|e| CoreError::Vault(format!("paused_roles serialize: {e}")))?;
        std::fs::write(file()?, text)?;
        Ok(())
    }
}

pub use paused::{is_paused as is_role_paused, set as set_role_paused};
pub fn paused_roles() -> Vec<String> {
    paused::load()
}

// --------------------------------------------------------------- decision

#[derive(Debug, Clone)]
pub struct Decision {
    pub rule_id: String,
    pub provider: String,
    pub model: String,
    pub explanation: String,
    /// Chain entries that were skipped, with reasons.
    pub skipped: Vec<(String, String)>,
}

#[derive(Debug, thiserror::Error)]
pub enum RuleError {
    #[error("no route for role '{role}': {reason}")]
    NoRoute { role: String, reason: String },
}

pub fn decide(rules: &RuleSet, role: &str, ctx: &EvalContext) -> Result<Decision, RuleError> {
    if is_role_paused(role) {
        return Err(RuleError::NoRoute {
            role: role.into(),
            reason: format!(
                "role paused by the quota alert action — resume with `rolen rule resume --role {role}`"
            ),
        });
    }
    let candidates = rules.for_role(role);
    if candidates.is_empty() {
        return Err(RuleError::NoRoute {
            role: role.into(),
            reason: "no rule matches this role (create one with `rolen rule add`)".into(),
        });
    }

    let mut notes: Vec<String> = Vec::new();
    for rule in candidates {
        // project scoping: a project-scoped rule only applies to its project
        if let Some(scope) = &rule.project_scope {
            if ctx.project.as_deref() != Some(scope) {
                notes.push(format!(
                    "rule '{}': skipped (project scope '{scope}')",
                    rule.id
                ));
                continue;
            }
        }
        match check_conditions(rule, ctx) {
            Ok(()) => match walk_chain(rule, ctx) {
                Ok(mut decision) => {
                    if !notes.is_empty() {
                        decision.explanation =
                            format!("{}\n  context: {}", decision.explanation, notes.join("; "));
                    }
                    return Ok(decision);
                }
                Err(reason) => notes.push(format!("rule '{}': {reason}", rule.id)),
            },
            Err(reason) => notes.push(format!("rule '{}': {reason}", rule.id)),
        }
    }

    Err(RuleError::NoRoute {
        role: role.into(),
        reason: notes.join("; "),
    })
}

fn check_conditions(rule: &Rule, ctx: &EvalContext) -> Result<(), String> {
    for c in &rule.conditions {
        eval_condition(c, ctx)?;
    }
    Ok(())
}

fn eval_condition(c: &Condition, ctx: &EvalContext) -> Result<(), String> {
    let pass = match c.field {
        ConditionField::TaskType => {
            cmp_str(ctx.task_type.as_deref().unwrap_or(""), &c.op, &c.value)
        }
        ConditionField::Project => cmp_str(ctx.project.as_deref().unwrap_or(""), &c.op, &c.value),
        ConditionField::TimeOfDay => {
            let now = ctx.now.unwrap_or_else(Utc::now);
            let hhmm = now.hour() as f64 + now.minute() as f64 / 60.0;
            let target: f64 = parse_hhmm(&c.value)?;
            cmp_f64(hhmm, &c.op, target)
        }
        ConditionField::ProviderHealth => {
            let pid = c.provider.as_deref().unwrap_or("");
            let healthy = ctx.providers.get(pid).map(|p| p.healthy).unwrap_or(false);
            let want = matches!(c.value.as_str(), "ok" | "healthy" | "true" | "1");
            cmp_bool(healthy, &c.op, want)
        }
        ConditionField::QuotaRemainingPct => {
            let pid = c.provider.as_deref().unwrap_or("");
            let pct = ctx
                .providers
                .get(pid)
                .and_then(|p| p.quota_remaining_pct)
                .ok_or_else(|| format!("quota of '{pid}' unknown (no plan limit set)"))?;
            let target: f64 = c
                .value
                .parse()
                .map_err(|_| format!("invalid quota threshold '{}'", c.value))?;
            cmp_f64(pct as f64, &c.op, target)
        }
        ConditionField::CostSoFar => {
            let pid = c.provider.as_deref().unwrap_or("");
            let cost = ctx
                .providers
                .get(pid)
                .map(|p| p.cost_so_far)
                .ok_or_else(|| format!("provider '{pid}' not registered"))?;
            let target: f64 = c
                .value
                .parse()
                .map_err(|_| format!("invalid cost threshold '{}'", c.value))?;
            cmp_f64(cost, &c.op, target)
        }
    };
    if pass {
        Ok(())
    } else {
        Err(format!(
            "condition not met: {:?} {:?} '{}'{}",
            c.field,
            c.op,
            c.value,
            c.provider
                .as_ref()
                .map(|p| format!(" (provider {p})"))
                .unwrap_or_default()
        ))
    }
}

fn walk_chain(rule: &Rule, ctx: &EvalContext) -> Result<Decision, String> {
    let mut skipped: Vec<(String, String)> = Vec::new();
    for entry in &rule.fallback_chain {
        let (pid, model) = entry
            .split_once('/')
            .map(|(p, m)| (p.trim(), m.trim()))
            .ok_or_else(|| format!("invalid chain entry '{entry}' (want provider/model)"))?;
        let Some(state) = ctx.providers.get(pid) else {
            skipped.push((entry.clone(), "provider not registered".into()));
            continue;
        };
        if !state.healthy {
            skipped.push((entry.clone(), "provider unhealthy".into()));
            continue;
        }
        if let Some(min) = rule.min_quota_pct.filter(|m| *m > 0) {
            match state.quota_remaining_pct {
                Some(pct) if pct < min => {
                    skipped.push((entry.clone(), format!("quota {pct}% < min {min}%")));
                    continue;
                }
                // Unknown quota (no plan limit set) is treated optimistically:
                // there is no evidence of exhaustion, so the entry stays usable.
                _ => {}
            }
        }
        if !state.models.is_empty() && !state.models.iter().any(|m| m == model) {
            skipped.push((
                entry.clone(),
                format!("model '{model}' not in provider catalog"),
            ));
            continue;
        }
        return Ok(Decision {
            rule_id: rule.id.clone(),
            provider: pid.to_string(),
            model: model.to_string(),
            explanation: format!("role via rule '{}' → {}/{}", rule.id, pid, model),
            skipped,
        });
    }
    Err(format!(
        "all {} chain entries unavailable",
        rule.fallback_chain.len()
    ))
}

// ------------------------------------------------------------------ helpers

fn cmp_str(a: &str, op: &CmpOp, b: &str) -> bool {
    match op {
        CmpOp::Eq => a == b,
        CmpOp::Ne => a != b,
        _ => false, // ordering ops make no sense for strings here
    }
}

fn cmp_bool(a: bool, op: &CmpOp, b: bool) -> bool {
    match op {
        CmpOp::Eq => a == b,
        CmpOp::Ne => a != b,
        _ => false,
    }
}

fn cmp_f64(a: f64, op: &CmpOp, b: f64) -> bool {
    match op {
        CmpOp::Lt => a < b,
        CmpOp::Le => a <= b,
        CmpOp::Eq => (a - b).abs() < f64::EPSILON,
        CmpOp::Ne => (a - b).abs() >= f64::EPSILON,
        CmpOp::Ge => a >= b,
        CmpOp::Gt => a > b,
    }
}

fn parse_hhmm(s: &str) -> Result<f64, String> {
    let (h, m) = s
        .split_once(':')
        .ok_or_else(|| format!("invalid time-of-day '{s}' (want HH:MM)"))?;
    let h: f64 = h.parse().map_err(|_| format!("invalid hour in '{s}'"))?;
    let m: f64 = m.parse().map_err(|_| format!("invalid minute in '{s}'"))?;
    Ok(h + m / 60.0)
}

// ------------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_with(providers: Vec<(&str, bool, Option<u8>, Vec<&str>)>) -> EvalContext {
        EvalContext {
            task_type: None,
            project: None,
            now: None,
            providers: providers
                .into_iter()
                .map(|(id, healthy, quota, models)| {
                    (
                        id.to_string(),
                        ProviderState {
                            id: id.into(),
                            healthy,
                            quota_remaining_pct: quota,
                            cost_so_far: 0.0,
                            models: models.into_iter().map(String::from).collect(),
                        },
                    )
                })
                .collect(),
        }
    }

    fn rule(id: &str, chain: Vec<&str>, min_quota: Option<u8>) -> Rule {
        Rule {
            id: id.into(),
            role: "coder".into(),
            conditions: vec![],
            fallback_chain: chain.into_iter().map(String::from).collect(),
            min_quota_pct: min_quota,
            priority: 0,
            project_scope: None,
        }
    }

    #[test]
    fn picks_first_healthy_entry() {
        let rules = RuleSet {
            rules: vec![rule("r1", vec!["a/m1", "b/m2"], None)],
        };
        let ctx = ctx_with(vec![
            ("a", true, None, vec!["m1"]),
            ("b", true, None, vec!["m2"]),
        ]);
        let d = decide(&rules, "coder", &ctx).unwrap();
        assert_eq!(d.provider, "a");
        assert_eq!(d.model, "m1");
    }

    #[test]
    fn falls_back_when_unhealthy() {
        let rules = RuleSet {
            rules: vec![rule("r1", vec!["a/m1", "b/m2"], None)],
        };
        let ctx = ctx_with(vec![
            ("a", false, None, vec!["m1"]),
            ("b", true, None, vec!["m2"]),
        ]);
        let d = decide(&rules, "coder", &ctx).unwrap();
        assert_eq!(d.provider, "b");
        assert_eq!(d.skipped.len(), 1);
        assert!(d.skipped[0].1.contains("unhealthy"));
    }

    #[test]
    fn quota_threshold_skips_exhausted_provider() {
        let rules = RuleSet {
            rules: vec![rule("r1", vec!["a/m1", "b/m2"], Some(20))],
        };
        let ctx = ctx_with(vec![
            ("a", true, Some(12), vec!["m1"]), // 12% < 20% min → skip
            ("b", true, Some(90), vec!["m2"]),
        ]);
        let d = decide(&rules, "coder", &ctx).unwrap();
        assert_eq!(d.provider, "b");
        assert!(d.skipped[0].1.contains("quota"));
    }

    #[test]
    fn unknown_quota_is_optimistic() {
        // with min_quota_pct set, a provider without plan info stays usable
        let rules = RuleSet {
            rules: vec![rule("r1", vec!["a/m1", "b/m2"], Some(20))],
        };
        let ctx = ctx_with(vec![
            ("a", true, Some(3), vec!["m1"]), // nearly exhausted → skip
            ("b", true, None, vec!["m2"]),    // unknown quota → allowed
        ]);
        let d = decide(&rules, "coder", &ctx).unwrap();
        assert_eq!(d.provider, "b");
    }

    #[test]
    fn priority_picks_higher_rule_first() {
        let mut low = rule("low", vec!["b/m2"], None);
        low.priority = 1;
        let mut high = rule("high", vec!["a/m1"], None);
        high.priority = 10;
        let rules = RuleSet {
            rules: vec![low, high],
        };
        let ctx = ctx_with(vec![
            ("a", true, None, vec!["m1"]),
            ("b", true, None, vec!["m2"]),
        ]);
        assert_eq!(decide(&rules, "coder", &ctx).unwrap().rule_id, "high");
    }

    #[test]
    fn condition_gates_rule() {
        let mut r = rule("night", vec!["a/m1"], None);
        r.conditions = vec![Condition {
            field: ConditionField::TaskType,
            op: CmpOp::Eq,
            value: "docs".into(),
            provider: None,
        }];
        let rules = RuleSet { rules: vec![r] };
        let mut ctx = ctx_with(vec![("a", true, None, vec!["m1"])]);
        // task_type "code" ≠ "docs" → no route
        ctx.task_type = Some("code".into());
        assert!(decide(&rules, "coder", &ctx).is_err());
        ctx.task_type = Some("docs".into());
        assert!(decide(&rules, "coder", &ctx).is_ok());
    }

    #[test]
    fn unknown_model_is_skipped() {
        let rules = RuleSet {
            rules: vec![rule("r1", vec!["a/nope", "a/m1"], None)],
        };
        let ctx = ctx_with(vec![("a", true, None, vec!["m1"])]);
        let d = decide(&rules, "coder", &ctx).unwrap();
        assert_eq!(d.model, "m1");
    }

    #[test]
    fn project_scope_filters() {
        let mut r = rule("scoped", vec!["a/m1"], None);
        r.project_scope = Some("shop".into());
        let rules = RuleSet { rules: vec![r] };
        let ctx = ctx_with(vec![("a", true, None, vec!["m1"])]);
        assert!(decide(&rules, "coder", &ctx).is_err()); // different/no project
    }
}
