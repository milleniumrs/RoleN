//! Off-thread work.
//!
//! Nothing in the Maestro workspace is async: `reqwest` is used in blocking
//! mode, SQLite is synchronous, and detection spawns child processes. A health
//! sweep is a *serial* 30 s-timeout HTTP round trip per provider
//! (`maestro_providers::routing::collect`), so anything that talks to a
//! provider must leave the UI thread or the window stops repainting.
//!
//! The rule this module enforces: the UI never calls a blocking API directly.
//! It spawns a named job, keeps painting, and picks the result up on a later
//! frame. Jobs are keyed by a static name so a second click cannot start the
//! same sweep twice.

use std::collections::BTreeSet;
use std::sync::mpsc::{self, Receiver, Sender};

use eframe::egui;

/// One provider's answer to a health sweep.
#[derive(Debug, Clone)]
pub struct HealthRow {
    pub provider_id: String,
    pub ok: bool,
    pub latency_ms: u64,
    pub models: usize,
    pub detail: String,
}

/// Outcome of `providers::detect::detect_all` plus the registry write.
#[derive(Debug, Default, Clone)]
pub struct DetectReport {
    /// Provider ids that the scan found (whether or not they were already known).
    pub found: Vec<String>,
    /// How many of those were newly added to the registry.
    pub added: usize,
}

