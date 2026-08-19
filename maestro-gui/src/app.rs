//! The application shell: navigation, status bar, and the wiring between the
//! poller, the job system and the views.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use eframe::egui;

use crate::dialogs::{
    AddProviderDialog, CliTaskDialog, NewProjectDialog, ProviderRequest, SettingsDialog,
};
use crate::jobs::{self, CheckRow, DryRun, HealthRow, JobMsg, Jobs};
use crate::menu::{self, Action};
use crate::state::{Poller, Snapshot};
use crate::views;

/// How often the background poller rebuilds its snapshot. Cheap now that a
/// single SQLite connection is held open for the life of the thread.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Dashboard,
    Providers,
    Projects,
    Rules,
    Questions,
    Activity,
    Chat,
}

impl View {
    const ALL: [View; 7] = [
        View::Dashboard,
        View::Providers,
        View::Projects,
        View::Rules,
        View::Questions,
        View::Activity,
        View::Chat,
    ];

    fn label(self) -> &'static str {
        match self {
            View::Dashboard => "Dashboard",
            View::Providers => "Providers",
            View::Projects => "Projects",
            View::Rules => "Rules",
            View::Questions => "Questions",
            View::Activity => "Activity",
            View::Chat => "Chat",
        }
    }
}

pub struct MaestroApp {
    pub view: View,
    pub jobs: Jobs,
    pub poller: Poller,
    pub snap: Snapshot,
    /// Health results keyed by provider id. Deliberately *not* in the snapshot:
    /// a health sweep is user-initiated and expensive, so it must not be
    /// silently re-run by the poller.
    pub health: HashMap<String, HealthRow>,
    pub selected_provider: Option<String>,
    pub selected_project: Option<String>,
    pub new_project: NewProjectDialog,
    pub settings: SettingsDialog,
    pub add_provider: AddProviderDialog,
    /// Role the dry-run will evaluate.
    pub dry_run_role: String,
    pub dry_run_result: Option<DryRun>,
    pub dry_run_progress: Option<String>,
    /// Cooperative stop for the routing sweep. Checked between providers - an
    /// HTTP request already in flight cannot be aborted.
    dry_run_cancel: Arc<AtomicBool>,
    pub chat_provider: Option<String>,
    pub chat_model: Option<String>,
    /// The conversation exactly as it is sent to the provider.
    pub chat_history: Vec<maestro_providers::chat::ChatMessage>,
    pub chat_input: String,
    pub chat_session: String,
    pub chat_totals: maestro_providers::conversation::Totals,
    pub cli_task: CliTaskDialog,
    /// Live agent output, ANSI already stripped.
    pub cli_output: String,
    pub cli_report: Option<jobs::CliReport>,
    /// `Some` while the doctor report modal is showing.
    doctor: Option<Vec<CheckRow>>,
    about_open: bool,
    status: String,
}

