//! The background poller and the snapshot it produces.
//!
//! The TUI refreshes by doing its reads inline on a 3 s timer: four separate
//! `Ledger::open_default()` calls - each of which re-runs the schema DDL - plus
//! a YAML parse of every project, all on the UI thread
//! (`rolen-tui/src/mission_control.rs:1092`). That is the single worst
//! pattern in the existing front-end and this module exists to not repeat it.
//!
//! Instead one worker thread owns a *persistent* `Ledger` connection, builds an
//! immutable [`Snapshot`] on an interval, and hands it to the UI. The UI never
//! touches SQLite. Opening the ledger once per process instead of ~80 times a
//! minute is most of the win.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use rolen_core::ledger::Ledger;

use crate::wake::Wake;

/// Some reads are much dearer than the ledger aggregates and change slowly:
/// quota costs one extra SQLite open per provider, and the project scan is a
/// directory walk plus a YAML parse per project. Both run every Nth cycle
/// rather than every tick - or immediately when something asked for a refresh.
const SLOW_EVERY: u64 = 10;

#[derive(Debug, Clone, Copy, Default)]
pub struct Usage {
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost: f64,
    pub requests: u64,
}

impl Usage {
    pub fn total_tokens(&self) -> u64 {
        self.tokens_in + self.tokens_out
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Tickets {
    pub applied: u64,
    pub rejected: u64,
    pub queued: u64,
}

/// A provider as the UI wants to show it - flattened, no borrow of the registry.
#[derive(Debug, Clone)]
pub struct ProviderRow {
    pub id: String,
    pub kind: String,
    pub endpoint: Option<String>,
    pub models: usize,
    /// Model ids, for the chat picker.
    pub model_ids: Vec<String>,
    pub has_key: bool,
    pub is_cli: bool,
    pub tokens_today: u64,
    pub cost_today: f64,
    pub quota_pct: Option<u8>,
}

/// A project in the workspace root, flattened for display.
#[derive(Debug, Clone)]
pub struct ProjectRow {
    pub id: String,
    pub name: String,
    pub dir: PathBuf,
    pub description: String,
    pub stack: Vec<String>,
    pub has_prd: bool,
    pub has_agents: bool,
    pub clarifications: usize,
    pub pending: usize,
    pub skills: usize,
}

/// A routing rule as the table shows it.
#[derive(Debug, Clone)]
pub struct RuleRow {
    pub id: String,
    pub role: String,
    pub priority: i32,
    pub chain: Vec<String>,
    pub conditions: usize,
    pub min_quota_pct: Option<u8>,
    pub project_scope: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SessionRow {
    pub id: String,
    pub provider_id: String,
    pub model: String,
    pub state: String,
    pub tokens: u64,
    pub cost: f64,
    pub started: DateTime<Utc>,
    pub transcript: Option<PathBuf>,
}

/// One consistent read of everything the dashboard and provider views need.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub generated: Option<DateTime<Utc>>,
    pub providers: Vec<ProviderRow>,
    pub projects: Vec<ProjectRow>,
    pub rules: Vec<RuleRow>,
    pub workspace_root: Option<PathBuf>,
    pub sessions: Vec<SessionRow>,
    pub today: Usage,
    pub tickets: Tickets,
    pub running_sessions: u64,
    /// Non-fatal problems hit while collecting. Shown rather than swallowed:
    /// a dashboard reading zeroes because the ledger failed to open should say so.
    pub problems: Vec<String>,
}

impl Snapshot {
    pub fn total_models(&self) -> usize {
        self.providers.iter().map(|p| p.models).sum()
    }

    /// Clarifications still awaiting an answer, across all projects.
    pub fn pending_questions(&self) -> usize {
        self.projects.iter().map(|p| p.pending).sum()
    }
}

/// Handle to the polling thread.
pub struct Poller {
    rx: Receiver<Snapshot>,
    refresh: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Poller {
    pub fn spawn(wake: Wake, interval: Duration) -> Self {
        let (tx, rx) = mpsc::channel();
        let refresh = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let handle = {
            let refresh = Arc::clone(&refresh);
            let stop = Arc::clone(&stop);
            std::thread::Builder::new()
                .name("rolen-gui:poller".to_string())
                .spawn(move || worker(&wake, &tx, &refresh, &stop, interval))
                .ok()
        };
        Self {
            rx,
            refresh,
            stop,
            handle,
        }
    }

    /// A poller that never produces anything and starts no thread.
    ///
    /// Used by the headless render tests so they neither touch the real ledger
    /// nor create config/data directories as a side effect.
    pub fn inert() -> Self {
        let (_tx, rx) = mpsc::channel();
        Self {
            rx,
            refresh: Arc::new(AtomicBool::new(false)),
            stop: Arc::new(AtomicBool::new(true)),
            handle: None,
        }
    }

    /// Take the newest snapshot, discarding any that piled up while the window
    /// was hidden. Returns `None` when nothing new has arrived.
    pub fn latest(&self) -> Option<Snapshot> {
        let mut newest = None;
        while let Ok(s) = self.rx.try_recv() {
            newest = Some(s);
        }
        newest
    }

    /// Ask for an out-of-band refresh (after a write, or on user request).
    pub fn refresh_now(&self) {
        self.refresh.store(true, Ordering::Relaxed);
    }
}

impl Drop for Poller {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn worker(
    wake: &Wake,
    tx: &mpsc::Sender<Snapshot>,
    refresh: &Arc<AtomicBool>,
    stop: &Arc<AtomicBool>,
    interval: Duration,
) {
    // Owned by this thread for its whole life. `Ledger` is Send but not Sync,
    // which is exactly the shape this design wants.
    let mut ledger: Option<Ledger> = None;
    let mut quota: HashMap<String, Option<u8>> = HashMap::new();
    let mut projects: Vec<ProjectRow> = Vec::new();
    let mut rules: Vec<RuleRow> = Vec::new();
    let mut workspace_root: Option<PathBuf> = None;
    let mut cycle: u64 = 0;
    // The first pass must populate the slow data, and an explicit refresh
    // (e.g. right after a project is created) must not wait for the cadence.
    let mut force_slow = true;

    while !stop.load(Ordering::Relaxed) {
        let mut problems = Vec::new();

        if ledger.is_none() {
            match Ledger::open_default() {
                Ok(l) => ledger = Some(l),
                Err(e) => problems.push(format!("ledger unavailable: {e}")),
            }
        }

        let registry = match rolen_providers::ProviderRegistry::load() {
            Ok(r) => Some(r),
            Err(e) => {
                problems.push(format!("provider registry: {e}"));
                None
            }
        };

        if force_slow || cycle.is_multiple_of(SLOW_EVERY) {
            force_slow = false;
            if let Some(reg) = registry.as_ref() {
                quota = reg
                    .list()
                    .iter()
                    .map(|p| (p.id.clone(), rolen_providers::routing::remaining_pct(&p.id)))
                    .collect();
            }
            let root = rolen_core::config::Config::load()
                .map(|c| c.general.workspace_root)
                .unwrap_or_default();
            projects = scan_projects(&root);
            workspace_root = Some(root);
            // `RuleSet::load` yields an empty default when rules.yaml is
            // absent, so a missing file is not a problem worth reporting.
            rules = rolen_core::rules::RuleSet::load()
                .map(|set| {
                    set.rules
                        .into_iter()
                        .map(|r| RuleRow {
                            id: r.id,
                            role: r.role,
                            priority: r.priority,
                            chain: r.fallback_chain,
                            conditions: r.conditions.len(),
                            min_quota_pct: r.min_quota_pct,
                            project_scope: r.project_scope,
                        })
                        .collect()
                })
                .unwrap_or_default();
        }

        let mut snapshot = collect(registry.as_ref(), &mut ledger, &quota, problems);
        snapshot.projects = projects.clone();
        snapshot.rules = rules.clone();
        snapshot.workspace_root = workspace_root.clone();
        if tx.send(snapshot).is_err() {
            return; // UI is gone
        }
        wake();
        cycle = cycle.wrapping_add(1);

        // Sleep in slices so stop/refresh are honoured promptly instead of
        // after a full interval.
        let deadline = Instant::now() + interval;
        while Instant::now() < deadline {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            if refresh.swap(false, Ordering::Relaxed) {
                // An explicit refresh means something changed on disk, so the
                // slow data is stale too - rescan it on the next pass.
                force_slow = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

/// Walk the workspace root for projects.
///
/// `list_projects` reads the directory and parses a YAML manifest per project;
/// the TUI does this on the UI thread every 3 s
/// (`rolen-tui/src/mission_control.rs:854`). Here it is on the poller thread
/// and only on the slow cadence.
fn scan_projects(root: &Path) -> Vec<ProjectRow> {
    rolen_core::project::list_projects(root)
        .into_iter()
        .map(|(dir, meta)| {
            let pending = meta
                .clarifications
                .iter()
                .filter(|c| !matches!(c.status, rolen_core::types::ClarificationStatus::Answered))
                .count();
            ProjectRow {
                id: meta.id,
                name: meta.name,
                description: meta.description,
                stack: meta.stack,
                has_prd: dir.join("PRD.md").exists(),
                has_agents: dir.join("AGENTS.md").exists(),
                clarifications: meta.clarifications.len(),
                pending,
                skills: meta.skills.len(),
                dir,
            }
        })
        .collect()
}

fn collect(
    registry: Option<&rolen_providers::ProviderRegistry>,
    ledger: &mut Option<Ledger>,
    quota: &HashMap<String, Option<u8>>,
    mut problems: Vec<String>,
) -> Snapshot {
    let mut snap = Snapshot {
        generated: Some(Utc::now()),
        ..Default::default()
    };

    // `usage_today` measures from UTC midnight (rolen-core/src/ledger.rs:279);
    // ticket counts use the same boundary so the dashboard is internally consistent.
    let day_start = Utc::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .map(|d| DateTime::<Utc>::from_naive_utc_and_offset(d, Utc).to_rfc3339());

    // A failed query usually means the connection went bad; drop it so the next
    // cycle reopens rather than repeating the same error forever.
    let mut lost_ledger = false;

    if let Some(l) = ledger.as_ref() {
        match l.usage_today(None) {
            Ok(u) => {
                snap.today = Usage {
                    tokens_in: u.tokens_in,
                    tokens_out: u.tokens_out,
                    cost: u.cost,
                    requests: u.requests,
                }
            }
            Err(e) => {
                problems.push(format!("usage: {e}"));
                lost_ledger = true;
            }
        }

        if let Some(since) = day_start.as_deref() {
            match l.ticket_counts_since(since) {
                Ok((applied, rejected, queued)) => {
                    snap.tickets = Tickets {
                        applied,
                        rejected,
                        queued,
                    }
                }
                Err(e) => {
                    problems.push(format!("write tickets: {e}"));
                    lost_ledger = true;
                }
            }
        }

        match l.count_sessions_by_state("running") {
            Ok(n) => snap.running_sessions = n,
            Err(e) => {
                problems.push(format!("sessions: {e}"));
                lost_ledger = true;
            }
        }

        match l.recent_sessions(12) {
            Ok(sessions) => {
                snap.sessions = sessions
                    .into_iter()
                    .map(|s| SessionRow {
                        id: s.id,
                        provider_id: s.provider_id,
                        model: s.model,
                        state: format!("{:?}", s.state).to_lowercase(),
                        tokens: s.tokens_in + s.tokens_out,
                        cost: s.cost,
                        started: s.started,
                        transcript: s.transcript_path,
                    })
                    .collect()
            }
            Err(e) => {
                problems.push(format!("recent sessions: {e}"));
                lost_ledger = true;
            }
        }
    }

    if let Some(reg) = registry {
        snap.providers = reg
            .list()
            .iter()
            .map(|p| {
                let usage = ledger
                    .as_ref()
                    .and_then(|l| l.usage_today(Some(&p.id)).ok())
                    .unwrap_or_default();
                ProviderRow {
                    id: p.id.clone(),
                    kind: format!("{:?}", p.ptype).to_lowercase(),
                    endpoint: p.endpoint.clone(),
                    models: p.models.len(),
                    model_ids: p.models.iter().map(|m| m.id.clone()).collect(),
                    has_key: p.key_ref.is_some(),
                    is_cli: p.ptype == rolen_core::types::ProviderType::Cli,
                    tokens_today: usage.tokens_in + usage.tokens_out,
                    cost_today: usage.cost,
                    quota_pct: quota.get(&p.id).copied().flatten(),
                }
            })
            .collect();
    }

    if lost_ledger {
        *ledger = None;
    }
    snap.problems = problems;
    snap
}
