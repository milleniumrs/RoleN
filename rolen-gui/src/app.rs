//! The application shell: navigation, status bar, and the wiring between the
//! poller, the job system and the views.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use dear_imgui_rs::{
    Condition, Key, KeyChord, KeyMods, ShortcutRoute, ThemePreset, Ui, WindowFlags,
};

use crate::dialogs::{
    self, AddProviderDialog, CliTaskDialog, ModalState, NewProjectDialog, ProviderRequest,
    SettingsDialog,
};
use crate::jobs::{self, CheckRow, DryRun, HealthRow, JobMsg, Jobs};
use crate::menu::{self, Action};
use crate::state::{Poller, Snapshot};
use crate::views;
use crate::wake::{self, Wake};

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

pub struct RoleNApp {
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
    pub chat_history: Vec<rolen_providers::chat::ChatMessage>,
    pub chat_input: String,
    pub chat_session: String,
    pub chat_totals: rolen_providers::conversation::Totals,
    pub cli_task: CliTaskDialog,
    /// Live agent output, ANSI already stripped.
    pub cli_output: String,
    pub cli_report: Option<jobs::CliReport>,
    /// `Some` while the doctor report modal is showing.
    doctor: Option<Vec<CheckRow>>,
    doctor_modal: ModalState,
    about_modal: ModalState,
    status: String,
    /// The colour preset the shell should apply. Owned here because only the
    /// shell holds `&mut Context`; the draw code can only ask for a change.
    theme: ThemePreset,
    theme_dirty: bool,
    /// Set by `File > Exit`; the shell turns it into an event-loop exit.
    exit_requested: bool,
}

impl RoleNApp {
    pub fn new(wake: Wake) -> Self {
        let poller = Poller::spawn(wake.clone(), POLL_INTERVAL);
        Self::with_poller(wake, poller)
    }

    /// An app with no polling thread and a no-op waker, for rendering the UI
    /// without a window.
    pub fn headless() -> Self {
        Self::with_poller(wake::no_op(), Poller::inert())
    }