impl MaestroApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let ctx = cc.egui_ctx.clone();
        let poller = Poller::spawn(ctx.clone(), POLL_INTERVAL);
        Self::with_poller(ctx, poller)
    }

    /// An app with no polling thread, for rendering the UI without a window.
    ///
    /// The TUI verifies its palettes by rendering offscreen rather than
    /// guessing (`maestro-tui/src/lib.rs:35`); this is the same idea for the
    /// GUI - `egui::Context::run` builds the whole widget tree with no
    /// windowing system, so the views can be proven to lay out.
    pub fn headless(ctx: eframe::egui::Context) -> Self {
        Self::with_poller(ctx, Poller::inert())
    }

    fn with_poller(ctx: eframe::egui::Context, poller: Poller) -> Self {
        Self {
            view: View::Dashboard,
            jobs: Jobs::new(ctx),
            poller,
            snap: Snapshot::default(),
            health: HashMap::new(),
            selected_provider: None,
            selected_project: None,
            new_project: NewProjectDialog::default(),
            settings: SettingsDialog::default(),
            add_provider: AddProviderDialog::default(),
            dry_run_role: "coder".to_string(),
            dry_run_result: None,
            dry_run_progress: None,
            dry_run_cancel: Arc::new(AtomicBool::new(false)),
            chat_provider: None,
            chat_model: None,
            chat_history: Vec::new(),
            chat_input: String::new(),
            chat_session: maestro_providers::conversation::new_session_id(),
            chat_totals: maestro_providers::conversation::Totals::default(),
            cli_task: CliTaskDialog::default(),
            cli_output: String::new(),
            cli_report: None,
            doctor: None,
            about_open: false,
            status: "ready".to_string(),
        }
    }

    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status = msg.into();
    }

    /// Kick off a routing dry-run for the selected role.
    pub fn start_dry_run(&mut self) {
        // A fresh flag per run: reusing one that was already set would cancel
        // the new sweep immediately.
        self.dry_run_cancel = Arc::new(AtomicBool::new(false));
        self.dry_run_result = None;
        self.dry_run_progress = Some("starting...".to_string());
        let role = self.dry_run_role.clone();
        let cancel = Arc::clone(&self.dry_run_cancel);
        self.jobs
            .spawn_streaming(jobs::DRY_RUN, move |emit| jobs::dry_run(role, cancel, emit));
    }

    /// Ids of cli providers that can actually be run.
    pub fn runnable_cli_providers(&self) -> Vec<String> {
        self.snap
            .providers
            .iter()
            .filter(|p| p.is_cli)
            .map(|p| p.id.clone())
            .collect()
    }

    pub fn open_cli_task(&mut self) {
        let providers = self.runnable_cli_providers();
        let workdir = self
            .snap
            .workspace_root
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        self.cli_task.open(&providers, workdir);
    }

    /// Append a chunk of agent output.
    ///
    /// The buffer is capped: a chatty agent can emit megabytes, and egui lays
    /// the whole label out every frame.
    fn append_cli_output(&mut self, chunk: &str, ctx: &egui::Context) {
        const CAP: usize = 200_000;
        self.cli_output
            .push_str(&crate::text::strip_ansi(&crate::text::renderable(
                ctx, chunk,
            )));
        if self.cli_output.len() > CAP {
            // Drop from the front on a char boundary, keeping the tail.
            let cut = self.cli_output.len() - CAP;
            let cut = (cut..self.cli_output.len())
                .find(|i| self.cli_output.is_char_boundary(*i))
                .unwrap_or(self.cli_output.len());
            self.cli_output.drain(..cut);
        }
    }

    /// Fold a finished turn back into the conversation.
    ///
    /// On success the reply joins the history, which is what makes the next
    /// turn multi-turn at all. On failure the unanswered user message is
    /// removed: leaving it would send two user messages in a row, a shape some
    /// providers reject outright.
    pub fn apply_chat_reply(
        &mut self,
        reply: Result<maestro_providers::conversation::Turn, String>,
    ) {
        match reply {
            Ok(turn) => {
                self.chat_totals.tokens_in += turn.tokens_in;
                self.chat_totals.tokens_out += turn.tokens_out;
                self.chat_totals.cost += turn.cost;
                self.set_status(format!(
                    "{} in / {} out · {} ms",
                    turn.tokens_in, turn.tokens_out, turn.latency_ms
                ));
                self.chat_history
                    .push(maestro_providers::chat::ChatMessage::assistant(
                        turn.text.trim(),
                    ));
                self.poller.refresh_now();
            }
            Err(e) => {
                self.chat_history.pop();
                self.set_status(format!("chat failed: {e} (message not kept in history)"));
            }
        }
    }

    /// Start a fresh conversation: new history and a new ledger session, so
    /// the old one keeps its own totals.
    pub fn reset_chat(&mut self) {
        self.chat_history.clear();
        self.chat_totals = maestro_providers::conversation::Totals::default();
        self.chat_session = maestro_providers::conversation::new_session_id();
        self.set_status("new chat");
    }

    /// Append the typed message and send the whole conversation.
    pub fn send_chat(&mut self) {
        let (Some(provider), Some(model)) = (self.chat_provider.clone(), self.chat_model.clone())
        else {
            self.set_status("pick a provider and model first");
            return;
        };
        let prompt = self.chat_input.trim().to_string();
        if prompt.is_empty() {
            return;
        }
        self.chat_input.clear();
        self.chat_history
            .push(maestro_providers::chat::ChatMessage::user(prompt));

        let history = self.chat_history.clone();
        let session = self.chat_session.clone();
        let prior = self.chat_totals;
        self.set_status(format!(
            "asking {provider}/{model} ({} turns of context)",
            history.len()
        ));
        self.jobs.spawn(jobs::CHAT, move || {
            jobs::chat_turn(provider, model, history, session, prior)
        });
    }

    pub fn cancel_dry_run(&mut self) {
        self.dry_run_cancel.store(true, Ordering::Relaxed);
        self.dry_run_progress = Some("cancelling after the current provider...".to_string());
    }

    fn handle_jobs(&mut self) {
        for msg in self.jobs.drain() {
            match msg {
                JobMsg::HealthChecked(Ok(rows)) => {
                    let ok = rows.iter().filter(|r| r.ok).count();
                    let total = rows.len();
                    for r in rows {
                        self.health.insert(r.provider_id.clone(), r);
                    }
                    self.set_status(format!("health check: {ok}/{total} reachable"));
                }
                JobMsg::HealthChecked(Err(e)) => {
                    self.set_status(format!("health check failed: {e}"))
                }
                JobMsg::Detected(Ok(report)) => {
                    self.set_status(format!(
                        "detected {} provider(s), {} newly registered",
                        report.found.len(),
                        report.added
                    ));
                    // The registry changed on disk; do not wait out the interval.
                    self.poller.refresh_now();
                }
                JobMsg::Detected(Err(e)) => self.set_status(format!("detection failed: {e}")),
                JobMsg::Doctor(rows) => {
                    let failed = rows.iter().filter(|r| !r.ok).count();
                    self.set_status(if failed == 0 {
                        "config doctor: all checks passed".to_string()
                    } else {
                        format!("config doctor: {failed} check(s) failed")
                    });
                    self.doctor = Some(rows);
                }
                JobMsg::ProjectCreated(Ok(msg)) => {
                    self.set_status(msg);
                    self.view = View::Projects;
                    self.poller.refresh_now();
                }
                JobMsg::ProjectCreated(Err(e)) => {
                    self.set_status(format!("could not create the project: {e}"))
                }
                JobMsg::ConfigLoaded(Ok(form)) => self.settings.populate(form),
                JobMsg::ConfigLoaded(Err(e)) => {
                    self.set_status(format!("could not read config.toml: {e}"))
                }
                JobMsg::ConfigSaved(Ok(())) => {
                    self.set_status("settings saved");
                    // workspace_root may have moved, so the project scan is stale.
                    self.poller.refresh_now();
                }
                JobMsg::ConfigSaved(Err(e)) => {
                    self.set_status(format!("could not save settings: {e}"))
                }
                JobMsg::ModelsDiscovered(Ok(models)) => {
                    self.set_status(format!("discovered {} model(s)", models.len()));
                    self.add_provider.discovered(models);
                }
                JobMsg::ModelsDiscovered(Err(e)) => {
                    self.set_status(format!("discovery failed: {e}"));
                    self.add_provider.failed(e);
                }
                JobMsg::ProviderSaved(Ok(msg)) => {
                    self.set_status(msg);
                    self.add_provider.close();
                    self.view = View::Providers;
                    self.poller.refresh_now();
                }
                JobMsg::ProviderSaved(Err(e)) => {
                    self.set_status(format!("could not save the provider: {e}"));
                    self.add_provider.failed(e);
                }
                JobMsg::ProviderRemoved(Ok(msg)) => {
                    self.set_status(msg);
                    self.selected_provider = None;
                    self.poller.refresh_now();
                }
                JobMsg::ProviderRemoved(Err(e)) => {
                    self.set_status(format!("could not remove the provider: {e}"))
                }
                JobMsg::DryRunProgress {
                    done,
                    total,
                    provider,
                } => {
                    self.dry_run_progress =
                        Some(format!("checking {provider} ({}/{total})", done + 1));
                }
                JobMsg::DryRun(Ok(outcome)) => {
                    self.dry_run_progress = None;
                    self.set_status(match &outcome {
                        DryRun::Decided {
                            role,
                            provider,
                            model,
                            ..
                        } => format!("{role} routes to {provider}/{model}"),
                        DryRun::NoRoute { role, .. } => format!("no route for '{role}'"),
                        DryRun::Cancelled => "dry-run cancelled".to_string(),
                    });
                    self.dry_run_result = Some(outcome);
                }
                JobMsg::DryRun(Err(e)) => {
                    self.dry_run_progress = None;
                    self.set_status(format!("dry-run failed: {e}"));
                }
                JobMsg::ChatReply(reply) => self.apply_chat_reply(reply),
                JobMsg::CliOutput(chunk) => {
                    let ctx = self.jobs.ctx().clone();
                    self.append_cli_output(&chunk, &ctx);
                }
                JobMsg::CliHarvested {
                    applied,
                    rejected,
                    paths,
                } => {
                    self.set_status(format!(
                        "harvested {applied} write(s), {rejected} rejected across {} path(s)",
                        paths.len()
                    ));
                }
                JobMsg::CliFinished(Ok(report)) => {
                    self.set_status(format!(
                        "session {} finished (exit {})",
                        report.session_id,
                        report
                            .exit_code
                            .map(|c| c.to_string())
                            .unwrap_or_else(|| "unknown".into())
                    ));
                    self.cli_report = Some(report);
                    self.poller.refresh_now();
                }
                JobMsg::CliFinished(Err(e)) => {
                    self.set_status(format!("cli session failed: {e}"));
                }
            }
        }
    }

    /// Route a menu selection (or its keyboard shortcut) to real work.
    fn dispatch(&mut self, action: Action, ctx: &egui::Context) {
        match action {
            Action::NewProject => self.new_project.open(),
            Action::Doctor => {
                self.jobs.spawn(jobs::DOCTOR, jobs::doctor);
            }
            Action::Exit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Action::AddProvider => {
                self.add_provider.open();
                self.view = View::Providers;
            }
            Action::DetectClis => {
                self.jobs.spawn(jobs::DETECT, jobs::detect_and_register);
                self.view = View::Providers;
            }
            Action::HealthCheck => {
                self.jobs.spawn(jobs::HEALTH_CHECK, jobs::health_check_all);
                self.view = View::Providers;
            }
            Action::Settings => {
                // Read the file off-thread, then open the dialog when it lands.
                self.jobs.spawn(jobs::LOAD_CONFIG, jobs::load_config);
            }
            Action::DryRun => {
                self.view = View::Rules;
                self.start_dry_run();
            }
            Action::QuickChat => self.view = View::Chat,
            Action::RunCliTask => self.open_cli_task(),
            Action::Theme(pref) => ctx.set_theme(pref),
            Action::About => self.about_open = true,
        }
    }

    /// Shortcuts that match the TUI's bindings.
    fn shortcut(&self, ctx: &egui::Context) -> Option<Action> {
        use egui::{Key, KeyboardShortcut, Modifiers};
        const NEW: KeyboardShortcut = KeyboardShortcut::new(Modifiers::CTRL, Key::N);
        const NEW_ALT: KeyboardShortcut =
            KeyboardShortcut::new(Modifiers::CTRL.plus(Modifiers::SHIFT), Key::N);
        const DOCTOR: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::F9);
        const SETTINGS: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::F10);
        const DRY_RUN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::CTRL, Key::D);
        const CHAT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::CTRL, Key::Q);
        const ABOUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::F1);

        ctx.input_mut(|i| {
            if i.consume_shortcut(&NEW) || i.consume_shortcut(&NEW_ALT) {
                Some(Action::NewProject)
            } else if i.consume_shortcut(&DOCTOR) {
                Some(Action::Doctor)
            } else if i.consume_shortcut(&SETTINGS) {
                Some(Action::Settings)
            } else if i.consume_shortcut(&DRY_RUN) {
                Some(Action::DryRun)
            } else if i.consume_shortcut(&CHAT) {
                Some(Action::QuickChat)
            } else if i.consume_shortcut(&ABOUT) {
                Some(Action::About)
            } else {
                None
            }
        })
    }

    fn modals(&mut self, ctx: &egui::Context) {
        if let Some(draft) = self.new_project.show(ctx) {
            self.jobs.spawn(jobs::NEW_PROJECT, move || {
                jobs::scaffold_project(draft.name, draft.description, draft.stack)
            });
        }

        if let Some(form) = self.settings.show(ctx) {
            self.jobs
                .spawn(jobs::SAVE_CONFIG, move || jobs::save_config(form));
        }

        let cli_providers = self.runnable_cli_providers();
        if let Some(req) = self.cli_task.show(ctx, &cli_providers) {
            self.cli_output.clear();
            self.cli_report = None;
            self.view = View::Activity;
            self.set_status(format!(
                "starting {} in {}",
                req.provider,
                req.workdir.display()
            ));
            self.jobs.spawn_streaming(jobs::CLI_TASK, move |emit| {
                jobs::run_cli_task(req.provider, req.task, req.workdir, emit)
            });
        }

        match self.add_provider.show(ctx) {
            Some(ProviderRequest::Discover(form)) => {
                self.jobs
                    .spawn(jobs::DISCOVER_MODELS, move || jobs::discover_models(form));
            }
            Some(ProviderRequest::Save(form)) => {
                self.jobs
                    .spawn(jobs::SAVE_PROVIDER, move || jobs::save_provider(form));
            }
            None => {}
        }

        let mut close_doctor = false;
        if let Some(rows) = &self.doctor {
            let response = egui::Modal::new(egui::Id::new("doctor-modal")).show(ctx, |ui| {
                ui.set_width(560.0);
                ui.heading("Config doctor");
                ui.add_space(8.0);
                for row in rows {
                    ui.horizontal(|ui| {
                        if row.ok {
                            ui.colored_label(egui::Color32::from_rgb(0x2e, 0x7d, 0x32), "OK");
                        } else {
                            ui.colored_label(egui::Color32::from_rgb(0xc6, 0x28, 0x28), "FAIL");
                        }
                        ui.strong(&row.name);
                        ui.label(&row.detail);
                    });
                }
                ui.add_space(10.0);
                if ui.button("Close").clicked() {
                    close_doctor = true;
                }
            });
            if response.should_close() {
                close_doctor = true;
            }
        }
        if close_doctor {
            self.doctor = None;
        }

        if self.about_open {
            let response = egui::Modal::new(egui::Id::new("about-modal")).show(ctx, |ui| {
                ui.set_width(380.0);
                ui.heading("Maestro");
                ui.add_space(6.0);
                ui.label(format!("version {}", env!("CARGO_PKG_VERSION")));
                ui.label("A conductor for LLM-powered development.");
                ui.label("MIT License");
                ui.add_space(10.0);
                if ui.button("Close").clicked() {
                    self.about_open = false;
                }
            });
            if response.should_close() {
                self.about_open = false;
            }
        }
    }
}

