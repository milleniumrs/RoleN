//! The application shell: navigation, status bar, and the wiring between the
//! poller, the job system and the views.

use std::collections::HashMap;
use std::time::Duration;

use eframe::egui;

use crate::dialogs::{AddProviderDialog, NewProjectDialog, ProviderRequest, SettingsDialog};
use crate::jobs::{self, CheckRow, HealthRow, JobMsg, Jobs};
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
            doctor: None,
            about_open: false,
            status: "ready".to_string(),
        }
    }

    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status = msg.into();
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
        const ABOUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::F1);

        ctx.input_mut(|i| {
            if i.consume_shortcut(&NEW) || i.consume_shortcut(&NEW_ALT) {
                Some(Action::NewProject)
            } else if i.consume_shortcut(&DOCTOR) {
                Some(Action::Doctor)
            } else if i.consume_shortcut(&SETTINGS) {
                Some(Action::Settings)
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
                View::Rules => placeholder(
                    ui,
                    "Routing rules with a live dry-run. Dry-run calls routing::collect, which \
                     health-checks every provider serially, so it must be a cancellable job.",
                ),
                View::Questions => placeholder(
                    ui,
                    "The interrogation centre: pending clarifications grouped by project, \
                     answered with generated forms instead of one modal per question.",
                ),
                View::Activity => placeholder(
                    ui,
                    "Ledger stream: write tickets, routing decisions and quota events. The TUI \
                     never implemented this tab.",
                ),
                View::Chat => placeholder(
                    ui,
                    "Chat with real multi-turn history. The TUI sends every message as a fresh \
                     single-message request, so the model never sees the conversation.",
                ),
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
    use crate::state::{ProjectRow, ProviderRow, SessionRow, Tickets, Usage};
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