    fn with_poller(wake: Wake, poller: Poller) -> Self {
        Self {
            view: View::Dashboard,
            jobs: Jobs::new(wake),
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
            chat_session: rolen_providers::conversation::new_session_id(),
            chat_totals: rolen_providers::conversation::Totals::default(),
            cli_task: CliTaskDialog::default(),
            cli_output: String::new(),
            cli_report: None,
            doctor: None,
            doctor_modal: ModalState::default(),
            about_modal: ModalState::default(),
            status: "ready".to_string(),
            theme: ThemePreset::Dark,
            theme_dirty: true, // the shell applies the initial preset on startup
            exit_requested: false,
        }
    }

    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status = msg.into();
    }

    /// A pending theme change for the shell to apply to the imgui context.
    pub fn take_theme_change(&mut self) -> Option<ThemePreset> {
        self.theme_dirty.then(|| {
            self.theme_dirty = false;
            self.theme
        })
    }

    /// Did the user ask to quit? The shell checks this once per frame.
    pub fn take_exit_request(&mut self) -> bool {
        std::mem::take(&mut self.exit_requested)
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
    /// The buffer is capped: a chatty agent can emit megabytes, and the whole
    /// buffer is drawn every frame.
    fn append_cli_output(&mut self, chunk: &str) {
        const CAP: usize = 200_000;
        self.cli_output
            .push_str(&crate::text::strip_ansi(&crate::text::renderable(chunk)));
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
    pub fn apply_chat_reply(&mut self, reply: Result<rolen_providers::conversation::Turn, String>) {
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
                    .push(rolen_providers::chat::ChatMessage::assistant(
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
        self.chat_totals = rolen_providers::conversation::Totals::default();
        self.chat_session = rolen_providers::conversation::new_session_id();
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
            .push(rolen_providers::chat::ChatMessage::user(prompt));

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
                    self.doctor_modal.open();
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
                JobMsg::CliOutput(chunk) => self.append_cli_output(&chunk),
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
    fn dispatch(&mut self, action: Action) {
        match action {
            Action::NewProject => self.new_project.open(),
            Action::Doctor => {
                self.jobs.spawn(jobs::DOCTOR, jobs::doctor);
            }
            Action::Exit => self.exit_requested = true,
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
            Action::Theme(preset) => {
                self.theme = preset;
                self.theme_dirty = true;
            }
            Action::About => self.about_modal.open(),
        }
    }

    /// Shortcuts that match the TUI's bindings.
    fn shortcut(&self, ui: &Ui) -> Option<Action> {
        const R: ShortcutRoute = ShortcutRoute::Focused;
        let pressed =
            |key: Key, mods: KeyMods| ui.shortcut_with_flags(KeyChord::new(key).with_mods(mods), R);
        if pressed(Key::N, KeyMods::CTRL) || pressed(Key::N, KeyMods::CTRL | KeyMods::SHIFT) {
            Some(Action::NewProject)
        } else if pressed(Key::F9, KeyMods::empty()) {
            Some(Action::Doctor)
        } else if pressed(Key::F10, KeyMods::empty()) {
            Some(Action::Settings)
        } else if pressed(Key::D, KeyMods::CTRL) {
            Some(Action::DryRun)
        } else if pressed(Key::Q, KeyMods::CTRL) {
            Some(Action::QuickChat)
        } else if pressed(Key::F1, KeyMods::empty()) {
            Some(Action::About)
        } else {
            None
        }
    }

    fn modals(&mut self, ui: &Ui) {
        if let Some(draft) = self.new_project.show(ui) {
            self.jobs.spawn(jobs::NEW_PROJECT, move || {
                jobs::scaffold_project(draft.name, draft.description, draft.stack)
            });
        }

        if let Some(form) = self.settings.show(ui) {
            self.jobs
                .spawn(jobs::SAVE_CONFIG, move || jobs::save_config(form));
        }

        let cli_providers = self.runnable_cli_providers();
        if let Some(req) = self.cli_task.show(ui, &cli_providers) {
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

        match self.add_provider.show(ui) {
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

        if self.doctor.is_some() {
            match self.doctor_modal.begin(ui, "Config doctor") {
                Some(_modal) => {
                    ui.set_window_size_with_cond([600.0, 0.0], Condition::FirstUseEver);
                    if let Some(rows) = &self.doctor {
                        for row in rows {
                            if row.ok {
                                ui.text_colored(dialogs::OK, "OK  ");
                            } else {
                                ui.text_colored(dialogs::ERROR, "FAIL");
                            }
                            ui.same_line();
                            ui.text(&row.name);
                            ui.same_line();
                            ui.text_disabled(&row.detail);
                        }
                    }
                    ui.spacing();
                    if ui.button("Close") {
                        ui.close_current_popup();
                    }
                }
                // The popup was closed; drop the report with it.
                None => self.doctor = None,
            }
        }

        if let Some(_modal) = self.about_modal.begin(ui, "About RoleN") {
            ui.text(format!("version {}", env!("CARGO_PKG_VERSION")));
            ui.text("A conductor for LLM-powered development.");
            ui.text("MIT License");
            ui.spacing();
            if ui.button("Close") {
                ui.close_current_popup();
            }
        }
    }

    /// The whole frame, driven by the shell's `Context::frame()`.
    ///
    /// Kept independent of the winit/glow plumbing so tests can build it on a
    /// headless `Context`.
    pub fn draw(&mut self, ui: &Ui) {
        self.handle_jobs();
        if let Some(snap) = self.poller.latest() {
            self.snap = snap;
        }

        let mut action = self.shortcut(ui);

        // One borderless window covers the whole viewport; the menu bar, nav
        // column, content area and status bar all live inside it.
        let viewport = ui.main_viewport();
        ui.set_next_window_viewport(viewport.id());
        let pos = viewport.pos();
        let size = viewport.size();

        let flags = WindowFlags::NO_TITLE_BAR
            | WindowFlags::NO_COLLAPSE
            | WindowFlags::NO_RESIZE
            | WindowFlags::NO_MOVE
            | WindowFlags::NO_BRING_TO_FRONT_ON_FOCUS
            | WindowFlags::MENU_BAR;

        ui.window("rolen-root")
            .flags(flags)
            .position(pos, Condition::Always)
            .size(size, Condition::Always)
            .build(|| {
                if let Some(picked) = menu::show(ui) {
                    action = Some(picked);
                }

                let avail = ui.content_region_avail();
                let status_h = ui.frame_height_with_spacing() + 6.0;
                let content_h = (avail[1] - status_h).max(80.0);

                ui.child_window("nav")
                    .size([160.0, content_h])
                    .build(ui, || {
                        ui.spacing();
                        ui.text("RoleN");
                        ui.separator();
                        ui.spacing();
                        for v in View::ALL {
                            if ui
                                .selectable_config(v.label())
                                .selected(self.view == v)
                                .build()
                            {
                                self.view = v;
                            }
                        }
                    });

                ui.same_line();

                ui.child_window("main")
                    .size([0.0, content_h])
                    .build(ui, || {
                        // Collection problems are surfaced rather than swallowed: a
                        // dashboard of zeroes because the ledger would not open is
                        // indistinguishable from a genuinely idle day otherwise.
                        if !self.snap.problems.is_empty() {
                            ui.text_colored(dialogs::ERROR, self.snap.problems.join("; "));
                            ui.spacing();
                        }

                        ui.text(self.view.label());
                        ui.separator();
                        ui.spacing();

                        match self.view {
                            View::Dashboard => views::dashboard::show(self, ui),
                            View::Providers => views::providers::show(self, ui),
                            View::Projects => views::projects::show(self, ui),
                            View::Rules => views::rules::show(self, ui),
                            View::Questions => {
                                ui.text_disabled("Not built yet.");
                                ui.text_wrapped(
                                    "The interrogation centre: pending clarifications grouped by \
                                 project, answered with generated forms instead of one modal \
                                 per question.",
                                );
                            }
                            View::Activity => views::activity::show(self, ui),
                            View::Chat => views::chat::show(self, ui),
                        }
                    });

                ui.separator();
                ui.text(&self.status);
                ui.same_line();
                let running: Vec<_> = self.jobs.running().copied().collect();
                let right = if running.is_empty() {
                    match self.snap.generated {
                        Some(t) => format!("updated {}", t.format("%H:%M:%S")),
                        None => "waiting for first snapshot".to_string(),
                    }
                } else {
                    format!("running: {}", running.join(", "))
                };
                let right_w = ui.calc_text_size(&right)[0];
                let x = (ui.window_width() - right_w - 16.0).max(ui.cursor_pos_x());
                ui.set_cursor_pos_x(x);
                ui.text_disabled(right);

                if let Some(action) = action {
                    self.dispatch(action);
                }
                self.modals(ui);
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{ProjectRow, ProviderRow, RuleRow, SessionRow, Tickets, Usage};
    use chrono::Utc;
    use dear_imgui_rs::Context;
    use std::path::PathBuf;

    /// A headless imgui context: no window, no GPU, but real layout. A widget
    /// tree that panics - duplicate ids, unbalanced stacks - fails here.
    ///
    /// The binding allows only one live context per thread set, and the test
    /// harness runs tests on parallel threads, so tests serialize on a mutex.
    struct TestUi {
        ctx: Context,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl TestUi {
        fn new() -> Self {
            static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
            let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let mut ctx = Context::create();
            // Tests run with the crate root as cwd; don't leave imgui.ini behind.
            let _ = ctx.set_ini_filename(None::<String>);
            ctx.io_mut().set_display_size([1280.0, 820.0]);
            ctx.io_mut().set_delta_time(1.0 / 60.0);
            ctx.font_atlas()
                .try_claim_legacy_renderer()
                .expect("legacy renderer font atlas should be available")
                .build();
            Self { ctx, _guard: guard }
        }

        fn frame(&mut self, f: impl FnOnce(&mut RoleNApp, &Ui), app: &mut RoleNApp) {
            {
                let ui = self.ctx.frame();
                f(app, ui);
            }
            // Ends the frame without drawing; the draw data is discarded.
            let _ = self.ctx.render_legacy();
        }
    }

    /// Build the whole widget tree for `view` with no window and no IO.
    fn render(view: View, snap: Snapshot, selected: Option<&str>) {
        let mut ui = TestUi::new();
        let mut app = RoleNApp::headless();
        app.view = view;
        app.snap = snap;
        app.selected_provider = selected.map(str::to_string);
        // Two passes: some sizing resolves on the frame after first use, and a
        // duplicate-id clash only surfaces once ids are remembered.
        for _ in 0..2 {
            ui.frame(|app, ui| app.draw(ui), &mut app);
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
                    stack: vec!["rust".into(), "imgui".into()],
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
            let mut ui = TestUi::new();
            let mut app = RoleNApp::headless();
            app.view = View::Projects;
            app.snap = populated();
            app.selected_project = Some(p.id.clone());
            for _ in 0..2 {
                ui.frame(|app, ui| app.draw(ui), &mut app);
            }
        }
    }

    /// Every modal must lay out on top of every view, not just the one that
    /// happens to open it.
    #[test]
    fn modals_render_over_each_view() {
        for view in View::ALL {
            let mut ui = TestUi::new();
            let mut app = RoleNApp::headless();
            app.view = view;
            app.snap = populated();
            app.new_project.open();
            app.settings.populate(crate::jobs::ConfigForm {
                workspace_root: "/ws".into(),
                question_mode: rolen_core::types::QuestionMode::Balanced,
                global_cap: 0,
                per_provider_cap: 2,
                warn_pct: 80,
                crit_pct: 95,
            });
            app.add_provider.open();
            app.about_modal.open();
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
            app.doctor_modal.open();
            for _ in 0..2 {
                ui.frame(|app, ui| app.draw(ui), &mut app);
            }
        }
    }

    /// The menu bar builds every submenu each frame.
    #[test]
    fn menu_actions_dispatch_without_panicking() {
        let mut ui = TestUi::new();
        let mut app = RoleNApp::headless();
        app.snap = populated();
        for action in [
            Action::NewProject,
            Action::About,
            Action::Settings,
            Action::AddProvider,
            Action::Theme(ThemePreset::Light),
            Action::Theme(ThemePreset::Dark),
        ] {
            app.dispatch(action);
            ui.frame(|app, ui| app.draw(ui), &mut app);
        }
    }

    /// A theme change is handed to the shell exactly once.
    #[test]
    fn a_theme_change_is_pending_until_the_shell_takes_it() {
        let mut app = RoleNApp::headless();
        app.theme_dirty = false;
        app.dispatch(Action::Theme(ThemePreset::Light));
        assert_eq!(app.take_theme_change(), Some(ThemePreset::Light));
        assert_eq!(app.take_theme_change(), None);
    }

    #[test]
    fn exit_is_a_request_the_shell_consumes() {
        let mut app = RoleNApp::headless();
        app.dispatch(Action::Exit);
        assert!(app.take_exit_request());
        assert!(!app.take_exit_request());
    }

    /// The Add Provider form swaps fields depending on the type - a cli entry
    /// shows a binary path where an api entry shows endpoint and key.
    #[test]
    fn add_provider_renders_for_every_provider_type() {
        use rolen_core::types::ProviderType;
        for ptype in [
            ProviderType::Api,
            ProviderType::Cli,
            ProviderType::OllamaLocal,
            ProviderType::OllamaCloud,
            ProviderType::OllamaRemote,
        ] {
            let mut ui = TestUi::new();
            let mut app = RoleNApp::headless();
            app.view = View::Providers;
            app.snap = populated();
            app.add_provider.edit(crate::jobs::ProviderForm {
                id: "probe".into(),
                ptype,
                ..Default::default()
            });
            for _ in 0..2 {
                ui.frame(|app, ui| app.draw(ui), &mut app);
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
            let mut ui = TestUi::new();
            let mut app = RoleNApp::headless();
            app.view = View::Rules;
            app.snap = populated();
            app.dry_run_result = Some(outcome);
            app.dry_run_progress = Some("checking kimi (3/8)".into());
            for _ in 0..2 {
                ui.frame(|app, ui| app.draw(ui), &mut app);
            }
        }
    }

    /// Cancelling must not leave a set flag behind that would abort the next
    /// sweep before it starts.
    #[test]
    fn a_new_dry_run_gets_a_fresh_cancel_flag() {
        let mut app = RoleNApp::headless();
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

    fn turn(text: &str) -> rolen_providers::conversation::Turn {
        rolen_providers::conversation::Turn {
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
        let mut app = RoleNApp::headless();
        app.chat_history
            .push(rolen_providers::chat::ChatMessage::user("hello"));
        app.apply_chat_reply(Ok(turn("  hi there  ")));

        assert_eq!(app.chat_history.len(), 2);
        assert_eq!(app.chat_history[1].role, "assistant");
        assert_eq!(app.chat_history[1].content, "hi there");
        assert_eq!(app.chat_totals.tokens_in, 10);
        assert_eq!(app.chat_totals.tokens_out, 20);

        // A second exchange keeps accumulating rather than replacing.
        app.chat_history
            .push(rolen_providers::chat::ChatMessage::user("more"));
        app.apply_chat_reply(Ok(turn("sure")));
        assert_eq!(app.chat_history.len(), 4);
        assert_eq!(app.chat_totals.tokens_out, 40);
    }

    /// A failed turn must not leave two user messages in a row behind.
    #[test]
    fn a_failed_turn_is_removed_from_history() {
        let mut app = RoleNApp::headless();
        app.chat_history
            .push(rolen_providers::chat::ChatMessage::user("hello"));
        app.apply_chat_reply(Err("HTTP 429".into()));
        assert!(app.chat_history.is_empty());

        app.chat_history
            .push(rolen_providers::chat::ChatMessage::user("a"));
        app.apply_chat_reply(Ok(turn("b")));
        app.chat_history
            .push(rolen_providers::chat::ChatMessage::user("c"));
        app.apply_chat_reply(Err("boom".into()));
        let roles: Vec<&str> = app.chat_history.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(roles, vec!["user", "assistant"]);
    }

    /// A new chat must not bill its tokens to the previous conversation.
    #[test]
    fn resetting_starts_a_new_session() {
        let mut app = RoleNApp::headless();
        app.chat_history
            .push(rolen_providers::chat::ChatMessage::user("hello"));
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
        let mut app = RoleNApp::headless();
        app.chat_input = "hello".into();
        app.send_chat();
        assert_eq!(app.chat_input, "hello");
        assert!(app.chat_history.is_empty());
    }

    /// A chatty agent must not grow the buffer without bound, and trimming
    /// must not split a multi-byte character.
    #[test]
    fn agent_output_is_capped_without_breaking_utf8() {
        let mut app = RoleNApp::headless();
        for _ in 0..40 {
            // 10k of multi-byte text per chunk.
            app.append_cli_output(&"é".repeat(5_000));
        }
        assert!(app.cli_output.len() <= 200_000 + 16);
        // All characters that remain are the ones that were fed in.
        assert!(app.cli_output.chars().all(|c| c == 'é'));
    }

    #[test]
    fn agent_output_has_escape_codes_stripped() {
        let mut app = RoleNApp::headless();
        app.append_cli_output("\u{1b}[32m[mock-agent] done\u{1b}[0m\r\n");
        assert_eq!(app.cli_output, "[mock-agent] done\n");
    }

    #[test]
    fn activity_renders_a_finished_report() {
        let mut ui = TestUi::new();
        let mut app = RoleNApp::headless();
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
            ui.frame(|app, ui| app.draw(ui), &mut app);
        }

        // A non-zero exit renders down a different branch.
        app.cli_report.as_mut().unwrap().exit_code = Some(2);
        ui.frame(|app, ui| app.draw(ui), &mut app);
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