impl eframe::App for MaestroApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.draw(ui);
    }
}

impl MaestroApp {
    /// The whole frame, independent of `eframe::Frame` so it can be driven by
    /// `egui::Context::run_ui` in a test.
    pub fn draw(&mut self, ui: &mut egui::Ui) {
        self.handle_jobs();
        if let Some(snap) = self.poller.latest() {
            self.snap = snap;
        }

        let ctx = ui.ctx().clone();
        let mut action = self.shortcut(&ctx);
        egui::Panel::top("menu").show(ui, |ui| {
            if let Some(picked) = menu::show(ui) {
                action = Some(picked);
            }
        });
        if let Some(action) = action {
            self.dispatch(action, &ctx);
        }
        self.modals(&ctx);

        egui::Panel::left("nav")
            .resizable(false)
            .exact_size(160.0)
            .show(ui, |ui| {
                ui.add_space(8.0);
                ui.heading("Maestro");
                ui.add_space(8.0);
                for v in View::ALL {
                    ui.selectable_value(&mut self.view, v, v.label());
                }
            });

        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(&self.status);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    match self.snap.generated {
                        Some(t) => ui.weak(format!("updated {}", t.format("%H:%M:%S"))),
                        None => ui.weak("waiting for first snapshot"),
                    };
                    let running: Vec<_> = self.jobs.running().copied().collect();
                    if !running.is_empty() {
                        ui.spinner();
                        ui.weak(running.join(", "));
                    }
                });
            });
        });

        egui::CentralPanel::default().show(ui, |ui| {
            // Collection problems are surfaced rather than swallowed: a
            // dashboard of zeroes because the ledger would not open is
            // indistinguishable from a genuinely idle day otherwise.
            if !self.snap.problems.is_empty() {
                let problems = self.snap.problems.join("; ");
                ui.colored_label(egui::Color32::from_rgb(0xc6, 0x28, 0x28), problems);
                ui.add_space(4.0);
            }

            ui.heading(self.view.label());
            ui.add_space(6.0);

            match self.view {
                View::Dashboard => views::dashboard::show(self, ui),
                View::Providers => views::providers::show(self, ui),
                View::Projects => views::projects::show(self, ui),
                View::Rules => views::rules::show(self, ui),
                View::Questions => placeholder(
                    ui,
                    "The interrogation centre: pending clarifications grouped by project, \
                     answered with generated forms instead of one modal per question.",
                ),
                View::Activity => views::activity::show(self, ui),
                View::Chat => views::chat::show(self, ui),
            }
        });
    }
}