/// One line of `maestro_core::doctor::run_all`.
#[derive(Debug, Clone)]
pub struct CheckRow {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

/// The editable slice of `config.toml`.
///
/// Deliberately not the whole `Config`: `general.theme` belongs to the TUI and
/// `quotas.action` has no UI yet, so both are read back from disk at save time
/// and preserved. The TUI overwrites `quotas.action` with `Notify` on every
/// save (`maestro-tui/src/settings.rs:175`), silently discarding a
/// `switch-rule` or `pause-role` setting; this does not.
#[derive(Debug, Clone)]
pub struct ConfigForm {
    pub workspace_root: String,
    pub question_mode: maestro_core::types::QuestionMode,
    pub global_cap: usize,
    pub per_provider_cap: usize,
    pub warn_pct: u8,
    pub crit_pct: u8,
}

/// The `Providers > Add Provider` form.
#[derive(Debug, Clone)]
pub struct ProviderForm {
    pub id: String,
    pub ptype: maestro_core::types::ProviderType,
    pub endpoint: String,
    /// Typed in the dialog; stored in the OS keychain (or the age vault) on
    /// save and never written to `providers.toml`.
    pub key: String,
    /// Path to the agent binary for `cli` providers.
    pub cli_path: String,
    /// Filled in by a discovery job so saving does not have to fetch again.
    pub models: Vec<maestro_core::types::Model>,
}

impl Default for ProviderForm {
    fn default() -> Self {
        Self {
            id: String::new(),
            ptype: maestro_core::types::ProviderType::Api,
            endpoint: String::new(),
            key: String::new(),
            cli_path: String::new(),
            models: Vec::new(),
        }
    }
}

/// A finished job's payload.
#[derive(Debug)]
pub enum JobMsg {
    HealthChecked(Result<Vec<HealthRow>, String>),
    Detected(Result<DetectReport, String>),
    Doctor(Vec<CheckRow>),
    ProjectCreated(Result<String, String>),
    ConfigLoaded(Result<ConfigForm, String>),
    ConfigSaved(Result<(), String>),
    ModelsDiscovered(Result<Vec<maestro_core::types::Model>, String>),
    ProviderSaved(Result<String, String>),
    ProviderRemoved(Result<String, String>),
    /// Emitted before each provider is probed during a dry-run sweep.
    DryRunProgress {
        done: usize,
        total: usize,
        provider: String,
    },
    DryRun(Result<DryRun, String>),
    ChatReply(Result<maestro_providers::conversation::Turn, String>),
    /// A chunk of live PTY output from a wrapped CLI agent.
    CliOutput(String),
    CliHarvested {
        applied: usize,
        rejected: usize,
        paths: Vec<String>,
    },
    CliFinished(Result<CliReport, String>),
}

/// What a finished CLI session produced.
#[derive(Debug, Clone)]
pub struct CliReport {
    pub session_id: String,
    pub exit_code: Option<i32>,
    pub applied: usize,
    pub rejected: usize,
    pub paths: Vec<String>,
    pub transcript: std::path::PathBuf,
    pub tokens_in_est: u64,
    pub tokens_out_est: u64,
}

/// Run a task through a PTY-wrapped CLI agent, streaming its output.
///
/// This is the only genuinely streaming path in the workspace: the adapter
/// hands over stdout chunks at roughly 80 ms intervals, so the UI can show the
/// agent working instead of freezing until it exits. The TUI discards those
/// events entirely (`mission_control.rs:108` passes `&mut |_| {}`) and shows
/// only the final report.
pub fn run_cli_task(
    provider_id: String,
    task: String,
    workdir: std::path::PathBuf,
    emit: &Emitter,
) -> JobMsg {
    let reg = match maestro_providers::ProviderRegistry::load() {
        Ok(r) => r,
        Err(e) => return JobMsg::CliFinished(Err(e.to_string())),
    };
    let Some(provider) = reg.get(&provider_id).cloned() else {
        return JobMsg::CliFinished(Err(format!("provider '{provider_id}' is not registered")));
    };
    if provider.cli_path.is_none() {
        return JobMsg::CliFinished(Err(format!(
            "provider '{provider_id}' has no cli path, so there is nothing to run"
        )));
    }

    let mut on_event = |event: maestro_cliadapters::CliEvent| match event {
        maestro_cliadapters::CliEvent::Output(chunk) => {
            emit.progress(JobMsg::CliOutput(chunk));
        }
        maestro_cliadapters::CliEvent::Harvested {
            applied,
            rejected,
            paths,
        } => {
            emit.progress(JobMsg::CliHarvested {
                applied,
                rejected,
                paths,
            });
        }
    };

    // `None` for the queue: the adapter makes its own. Sharing one across
    // concurrent sessions is what preserves single-writer ordering, and will
    // matter once more than one session can run at a time.
    match maestro_cliadapters::run_cli_session(&provider, &task, &workdir, None, &mut on_event) {
        Ok(report) => JobMsg::CliFinished(Ok(CliReport {
            session_id: report.session_id,
            exit_code: report.exit_code,
            applied: report.applied,
            rejected: report.rejected,
            paths: report.paths,
            transcript: report.transcript_path,
            tokens_in_est: report.tokens_in_est,
            tokens_out_est: report.tokens_out_est,
        })),
        Err(e) => JobMsg::CliFinished(Err(e.to_string())),
    }
}

/// Output cap for a chat reply. `ChatRequest::single` caps at 256, which is a
/// health-probe budget, not an answer.
pub const MAX_REPLY_TOKENS: u32 = 2048;

/// Send one turn of a conversation. The history must already end with the
/// message the user just typed.
pub fn chat_turn(
    provider_id: String,
    model: String,
    history: Vec<maestro_providers::chat::ChatMessage>,
    session_id: String,
    prior: maestro_providers::conversation::Totals,
) -> JobMsg {
    match maestro_providers::conversation::send(
        &provider_id,
        &model,
        history,
        &session_id,
        prior,
        MAX_REPLY_TOKENS,
    ) {
        Ok(turn) => JobMsg::ChatReply(Ok(turn)),
        Err(e) => JobMsg::ChatReply(Err(e.to_string())),
    }
}

/// What a rule dry-run concluded.
#[derive(Debug, Clone)]
pub enum DryRun {
    Decided {
        role: String,
        rule_id: String,
        provider: String,
        model: String,
        explanation: String,
        /// Chain entries that were passed over, with the reason.
        skipped: Vec<(String, String)>,
    },
    NoRoute {
        role: String,
        reason: String,
    },
    Cancelled,
}

/// Job names. Static strings so the in-flight set needs no allocation and the
/// UI can ask "is this particular job running?" without stringly-typed guesses.
pub const HEALTH_CHECK: &str = "health-check";
pub const DETECT: &str = "detect";
pub const DOCTOR: &str = "doctor";
pub const NEW_PROJECT: &str = "new-project";
pub const LOAD_CONFIG: &str = "load-config";
pub const SAVE_CONFIG: &str = "save-config";
pub const DRY_RUN: &str = "dry-run";
pub const CHAT: &str = "chat";
pub const CLI_TASK: &str = "cli-task";
pub const DISCOVER_MODELS: &str = "discover-models";
pub const SAVE_PROVIDER: &str = "save-provider";
pub const REMOVE_PROVIDER: &str = "remove-provider";

/// Tracks in-flight background work and delivers results to the UI thread.
/// Lets a long job report progress before it finishes.
///
/// Needed because the only progress the core can offer for a routing sweep is
/// "which provider am I on", and that is worth showing when the worst case is
/// 30 s per provider.
pub struct Emitter {
    name: &'static str,
    tx: Sender<(&'static str, JobMsg, bool)>,
    ctx: egui::Context,
}

impl Emitter {
    pub fn progress(&self, msg: JobMsg) {
        let _ = self.tx.send((self.name, msg, false));
        self.ctx.request_repaint();
    }
}

pub struct Jobs {
    ctx: egui::Context,
    tx: Sender<(&'static str, JobMsg, bool)>,
    rx: Receiver<(&'static str, JobMsg, bool)>,
    running: BTreeSet<&'static str>,
}

impl Jobs {
    pub fn new(ctx: egui::Context) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            ctx,
            tx,
            rx,
            running: BTreeSet::new(),
        }
    }

