//! Domain types — mirrors PRD §8 (data model).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ---------------------------------------------------------------- providers

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderType {
    Api,
    Cli,
    OllamaLocal,
    OllamaCloud,
    /// Ollama server reached through an SSH tunnel (Provider.tunnel required).
    OllamaRemote,
}

/// How a provider charges, which decides what a price even means for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Billing {
    /// Runs on hardware you already pay for. No per-token charge.
    Free,
    /// Charged per token at a published rate.
    PerToken,
    /// A subscription or plan allowance. No per-token rate exists to look up:
    /// spend is the monthly fee, and the binding constraint is the allowance,
    /// not the price of any one call.
    Subscription,
}

impl ProviderType {
    /// How this kind of provider charges.
    ///
    /// A local Ollama server, and one reached over an SSH tunnel, both run on
    /// your own hardware, so there is nothing to bill per token.
    ///
    /// Ollama's hosted cloud and PTY-wrapped CLI agents are subscriptions:
    /// they publish no per-token rate, meter an opaque compute allowance, and
    /// queue or refuse rather than bill once it is spent. That is a different
    /// thing from a metered provider whose rate nobody has entered yet.
    pub fn billing(self) -> Billing {
        match self {
            Self::OllamaLocal | Self::OllamaRemote => Billing::Free,
            Self::Api => Billing::PerToken,
            Self::Cli | Self::OllamaCloud => Billing::Subscription,
        }
    }
}

/// One model in a provider's catalogue. Discovered models are rebuilt from the
/// provider's API on every refresh, so nothing durable belongs here — prices
/// live in `rolen-core::pricing`, keyed by provider and model id.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    #[serde(default)]
    pub context_tokens: Option<u32>,
    #[serde(default)]
    pub vision: bool,
    #[serde(default)]
    pub tools: bool,
    #[serde(default)]
    pub streaming: bool,
    /// Added by hand rather than found by discovery.
    ///
    /// Providers routinely serve models their `/models` endpoint never lists.
    /// A manual entry survives refresh so those stay usable and priced; a
    /// discovered one is replaced by whatever the API currently says.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub manual: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthKind {
    /// Plain API key (x-api-key / bearer depending on the API flavor).
    #[default]
    Key,
    /// OAuth subscription tokens (access/refresh) stored as a JSON secret.
    OAuth,
}

/// SSH port-forward to reach a remote Ollama server (e.g. inside docker on a
/// remote host). RoleN manages an `ssh -N -L` child process and points the
/// provider at the local forwarded port.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelSpec {
    pub user: String,
    pub host: String,
    #[serde(default = "default_ssh_port")]
    pub port: u16,
    /// Remote side of the forward, from the SSH server's perspective.
    #[serde(default = "default_remote_host")]
    pub remote_host: String,
    #[serde(default = "default_ollama_port")]
    pub remote_port: u16,
    /// Local loopback port RoleN forwards to.
    #[serde(default = "default_tunnel_local_port")]
    pub local_port: u16,
    /// Explicit identity file; default ssh key resolution (~/.ssh) otherwise.
    #[serde(default)]
    pub identity_file: Option<PathBuf>,
}

fn default_ssh_port() -> u16 {
    22
}
fn default_remote_host() -> String {
    "localhost".into()
}
fn default_ollama_port() -> u16 {
    11434
}
fn default_tunnel_local_port() -> u16 {
    11435
}