/// Honest stub: says what belongs here rather than pretending to be empty.
fn placeholder(ui: &mut egui::Ui, what: &str) {
    ui.weak("Not built yet.");
    ui.add_space(4.0);
    ui.label(what);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{ProjectRow, ProviderRow, RuleRow, SessionRow, Tickets, Usage};
    use chrono::Utc;
    use std::path::PathBuf;

    /// Build the whole widget tree for `view` with no window and no IO.
    ///
    /// egui panics on real layout mistakes - duplicate widget ids, a table
    /// whose column count does not match its header - so this is a genuine
    /// assertion, not a smoke test that only proves the code compiles.
    fn render(view: View, snap: Snapshot, selected: Option<&str>) {
        let ctx = egui::Context::default();
        let mut app = MaestroApp::headless(ctx.clone());
        app.view = view;
        app.snap = snap;
        app.selected_provider = selected.map(str::to_string);
        // Two passes: egui resolves some sizing on the frame after first use,
        // and a duplicate-id clash only surfaces once ids are remembered.
        for _ in 0..2 {
            let mut output = ctx.run_ui(Default::default(), |ui| app.draw(ui));
            // epaint asserts on drop that texture deltas were consumed, which
            // a real backend would do when uploading to the GPU. There is no
            // GPU here, so discard them explicitly.
            output.textures_delta.clear();
        }
    }

    fn populated() -> Snapshot {
        Snapshot {
            generated: Some(Utc::now()),
            providers: vec![
                ProviderRow {
                    id: "kimi".into(),
                    kind: "api".into(),
                    endpoint: Some("https://api.example.com".into()),
                    models: 3,
                    model_ids: vec!["k2".into(), "k3".into(), "k2.7".into()],
                    has_key: true,
                    is_cli: false,
                    tokens_today: 12_345,
                    cost_today: 0.42,
                    quota_pct: Some(51),
                },
                ProviderRow {
                    id: "cli-claude".into(),
                    kind: "cli".into(),
                    endpoint: None,
                    models: 0,
                    model_ids: vec![],
                    has_key: false,
                    is_cli: true,
                    tokens_today: 0,
                    cost_today: 0.0,
                    quota_pct: None,
                },
            ],
            projects: vec![
                ProjectRow {
                    id: "my-thing".into(),
                    name: "My Thing".into(),
                    dir: PathBuf::from("/ws/my-thing"),
                    description: "does a thing".into(),
                    stack: vec!["rust".into(), "egui".into()],
                    has_prd: true,
                    has_agents: true,
                    clarifications: 6,
                    pending: 2,
                    skills: 3,
                },
                ProjectRow {
                    id: "bare".into(),
                    name: "Bare".into(),
                    dir: PathBuf::from("/ws/bare"),
                    description: String::new(),
                    stack: vec![],
                    has_prd: false,
                    has_agents: false,
                    clarifications: 0,
                    pending: 0,
                    skills: 0,
                },
            ],
            rules: vec![
                RuleRow {
                    id: "coder-default".into(),
                    role: "coder".into(),
                    priority: 10,
                    chain: vec!["kimi/k2".into(), "ollama-local/qwen".into()],
                    conditions: 2,
                    min_quota_pct: Some(15),
                    project_scope: Some("my-thing".into()),
                },
                RuleRow {
                    id: "bare".into(),
                    role: "planner".into(),
                    priority: 0,
                    chain: vec![],
                    conditions: 0,
                    min_quota_pct: None,
                    project_scope: None,
                },
            ],
            workspace_root: Some(PathBuf::from("/ws")),
            sessions: vec![SessionRow {
                id: "qc-1".into(),
                provider_id: "kimi".into(),
                model: "k2".into(),
                state: "done".into(),
                tokens: 2_500_000,
                cost: 1.5,
                started: Utc::now(),
                transcript: None,
            }],
            today: Usage {
                tokens_in: 900,
                tokens_out: 300,
                cost: 0.0042,
                requests: 7,
            },
            tickets: Tickets {
                applied: 4,
                rejected: 1,
                queued: 0,
            },
            running_sessions: 1,
            problems: vec![],
        }
    }

    #[test]
    fn every_view_renders_when_empty() {
        for view in View::ALL {
            render(view, Snapshot::default(), None);
        }
    }

    #[test]
    fn every_view_renders_with_data() {
        for view in View::ALL {
            render(view, populated(), None);
        }
    }

    #[test]
    fn provider_detail_pane_renders_for_every_row() {
        for p in populated().providers {
            render(View::Providers, populated(), Some(&p.id));
        }
    }

    /// A selection left over from a provider that has since disappeared from
    /// the registry must not take the detail pane down with it.
    #[test]
    fn stale_provider_selection_is_harmless() {
        render(View::Providers, populated(), Some("deleted-provider"));
    }

    #[test]
    fn project_detail_pane_renders_for_every_row() {
        for p in populated().projects {
            let ctx = egui::Context::default();
            let mut app = MaestroApp::headless(ctx.clone());
            app.view = View::Projects;
            app.snap = populated();
            app.selected_project = Some(p.id.clone());
            for _ in 0..2 {
                let mut output = ctx.run_ui(Default::default(), |ui| app.draw(ui));
                output.textures_delta.clear();
            }
        }
    }

    /// Every modal must lay out on top of every view, not just the one that
    /// happens to open it.
    #[test]
    fn modals_render_over_each_view() {
        for view in View::ALL {
            let ctx = egui::Context::default();
            let mut app = MaestroApp::headless(ctx.clone());
            app.view = view;
            app.snap = populated();
            app.new_project.open();
            app.settings.populate(crate::jobs::ConfigForm {
                workspace_root: "/ws".into(),
                question_mode: maestro_core::types::QuestionMode::Balanced,
                global_cap: 0,
                per_provider_cap: 2,
                warn_pct: 80,
                crit_pct: 95,
            });
            app.add_provider.open();
            app.about_open = true;
            app.doctor = Some(vec![
                CheckRow {
                    name: "config.toml".into(),
                    ok: true,
                    detail: "loaded".into(),
                },
                CheckRow {
                    name: "secrets".into(),
                    ok: false,
                    detail: "keychain unavailable".into(),
                },
            ]);
            for _ in 0..2 {
                let mut output = ctx.run_ui(Default::default(), |ui| app.draw(ui));
                output.textures_delta.clear();
            }
        }
    }

    /// The menu bar builds every submenu each frame; a duplicate widget id
    /// across menus would panic here.
    #[test]
    fn menu_actions_dispatch_without_panicking() {
        let ctx = egui::Context::default();
        let mut app = MaestroApp::headless(ctx.clone());
        app.snap = populated();
        // Exit and the job-spawning actions are exercised too: dispatch must be
        // safe to call outside a real event loop.
        for action in [
            Action::NewProject,
            Action::About,
            Action::Settings,
            Action::AddProvider,
            Action::Theme(egui::ThemePreference::Light),
            Action::Theme(egui::ThemePreference::Dark),
            Action::Exit,
        ] {
            app.dispatch(action, &ctx);
            let mut output = ctx.run_ui(Default::default(), |ui| app.draw(ui));
            output.textures_delta.clear();
        }
    }

    /// The Add Provider form swaps fields depending on the type - a cli entry
    /// shows a binary path where an api entry shows endpoint and key.
    #[test]
    fn add_provider_renders_for_every_provider_type() {
        use maestro_core::types::ProviderType;
        for ptype in [
            ProviderType::Api,
            ProviderType::Cli,
            ProviderType::OllamaLocal,
            ProviderType::OllamaCloud,
            ProviderType::OllamaRemote,
        ] {
            let ctx = egui::Context::default();
            let mut app = MaestroApp::headless(ctx.clone());
            app.view = View::Providers;
            app.snap = populated();
            app.add_provider.edit(crate::jobs::ProviderForm {
                id: "probe".into(),
                ptype,
                ..Default::default()
            });
            for _ in 0..2 {
                let mut output = ctx.run_ui(Default::default(), |ui| app.draw(ui));
                output.textures_delta.clear();
            }
        }
    }

    /// Each dry-run outcome renders differently, including the skipped-entry
    /// list that only appears on a successful decision.
    #[test]
    fn every_dry_run_outcome_renders() {
        let outcomes = [
            DryRun::Decided {
                role: "coder".into(),
                rule_id: "coder-default".into(),
                provider: "kimi".into(),
                model: "k2".into(),
                explanation: "first healthy entry with quota left".into(),
                skipped: vec![
                    ("anthropic/claude".into(), "unhealthy".into()),
                    ("ollama-cloud/glm".into(), "quota below 15%".into()),
                ],
            },
            DryRun::NoRoute {
                role: "reviewer".into(),
                reason: "no rule matches and no provider is healthy".into(),
            },
            DryRun::Cancelled,
        ];
        for outcome in outcomes {
            let ctx = egui::Context::default();
            let mut app = MaestroApp::headless(ctx.clone());
            app.view = View::Rules;
            app.snap = populated();
            app.dry_run_result = Some(outcome);
            app.dry_run_progress = Some("checking kimi (3/8)".into());
            for _ in 0..2 {
                let mut output = ctx.run_ui(Default::default(), |ui| app.draw(ui));
                output.textures_delta.clear();
            }
        }
    }

    /// Cancelling must not leave a set flag behind that would abort the next
    /// sweep before it starts.
    #[test]
    fn a_new_dry_run_gets_a_fresh_cancel_flag() {
        let ctx = egui::Context::default();
        let mut app = MaestroApp::headless(ctx);
        app.start_dry_run();
        app.cancel_dry_run();
        assert!(app.dry_run_cancel.load(Ordering::Relaxed));
        let cancelled_flag = Arc::clone(&app.dry_run_cancel);

        app.start_dry_run();
        assert!(
            !app.dry_run_cancel.load(Ordering::Relaxed),
            "the new run must start uncancelled"
        );
        assert!(
            cancelled_flag.load(Ordering::Relaxed),
            "the old run keeps its own flag so it still stops"
        );
    }

    /// Fonts are built lazily, and stripping output consults them, so lay out
    /// one frame first - exactly what happens before any real chunk arrives.
    fn warmed() -> egui::Context {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(Default::default(), |ui| {
            ui.label("warm up");
        });
        out.textures_delta.clear();
        ctx
    }

    fn turn(text: &str) -> maestro_providers::conversation::Turn {
        maestro_providers::conversation::Turn {
            text: text.to_string(),
            tokens_in: 10,
            tokens_out: 20,
            cost: 0.001,
            latency_ms: 5,
        }
    }

    /// The defect this whole view exists to fix: a reply must join the history
    /// so the next request carries the conversation.
    #[test]
    fn a_reply_becomes_context_for_the_next_turn() {
        let ctx = egui::Context::default();
        let mut app = MaestroApp::headless(ctx);
        app.chat_history
            .push(maestro_providers::chat::ChatMessage::user("hello"));
        app.apply_chat_reply(Ok(turn("  hi there  ")));

        assert_eq!(app.chat_history.len(), 2);
        assert_eq!(app.chat_history[1].role, "assistant");
        assert_eq!(app.chat_history[1].content, "hi there");
        assert_eq!(app.chat_totals.tokens_in, 10);
        assert_eq!(app.chat_totals.tokens_out, 20);

        // A second exchange keeps accumulating rather than replacing.
        app.chat_history
            .push(maestro_providers::chat::ChatMessage::user("more"));
        app.apply_chat_reply(Ok(turn("sure")));
        assert_eq!(app.chat_history.len(), 4);
        assert_eq!(app.chat_totals.tokens_out, 40);
    }

    /// A failed turn must not leave two user messages in a row behind.
    #[test]
    fn a_failed_turn_is_removed_from_history() {
        let ctx = egui::Context::default();
        let mut app = MaestroApp::headless(ctx);
        app.chat_history
            .push(maestro_providers::chat::ChatMessage::user("hello"));
        app.apply_chat_reply(Err("HTTP 429".into()));
        assert!(app.chat_history.is_empty());

        app.chat_history
            .push(maestro_providers::chat::ChatMessage::user("a"));
        app.apply_chat_reply(Ok(turn("b")));
        app.chat_history
            .push(maestro_providers::chat::ChatMessage::user("c"));
        app.apply_chat_reply(Err("boom".into()));
        let roles: Vec<&str> = app.chat_history.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(roles, vec!["user", "assistant"]);
    }

    /// A new chat must not bill its tokens to the previous conversation.
    #[test]
    fn resetting_starts_a_new_session() {
        let ctx = egui::Context::default();
        let mut app = MaestroApp::headless(ctx);
        app.chat_history
            .push(maestro_providers::chat::ChatMessage::user("hello"));
        app.apply_chat_reply(Ok(turn("hi")));
        let first = app.chat_session.clone();

        app.reset_chat();
        assert!(app.chat_history.is_empty());
        assert_eq!(app.chat_totals.tokens_in, 0);
        assert_ne!(app.chat_session, first);
    }

    /// Sending without a provider must not silently swallow the typed message.
    #[test]
    fn sending_without_a_provider_keeps_the_input() {
        let ctx = egui::Context::default();
        let mut app = MaestroApp::headless(ctx);
        app.chat_input = "hello".into();
        app.send_chat();
        assert_eq!(app.chat_input, "hello");
        assert!(app.chat_history.is_empty());
    }

    /// A chatty agent must not grow the buffer without bound, and trimming
    /// must not split a multi-byte character.
    #[test]
    fn agent_output_is_capped_without_breaking_utf8() {
        let ctx = warmed();
        let mut app = MaestroApp::headless(ctx.clone());
        for _ in 0..40 {
            // 10k of multi-byte text per chunk.
            app.append_cli_output(&"é".repeat(5_000), &ctx);
        }
        assert!(app.cli_output.len() <= 200_000 + 16);
        // Still valid UTF-8 and still the same character throughout.
        assert!(app.cli_output.chars().all(|c| c == 'é'));
    }

    #[test]
    fn agent_output_has_escape_codes_stripped() {
        let ctx = warmed();
        let mut app = MaestroApp::headless(ctx.clone());
        app.append_cli_output("\u{1b}[32m[mock-agent] done\u{1b}[0m\r\n", &ctx);
        assert_eq!(app.cli_output, "[mock-agent] done\n");
    }

    #[test]
    fn activity_renders_a_finished_report() {
        let ctx = egui::Context::default();
        let mut app = MaestroApp::headless(ctx.clone());
        app.view = View::Activity;
        app.snap = populated();
        app.cli_output = "[mock-agent] working...\n".into();
        app.cli_report = Some(crate::jobs::CliReport {
            session_id: "cli-123".into(),
            exit_code: Some(0),
            applied: 2,
            rejected: 1,
            paths: vec!["mock-created.md".into(), "existing.txt".into()],
            transcript: PathBuf::from("/tmp/cli-123.log"),
            tokens_in_est: 1200,
            tokens_out_est: 800,
        });
        for _ in 0..2 {
            let mut output = ctx.run_ui(Default::default(), |ui| app.draw(ui));
            output.textures_delta.clear();
        }

        // A non-zero exit renders down a different branch.
        app.cli_report.as_mut().unwrap().exit_code = Some(2);
        let mut output = ctx.run_ui(Default::default(), |ui| app.draw(ui));
        output.textures_delta.clear();
    }

    /// Collection problems are meant to be visible, not swallowed.
    #[test]
    fn problems_are_rendered() {
        let snap = Snapshot {
            problems: vec!["ledger unavailable: locked".into()],
            ..Snapshot::default()
        };
        render(View::Dashboard, snap, None);
    }
}