    /// The egui context, for code that needs it while handling a result.
    pub fn ctx(&self) -> &egui::Context {
        &self.ctx
    }

    /// Is this specific job in flight?
    pub fn is_running(&self, name: &'static str) -> bool {
        self.running.contains(name)
    }

    /// Names of everything currently in flight, for the status bar.
    pub fn running(&self) -> impl Iterator<Item = &&'static str> {
        self.running.iter()
    }

    /// Start `work` on its own thread unless a job of the same name is already
    /// running. Returns false when the call was suppressed as a duplicate.
    pub fn spawn<F>(&mut self, name: &'static str, work: F) -> bool
    where
        F: FnOnce() -> JobMsg + Send + 'static,
    {
        self.spawn_streaming(name, move |_| work())
    }

    /// Like [`Jobs::spawn`], but the closure may emit progress messages before
    /// returning its result.
    pub fn spawn_streaming<F>(&mut self, name: &'static str, work: F) -> bool
    where
        F: FnOnce(&Emitter) -> JobMsg + Send + 'static,
    {
        if !self.running.insert(name) {
            return false;
        }
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("maestro-gui:{name}"))
            .spawn(move || {
                let emitter = Emitter {
                    name,
                    tx: tx.clone(),
                    ctx: ctx.clone(),
                };
                let msg = work(&emitter);
                // If the receiver is gone the window is closing; dropping the
                // result is the correct behaviour.
                let _ = tx.send((name, msg, true));
                // Wake the UI thread: egui is not repainting continuously when
                // idle, so without this the result would sit unnoticed.
                ctx.request_repaint();
            });
        if spawned.is_err() {
            self.running.remove(name);
            return false;
        }
        true
    }

    /// Collect everything delivered since the last frame. A job stays in the
    /// running set until it sends its final message.
    pub fn drain(&mut self) -> Vec<JobMsg> {
        let mut out = Vec::new();
        while let Ok((name, msg, finished)) = self.rx.try_recv() {
            if finished {
                self.running.remove(name);
            }
            out.push(msg);
        }
        out
    }
}

/// Health-sweep body. Runs on a worker thread; may take 30 s per unreachable
/// provider because `client::health` inherits the 30 s discovery timeout.
pub fn health_check_all() -> JobMsg {
    let reg = match maestro_providers::ProviderRegistry::load() {
        Ok(r) => r,
        Err(e) => return JobMsg::HealthChecked(Err(e.to_string())),
    };
    let rows = reg
        .list()
        .iter()
        .map(|p| {
            // CLI providers are PTY-wrapped binaries, not HTTP endpoints, so a
            // health probe would be meaningless rather than merely slow.
            if p.ptype == maestro_core::types::ProviderType::Cli {
                return HealthRow {
                    provider_id: p.id.clone(),
                    ok: p.cli_path.is_some(),
                    latency_ms: 0,
                    models: p.models.len(),
                    detail: match &p.cli_path {
                        Some(path) => format!("cli: {}", path.display()),
                        None => "cli: no path registered".to_string(),
                    },
                };
            }
            let h = maestro_providers::client::health(p);
            HealthRow {
                provider_id: p.id.clone(),
                ok: h.ok,
                latency_ms: h.latency_ms,
                models: h.models,
                detail: h.detail,
            }
        })
        .collect();
    JobMsg::HealthChecked(Ok(rows))
}

/// Detection body: probes local Ollama over HTTP and shells out to
/// `where`/`which` for the known CLI agents, then merges the result into the
/// registry. Seconds, not milliseconds - hence a job.
pub fn detect_and_register() -> JobMsg {
    let found = maestro_providers::detect::detect_all();
    let mut reg = match maestro_providers::ProviderRegistry::load() {
        Ok(r) => r,
        Err(e) => return JobMsg::Detected(Err(e.to_string())),
    };
    let mut report = DetectReport::default();
    for mut p in found {
        report.found.push(p.id.clone());
        if reg.get(&p.id).is_none() {
            report.added += 1;
        }
        // Discovering models needs another HTTP call, and only makes sense for
        // endpoints - a CLI agent has no /models to list.
        if p.ptype != maestro_core::types::ProviderType::Cli && p.models.is_empty() {
            if let Ok(models) = maestro_providers::client::list_models(&p) {
                p.models = models;
            }
        }
        reg.upsert(p);
    }
    match reg.save() {
        Ok(()) => JobMsg::Detected(Ok(report)),
        Err(e) => JobMsg::Detected(Err(e.to_string())),
    }
}

/// Environment diagnostics. Never fails as a whole - each check reports its
/// own verdict. Off-thread because it does a full OS-keychain write/read/delete
/// roundtrip plus a SQLite probe, which is hundreds of milliseconds on Windows.
pub fn doctor() -> JobMsg {
    let rows = maestro_core::doctor::run_all()
        .into_iter()
        .map(|c| CheckRow {
            name: c.name.to_string(),
            ok: c.ok,
            detail: c.detail,
        })
        .collect();
    JobMsg::Doctor(rows)
}

/// Scaffold a project: create the directory, write the manifest, `git init`.
///
/// Fast (~100 ms) but it spawns a child process, so it still belongs off the
/// UI thread. The clarification interview is deliberately *not* run here - that
/// is a multi-minute LLM call and needs its own progress/cancel design.
pub fn scaffold_project(name: String, description: String, stack: String) -> JobMsg {
    let cfg = match maestro_core::config::Config::ensure() {
        Ok((c, _created)) => c,
        Err(e) => return JobMsg::ProjectCreated(Err(e.to_string())),
    };
    if let Err(e) = cfg.ensure_workspace_root() {
        return JobMsg::ProjectCreated(Err(e.to_string()));
    }
    let stack: Vec<String> = stack
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    match maestro_core::project::scaffold(&name, &description, stack, &cfg.general.workspace_root) {
        Ok((meta, dir)) => {
            JobMsg::ProjectCreated(Ok(format!("created '{}' at {}", meta.id, dir.display())))
        }
        Err(e) => JobMsg::ProjectCreated(Err(e.to_string())),
    }
}

/// Read `config.toml` for the settings form.
///
/// `Config::load` returns an error when the file does not exist rather than a
/// default, so `ensure` is used: a fresh install gets a real file written and
/// the form shows what is actually on disk.
pub fn load_config() -> JobMsg {
    match maestro_core::config::Config::ensure() {
        Ok((c, _created)) => JobMsg::ConfigLoaded(Ok(ConfigForm {
            workspace_root: c.general.workspace_root.display().to_string(),
            question_mode: c.general.question_mode,
            global_cap: c.parallelism.global_cap,
            per_provider_cap: c.parallelism.per_provider_cap,
            warn_pct: c.quotas.warn_pct,
            crit_pct: c.quotas.crit_pct,
        })),
        Err(e) => JobMsg::ConfigLoaded(Err(e.to_string())),
    }
}

/// Write the form back, preserving every field the form does not own.
pub fn save_config(form: ConfigForm) -> JobMsg {
    let mut cfg = match maestro_core::config::Config::ensure() {
        Ok((c, _created)) => c,
        Err(e) => return JobMsg::ConfigSaved(Err(e.to_string())),
    };
    if !form.workspace_root.trim().is_empty() {
        cfg.general.workspace_root = std::path::PathBuf::from(form.workspace_root.trim());
    }
    cfg.general.question_mode = form.question_mode;
    cfg.parallelism.global_cap = form.global_cap;
    cfg.parallelism.per_provider_cap = form.per_provider_cap;
    cfg.quotas.warn_pct = form.warn_pct;
    cfg.quotas.crit_pct = form.crit_pct;
    match cfg.save() {
        Ok(()) => JobMsg::ConfigSaved(Ok(())),
        Err(e) => JobMsg::ConfigSaved(Err(e.to_string())),
    }
}

/// Build the in-memory provider a form describes. No IO.
fn provider_from(form: &ProviderForm) -> maestro_core::types::Provider {
    use maestro_core::types::{Provider, ProviderType};
    let endpoint = form.endpoint.trim();
    let endpoint = if endpoint.is_empty() {
        // Ollama has well-known bases, so an empty field is a sensible default
        // rather than an error.
        match form.ptype {
            ProviderType::OllamaLocal => Some(maestro_providers::ollama::DEFAULT_LOCAL_BASE.into()),
            ProviderType::OllamaCloud => Some(maestro_providers::ollama::DEFAULT_CLOUD_BASE.into()),
            _ => None,
        }
    } else {
        Some(endpoint.to_string())
    };
    Provider {
        id: form.id.trim().to_string(),
        ptype: form.ptype,
        auth: Default::default(),
        tunnel: None,
        endpoint,
        cli_path: if form.cli_path.trim().is_empty() {
            None
        } else {
            Some(std::path::PathBuf::from(form.cli_path.trim()))
        },
        key_ref: None,
        models: form.models.clone(),
    }
}

/// Ask the endpoint what models it serves, using the key that is still only in
/// the dialog. One HTTP round trip at a 30 s timeout.
pub fn discover_models(form: ProviderForm) -> JobMsg {
    let provider = provider_from(&form);
    let key = form.key.trim();
    let key = (!key.is_empty()).then_some(key);
    match maestro_providers::client::list_models_with_key(&provider, key) {
        Ok(models) => JobMsg::ModelsDiscovered(Ok(models)),
        Err(e) => JobMsg::ModelsDiscovered(Err(e.to_string())),
    }
}

/// Store the key, then upsert the provider into the registry.
///
/// Order matters: if the secret cannot be stored the registry is left alone,
/// so a provider never ends up recorded with a `key_ref` pointing at nothing.
pub fn save_provider(form: ProviderForm) -> JobMsg {
    let mut provider = provider_from(&form);
    let key = form.key.trim();
    if !key.is_empty() {
        let key_ref = maestro_providers::registry::key_ref_for(&provider.id);
        if let Err(e) = maestro_core::secrets::set_secret(&key_ref, key) {
            return JobMsg::ProviderSaved(Err(format!("could not store the key: {e}")));
        }
        provider.key_ref = Some(key_ref);
    }

    let mut reg = match maestro_providers::ProviderRegistry::load() {
        Ok(r) => r,
        Err(e) => return JobMsg::ProviderSaved(Err(e.to_string())),
    };
    // Editing an existing provider must not drop a key that was stored earlier
    // but not retyped now.
    if provider.key_ref.is_none() {
        if let Some(existing) = reg.get(&provider.id) {
            provider.key_ref = existing.key_ref.clone();
        }
    }
    let id = provider.id.clone();
    let models = provider.models.len();
    reg.upsert(provider);
    match reg.save() {
        Ok(()) => JobMsg::ProviderSaved(Ok(format!("saved '{id}' with {models} model(s)"))),
        Err(e) => JobMsg::ProviderSaved(Err(e.to_string())),
    }
}

/// Evaluate the routing rules for `role` against live provider state.
///
/// The expensive part is the routing sweep, not the decision: `decide` is pure
/// and instant, while collecting the context health-checks every provider. So
/// the cancel flag is threaded into the sweep and the role is only resolved
/// once real state is in hand.
pub fn dry_run(
    role: String,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    emit: &Emitter,
) -> JobMsg {
    use std::sync::atomic::Ordering;

    let rules = match maestro_core::rules::RuleSet::load() {
        Ok(r) => r,
        Err(e) => return JobMsg::DryRun(Err(format!("could not read rules.yaml: {e}"))),
    };

    let is_cancelled = || cancel.load(Ordering::Relaxed);
    let mut on_provider = |done: usize, total: usize, provider: &str| {
        emit.progress(JobMsg::DryRunProgress {
            done,
            total,
            provider: provider.to_string(),
        });
    };

    let ctx = match maestro_providers::routing::collect_cancellable(
        None,
        None,
        &is_cancelled,
        &mut on_provider,
    ) {
        Ok(Some(ctx)) => ctx,
        Ok(None) => return JobMsg::DryRun(Ok(DryRun::Cancelled)),
        Err(e) => return JobMsg::DryRun(Err(e.to_string())),
    };

    match maestro_core::rules::decide(&rules, &role, &ctx) {
        Ok(d) => JobMsg::DryRun(Ok(DryRun::Decided {
            role,
            rule_id: d.rule_id,
            provider: d.provider,
            model: d.model,
            explanation: d.explanation,
            skipped: d.skipped,
        })),
        Err(maestro_core::rules::RuleError::NoRoute { role, reason }) => {
            JobMsg::DryRun(Ok(DryRun::NoRoute { role, reason }))
        }
    }
}

/// Drop a provider from the registry and delete its stored key.
pub fn remove_provider(id: String) -> JobMsg {
    let mut reg = match maestro_providers::ProviderRegistry::load() {
        Ok(r) => r,
        Err(e) => return JobMsg::ProviderRemoved(Err(e.to_string())),
    };
    let key_ref = reg.get(&id).and_then(|p| p.key_ref.clone());
    if !reg.remove(&id) {
        return JobMsg::ProviderRemoved(Err(format!("provider '{id}' is not registered")));
    }
    if let Err(e) = reg.save() {
        return JobMsg::ProviderRemoved(Err(e.to_string()));
    }
    // Best effort: a leftover secret is untidy but not a failure of the removal.
    let mut note = String::new();
    if let Some(kref) = key_ref {
        if maestro_core::secrets::delete_secret(&kref).is_err() {
            note = " (stored key could not be deleted)".to_string();
        }
    }
    JobMsg::ProviderRemoved(Ok(format!("removed '{id}'{note}")))
}