impl TunnelSpec {
    /// Parse "user@host[:port]".
    pub fn parse(spec: &str) -> Result<Self, String> {
        let (user, rest) = spec
            .split_once('@')
            .ok_or_else(|| format!("tunnel spec '{spec}' must look like user@host[:port]"))?;
        if user.is_empty() {
            return Err("tunnel spec: empty user".into());
        }
        let (host, port) = match rest.split_once(':') {
            Some((h, p)) => (
                h,
                p.parse::<u16>()
                    .map_err(|_| format!("tunnel spec: invalid port '{p}'"))?,
            ),
            None => (rest, default_ssh_port()),
        };
        if host.is_empty() {
            return Err("tunnel spec: empty host".into());
        }
        Ok(Self {
            user: user.to_string(),
            host: host.to_string(),
            port,
            remote_host: default_remote_host(),
            remote_port: default_ollama_port(),
            local_port: default_tunnel_local_port(),
            identity_file: None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provider {
    pub id: String,
    #[serde(rename = "type")]
    pub ptype: ProviderType,
    #[serde(default)]
    pub auth: AuthKind,
    #[serde(default)]
    pub tunnel: Option<TunnelSpec>,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub cli_path: Option<PathBuf>,
    /// Reference into the OS keychain / vault — never the secret itself (FR-2.1)
    #[serde(default)]
    pub key_ref: Option<String>,
    #[serde(default)]
    pub models: Vec<Model>,
}

// ----------------------------------------------------------- subscriptions

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QuotaSource {
    Api,
    Parsed,
    Manual,
    Estimated,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    pub provider_id: String,
    #[serde(default)]
    pub plan_limit: Option<u64>,
    #[serde(default)]
    pub used: u64,
    #[serde(default)]
    pub cycle_start: Option<DateTime<Utc>>,
    #[serde(default)]
    pub renewal: Option<DateTime<Utc>>,
    pub source: QuotaSource,
}

// ------------------------------------------------------------------ rules

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConditionField {
    QuotaRemainingPct,
    CostSoFar,
    TaskType,
    Project,
    TimeOfDay,
    ProviderHealth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CmpOp {
    Lt,
    Le,
    Eq,
    Ne,
    Ge,
    Gt,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Condition {
    pub field: ConditionField,
    pub op: CmpOp,
    pub value: String,
    /// Provider a quota/health condition applies to, if relevant.
    #[serde(default)]
    pub provider: Option<String>,
}

/// Declarative routing rule (PRD §3, FR-3.2). Canonical on-disk format is
/// YAML (decision D2); this is the in-memory representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub id: String,
    pub role: String,
    #[serde(default)]
    pub conditions: Vec<Condition>,
    /// Ordered "provider/model" fallbacks, e.g. ["kimi/kimi-k2.7", "glm/glm-5.2"]
    pub fallback_chain: Vec<String>,
    /// Chain entries whose provider has less remaining quota than this are
    /// skipped (None/0 = only skip when fully exhausted).
    #[serde(default)]
    pub min_quota_pct: Option<u8>,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub project_scope: Option<String>,
}

// --------------------------------------------------------------- projects

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskState {
    Pending,
    Blocked,
    Running,
    Paused,
    Done,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub project_id: String,
    pub role: String,
    pub title: String,
    #[serde(default)]
    pub deps: Vec<String>,
    /// Files this task owns — enforced by the orchestrator (FR-7.5)
    #[serde(default)]
    pub claimed_paths: Vec<PathBuf>,
    pub state: TaskState,
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub dir: PathBuf,
    #[serde(default)]
    pub prd_json: Option<serde_json::Value>,
    #[serde(default)]
    pub agents_md_hash: Option<String>,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub tasks: Vec<Task>,
    #[serde(default)]
    pub rules_override: Vec<Rule>,
}

// --------------------------------------------------------------- sessions

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionState {
    Starting,
    Running,
    Paused,
    Migrating,
    Done,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub task_id: Option<String>,
    pub provider_id: String,
    pub model: String,
    pub state: SessionState,
    #[serde(default)]
    pub tokens_in: u64,
    #[serde(default)]
    pub tokens_out: u64,
    #[serde(default)]
    pub cost: f64,
    pub started: DateTime<Utc>,
    #[serde(default)]
    pub transcript_path: Option<PathBuf>,
}

// ----------------------------------------------------------- write tickets

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WriteOp {
    Create,
    Patch,
    Replace,
    Delete,
    Rename,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TicketState {
    Queued,
    Applied,
    Rejected,
}

/// Atomic file-mutation request — the ONLY way agents write files (FR-7.1/7.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteTicket {
    pub id: String,
    pub task_id: String,
    pub path: PathBuf,
    pub op: WriteOp,
    /// Full content (create/replace), unified diff (patch), target path (rename)
    /// or empty (delete).
    pub payload: String,
    /// Hash of the file version the agent read (FR-7.4 optimistic concurrency).
    #[serde(default)]
    pub base_hash: Option<String>,
    pub state: TicketState,
    pub ts: DateTime<Utc>,
}

// ----------------------------------------------------------------- ledger

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub id: String,
    pub session_id: String,
    pub provider_id: String,
    /// Token counts split into the buckets the price list bills. Rows written
    /// before cache pricing existed read their cache buckets back as 0, which
    /// bills them exactly as they were billed before.
    #[serde(default, flatten)]
    pub usage: crate::pricing::Tokens,
    #[serde(default)]
    pub cost: f64,
    #[serde(default)]
    pub latency_ms: Option<u64>,
    pub ts: DateTime<Utc>,
}

// ---------------------------------------------------------- clarifications

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClarificationStatus {
    Pending,
    Answered,
    Deferred,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Clarification {
    pub id: String,
    pub project_id: String,
    pub question: String,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub answer: Option<String>,
    pub status: ClarificationStatus,
    #[serde(default)]
    pub linked_prd_path: Option<String>,
    pub ts: DateTime<Utc>,
}

// ------------------------------------------------------------------ config

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QuestionMode {
    Thorough,
    Balanced,
    Minimal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AlertAction {
    Notify,
    SwitchRule,
    PauseRole,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_your_own_hardware_is_free() {
        assert_eq!(ProviderType::OllamaLocal.billing(), Billing::Free);
        assert_eq!(ProviderType::OllamaRemote.billing(), Billing::Free);
    }

    #[test]
    fn subscriptions_are_not_per_token_providers() {
        // Neither publishes a per-token rate: a wrapped CLI agent bills its
        // own subscription, and Ollama Cloud meters a compute allowance.
        assert_eq!(ProviderType::Cli.billing(), Billing::Subscription);
        assert_eq!(ProviderType::OllamaCloud.billing(), Billing::Subscription);
    }

    #[test]
    fn a_plain_api_is_charged_per_token() {
        assert_eq!(ProviderType::Api.billing(), Billing::PerToken);
    }
}
