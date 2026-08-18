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

/// A finished job's payload.
#[derive(Debug)]
pub enum JobMsg {
    HealthChecked(Result<Vec<HealthRow>, String>),
    Detected(Result<DetectReport, String>),
    Doctor(Vec<CheckRow>),
    ProjectCreated(Result<String, String>),
    ConfigLoaded(Result<ConfigForm, String>),
    ConfigSaved(Result<(), String>),
}

/// Job names. Static strings so the in-flight set needs no allocation and the
/// UI can ask "is this particular job running?" without stringly-typed guesses.
pub const HEALTH_CHECK: &str = "health-check";
pub const DETECT: &str = "detect";
pub const DOCTOR: &str = "doctor";
pub const NEW_PROJECT: &str = "new-project";
pub const LOAD_CONFIG: &str = "load-config";
pub const SAVE_CONFIG: &str = "save-config";

/// Tracks in-flight background work and delivers results to the UI thread.
pub struct Jobs {
    ctx: egui::Context,
    tx: Sender<(&'static str, JobMsg)>,
    rx: Receiver<(&'static str, JobMsg)>,
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
        if !self.running.insert(name) {
            return false;
        }
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("maestro-gui:{name}"))
            .spawn(move || {
                let msg = work();
                // If the receiver is gone the window is closing; dropping the
                // result is the correct behaviour.
                let _ = tx.send((name, msg));
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

    /// Collect everything that finished since the last frame.
    pub fn drain(&mut self) -> Vec<JobMsg> {
        let mut out = Vec::new();
        while let Ok((name, msg)) = self.rx.try_recv() {
            self.running.remove(name);
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
