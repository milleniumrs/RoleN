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
    /// USD per million input tokens (FR-1.5, P1)
    #[serde(default)]
    pub cost_in_per_mtok: Option<f64>,
    /// USD per million output tokens (FR-1.5, P1)
    #[serde(default)]
    pub cost_out_per_mtok: Option<f64>,
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
/// remote host). Maestro manages an `ssh -N -L` child process and points the
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
    /// Local loopback port Maestro forwards to.
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
    #[serde(default)]
    pub tokens_in: u64,
    #[serde(default)]
    pub tokens_out: u64,
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
