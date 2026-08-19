//! Modal dialogs.
//!
//! Dialogs hold their own field state and return a value only when the user
//! confirms, so the app never has to reach into half-typed input.

use eframe::egui;
use maestro_core::types::{ProviderType, QuestionMode};

use crate::jobs::{ConfigForm, ProviderForm};

/// What the Add Provider dialog wants done next.
pub enum ProviderRequest {
    Discover(ProviderForm),
    Save(ProviderForm),
}

/// `Providers > Add Provider`.
#[derive(Default)]
pub struct AddProviderDialog {
    open: bool,
    form: ProviderForm,
    error: Option<String>,
    note: Option<String>,
    busy: bool,
}

impl AddProviderDialog {
    pub fn open(&mut self) {
        *self = Self {
            open: true,
            ..Default::default()
        };
    }

    /// Open pre-filled to edit an existing provider. The key is never shown -
    /// it lives in the keychain and cannot be read back into the form.
    pub fn edit(&mut self, form: ProviderForm) {
        *self = Self {
            open: true,
            form,
            ..Default::default()
        };
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn set_busy(&mut self, busy: bool) {
        self.busy = busy;
    }

    pub fn discovered(&mut self, models: Vec<maestro_core::types::Model>) {
        self.note = Some(format!(
            "found {} model(s); they will be saved with the provider",
            models.len()
        ));
        self.form.models = models;
        self.busy = false;
    }

    pub fn failed(&mut self, message: String) {
        self.error = Some(message);
        self.busy = false;
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Option<ProviderRequest> {
        if !self.open {
            return None;
        }
        let mut request = None;
        let mut close = false;

        let response = egui::Modal::new(egui::Id::new("add-provider-modal")).show(ctx, |ui| {
            ui.set_width(560.0);
            ui.heading("Add provider");
            ui.add_space(8.0);

            egui::Grid::new("add-provider-fields")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Id");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.form.id)
                            .desired_width(360.0)
                            .hint_text("unique, e.g. kimi or ollama-local"),
                    );
                    ui.end_row();

                    ui.label("Type");
                    egui::ComboBox::from_id_salt("provider-type")
                        .selected_text(type_label(self.form.ptype))
                        .show_ui(ui, |ui| {
                            for t in [
                                ProviderType::Api,
                                ProviderType::Cli,
                                ProviderType::OllamaLocal,
                                ProviderType::OllamaCloud,
                                ProviderType::OllamaRemote,
                            ] {
                                if ui
                                    .selectable_value(&mut self.form.ptype, t, type_label(t))
                                    .clicked()
                                {
                                    // Ollama's bases are known, so offer them
                                    // instead of making the user recall a URL.
                                    if self.form.endpoint.trim().is_empty() {
                                        self.form.endpoint = match t {
                                            ProviderType::OllamaLocal => {
                                                maestro_providers::ollama::DEFAULT_LOCAL_BASE.into()
                                            }
                                            ProviderType::OllamaCloud => {
                                                maestro_providers::ollama::DEFAULT_CLOUD_BASE.into()
                                            }
                                            _ => String::new(),
                                        };
                                    }
                                }
                            }
                        });
                    ui.end_row();

                    if self.form.ptype == ProviderType::Cli {
                        ui.label("CLI path");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.form.cli_path)
                                .desired_width(360.0)
                                .hint_text("full path to the agent binary"),
                        );
                        ui.end_row();
                    } else {
                        ui.label("Endpoint");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.form.endpoint)
                                .desired_width(360.0)
                                .hint_text("https://api.example.com/v1"),
                        );
                        ui.end_row();

                        ui.label("API key");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.form.key)
                                .desired_width(360.0)
                                .password(true)
                                .hint_text("stored in the OS keychain, never in providers.toml"),
                        );
                        ui.end_row();
                    }

                    ui.label("Models");
                    if self.form.models.is_empty() {
                        ui.weak("none yet - Discover asks the endpoint");
                    } else {
                        ui.label(format!("{} discovered", self.form.models.len()));
                    }
                    ui.end_row();
                });

            if let Some(note) = &self.note {
                ui.add_space(6.0);
                ui.weak(note);
            }
            if let Some(err) = &self.error {
                ui.add_space(6.0);
                ui.colored_label(egui::Color32::from_rgb(0xc6, 0x28, 0x28), err);
            }

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let can_discover = self.form.ptype != ProviderType::Cli && !self.busy;
                if ui
                    .add_enabled(can_discover, egui::Button::new("Discover models"))
                    .on_disabled_hover_text(if self.form.ptype == ProviderType::Cli {
                        "a CLI agent has no /models endpoint"
                    } else {
                        "already working"
                    })
                    .clicked()
                {
                    match validate_provider(&self.form) {
                        Some(err) => self.error = Some(err),
                        None => {
                            self.error = None;
                            self.busy = true;
                            request = Some(ProviderRequest::Discover(self.form.clone()));
                        }
                    }
                }

                if ui
                    .add_enabled(!self.busy, egui::Button::new("Save"))
                    .clicked()
                {
                    match validate_provider(&self.form) {
                        Some(err) => self.error = Some(err),
                        None => {
                            self.error = None;
                            self.busy = true;
                            request = Some(ProviderRequest::Save(self.form.clone()));
                        }
                    }
                }

                if ui.button("Cancel").clicked() {
                    close = true;
                }
                if self.busy {
                    ui.spinner();
                }
            });
        });

        if close || response.should_close() {
            self.open = false;
        }
        request
    }
}

/// Why a provider form cannot be submitted, if it cannot.
///
/// The CLI-path rule is stricter than the TUI, which registers `cli` providers
/// with `cli_path: None` (`maestro-tui/src/add_provider.rs:123`) - and
/// `run_cli_session` then refuses them, so the entry is dead on arrival.
pub fn validate_provider(form: &ProviderForm) -> Option<String> {
    let id = form.id.trim();
    if id.is_empty() {
        return Some("id is required".to_string());
    }
    if id.split_whitespace().count() > 1 {
        return Some("id must not contain whitespace".to_string());
    }
    match form.ptype {
        ProviderType::Cli => {
            if form.cli_path.trim().is_empty() {
                return Some("a cli provider needs the path to its binary".to_string());
            }
        }
        ProviderType::Api | ProviderType::OllamaRemote => {
            if form.endpoint.trim().is_empty() {
                return Some("an endpoint is required for this type".to_string());
            }
        }
        // Local and cloud Ollama fall back to their well-known bases.
        ProviderType::OllamaLocal | ProviderType::OllamaCloud => {}
    }
    None
}

fn type_label(t: ProviderType) -> &'static str {
    match t {
        ProviderType::Api => "api - OpenAI-compatible or Anthropic",
        ProviderType::Cli => "cli - PTY-wrapped agent",
        ProviderType::OllamaLocal => "ollama-local",
        ProviderType::OllamaCloud => "ollama-cloud",
        ProviderType::OllamaRemote => "ollama-remote - over an SSH tunnel",
    }
}

/// `Tools > Settings`.
///
/// Numeric fields are drag/spin controls rather than free text: the TUI parses
/// them with `parse()` and silently keeps the old value when parsing fails
/// (`maestro-tui/src/settings.rs:160`), so a typo looks like it saved.
#[derive(Default)]
pub struct SettingsDialog {
    open: bool,
    form: Option<ConfigForm>,
    error: Option<String>,
}

impl SettingsDialog {
    /// Populate from a freshly loaded config and show.
    pub fn populate(&mut self, form: ConfigForm) {
        self.form = Some(form);
        self.error = None;
        self.open = true;
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    /// Returns the form when Save is pressed and validation passes.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<ConfigForm> {
        if !self.open {
            return None;
        }
        let form = self.form.as_mut()?;
        let mut result = None;
        let mut close = false;

        let response = egui::Modal::new(egui::Id::new("settings-modal")).show(ctx, |ui| {
            ui.set_width(520.0);
            ui.heading("Settings");
            ui.add_space(8.0);

            egui::Grid::new("settings-fields")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Workspace root");
                    ui.add(
                        egui::TextEdit::singleline(&mut form.workspace_root).desired_width(360.0),
                    );
                    ui.end_row();

                    ui.label("Question mode");
                    egui::ComboBox::from_id_salt("question-mode")
                        .selected_text(mode_label(form.question_mode))
                        .show_ui(ui, |ui| {
                            for mode in [
                                QuestionMode::Thorough,
                                QuestionMode::Balanced,
                                QuestionMode::Minimal,
                            ] {
                                ui.selectable_value(
                                    &mut form.question_mode,
                                    mode,
                                    mode_label(mode),
                                );
                            }
                        });
                    ui.end_row();

                    ui.label("Global parallelism cap");
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut form.global_cap).range(0..=64));
                        ui.weak("0 = automatic (half the logical CPUs, min 2)");
                    });
                    ui.end_row();

                    ui.label("Per-provider cap");
                    ui.add(egui::DragValue::new(&mut form.per_provider_cap).range(1..=64));
                    ui.end_row();

                    ui.label("Quota warn %");
                    ui.add(egui::DragValue::new(&mut form.warn_pct).range(0..=100));
                    ui.end_row();

                    ui.label("Quota critical %");
                    ui.add(egui::DragValue::new(&mut form.crit_pct).range(0..=100));
                    ui.end_row();
                });

            ui.add_space(6.0);
            ui.weak(
                "The TUI colour theme and the quota alert action are stored in the same file \
                 and are left untouched by this form.",
            );

            if let Some(err) = &self.error {
                ui.add_space(6.0);
                ui.colored_label(egui::Color32::from_rgb(0xc6, 0x28, 0x28), err);
            }

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    match validate_thresholds(form.warn_pct, form.crit_pct) {
                        Some(err) => self.error = Some(err),
                        None => {
                            result = Some(form.clone());
                            close = true;
                        }
                    }
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });

        if close || response.should_close() {
            self.open = false;
        }
        result
    }
}

/// Why a settings form cannot be saved, if it cannot.
///
/// Pulled out of the widget closure so the rule is testable without a UI.
/// A critical threshold of 100 % means "never alert", so an equal warn value
/// is not a contradiction there.
pub fn validate_thresholds(warn_pct: u8, crit_pct: u8) -> Option<String> {
    if warn_pct >= crit_pct && crit_pct < 100 {
        Some("warn % must be below critical %".to_string())
    } else {
        None
    }
}

fn mode_label(mode: QuestionMode) -> &'static str {
    match mode {
        QuestionMode::Thorough => "thorough - ask everything",
        QuestionMode::Balanced => "balanced - fewer questions",
        QuestionMode::Minimal => "minimal - only blockers",
    }
}

/// `Sessions > Run CLI Task`.
#[derive(Default)]
pub struct CliTaskDialog {
    open: bool,
    pub provider: Option<String>,
    task: String,
    workdir: String,
    error: Option<String>,
}

/// A CLI run request, once confirmed.
pub struct CliTaskRequest {
    pub provider: String,
    pub task: String,
    pub workdir: std::path::PathBuf,
}

impl CliTaskDialog {
    /// `providers` is the list of registered `cli` provider ids.
    pub fn open(&mut self, providers: &[String], default_workdir: String) {
        *self = Self {
            open: true,
            provider: providers.first().cloned(),
            workdir: default_workdir,
            ..Default::default()
        };
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn show(&mut self, ctx: &egui::Context, providers: &[String]) -> Option<CliTaskRequest> {
        if !self.open {
            return None;
        }
        let mut request = None;
        let mut close = false;

        let response = egui::Modal::new(egui::Id::new("cli-task-modal")).show(ctx, |ui| {
            ui.set_width(560.0);
            ui.heading("Run CLI task");
            ui.add_space(8.0);

            if providers.is_empty() {
                ui.colored_label(
                    egui::Color32::from_rgb(0xc6, 0x28, 0x28),
                    "No cli providers are registered with a binary path.",
                );
            }

            egui::Grid::new("cli-task-fields")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Agent");
                    egui::ComboBox::from_id_salt("cli-task-provider")
                        .selected_text(self.provider.clone().unwrap_or_else(|| "-".into()))
                        .show_ui(ui, |ui| {
                            for id in providers {
                                ui.selectable_value(&mut self.provider, Some(id.clone()), id);
                            }
                        });
                    ui.end_row();

                    ui.label("Working directory");
                    ui.add(egui::TextEdit::singleline(&mut self.workdir).desired_width(380.0));
                    ui.end_row();

                    ui.label("Task");
                    ui.add(
                        egui::TextEdit::multiline(&mut self.task)
                            .desired_width(380.0)
                            .desired_rows(4)
                            .hint_text("what the agent should do"),
                    );
                    ui.end_row();
                });

            ui.add_space(6.0);
            ui.weak(
                "The workspace is copied to a staging directory, the agent runs there, and its \
                 changes come back through the write queue.",
            );

            if let Some(err) = &self.error {
                ui.add_space(6.0);
                ui.colored_label(egui::Color32::from_rgb(0xc6, 0x28, 0x28), err);
            }

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("Run").clicked() {
                    match self.validate() {
                        Some(err) => self.error = Some(err),
                        None => {
                            request = Some(CliTaskRequest {
                                provider: self.provider.clone().unwrap_or_default(),
                                task: self.task.trim().to_string(),
                                workdir: std::path::PathBuf::from(self.workdir.trim()),
                            });
                            close = true;
                        }
                    }
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });

        if close || response.should_close() {
            self.open = false;
        }
        request
    }

    fn validate(&self) -> Option<String> {
        if self.provider.is_none() {
            return Some("pick an agent".to_string());
        }
        if self.task.trim().is_empty() {
            return Some("describe the task".to_string());
        }
        let dir = self.workdir.trim();
        if dir.is_empty() {
            return Some("a working directory is required".to_string());
        }
        if !std::path::Path::new(dir).is_dir() {
            // Caught here rather than after the workspace copy has begun.
            return Some(format!("'{dir}' is not a directory"));
        }
        None
    }
}

/// Fields for `File > New Project`.
#[derive(Default)]
pub struct NewProjectDialog {
    open: bool,
    name: String,
    description: String,
    stack: String,
    error: Option<String>,
}

/// What the user typed, once Create is pressed.
pub struct NewProjectDraft {
    pub name: String,
    pub description: String,
    pub stack: String,
}

impl NewProjectDialog {
    /// Open with empty fields - a previous cancel must not leak into the next one.
    pub fn open(&mut self) {
        *self = Self {
            open: true,
            ..Default::default()
        };
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Draw the modal. Returns the draft only when Create was pressed and the
    /// name is non-empty.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<NewProjectDraft> {
        if !self.open {
            return None;
        }
        let mut result = None;
        let response = egui::Modal::new(egui::Id::new("new-project-modal")).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.heading("New project");
            ui.add_space(8.0);

            egui::Grid::new("new-project-fields")
                .num_columns(2)
                .spacing([10.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Name");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.name)
                            .desired_width(300.0)
                            .hint_text("My Thing"),
                    );
                    ui.end_row();

                    ui.label("Description");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.description)
                            .desired_width(300.0)
                            .hint_text("one or two sentences"),
                    );
                    ui.end_row();

                    ui.label("Stack");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.stack)
                            .desired_width(300.0)
                            .hint_text("comma separated, e.g. rust,egui"),
                    );
                    ui.end_row();
                });

            if let Some(err) = &self.error {
                ui.add_space(6.0);
                ui.colored_label(egui::Color32::from_rgb(0xc6, 0x28, 0x28), err);
            }

            ui.add_space(6.0);
            ui.weak(
                "Creates the directory, writes maestro-project.yaml and runs git init. \
                 The clarification interview is a separate step.",
            );
            ui.add_space(10.0);

            ui.horizontal(|ui| {
                if ui.button("Create").clicked() {
                    if self.name.trim().is_empty() {
                        self.error = Some("name is required".to_string());
                    } else {
                        result = Some(NewProjectDraft {
                            name: self.name.trim().to_string(),
                            description: self.description.trim().to_string(),
                            stack: self.stack.trim().to_string(),
                        });
                        self.open = false;
                    }
                }
                if ui.button("Cancel").clicked() {
                    self.open = false;
                }
            });
        });

        // Backdrop click or Esc.
        if response.should_close() {
            self.open = false;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warn_must_sit_below_critical() {
        assert!(validate_thresholds(80, 95).is_none());
        assert!(validate_thresholds(94, 95).is_none());
        assert!(validate_thresholds(95, 95).is_some());
        assert!(validate_thresholds(96, 95).is_some());
    }

    /// A critical threshold of 100 % means the alert never fires, so any warn
    /// value is consistent with it - including 100.
    #[test]
    fn a_critical_of_one_hundred_disables_the_rule() {
        assert!(validate_thresholds(100, 100).is_none());
        assert!(validate_thresholds(80, 100).is_none());
    }

    fn form(ptype: ProviderType) -> ProviderForm {
        ProviderForm {
            id: "test".into(),
            ptype,
            ..Default::default()
        }
    }

    #[test]
    fn an_id_is_required_and_cannot_contain_spaces() {
        let mut f = form(ProviderType::OllamaLocal);
        f.id = "  ".into();
        assert!(validate_provider(&f).is_some());
        f.id = "two words".into();
        assert!(validate_provider(&f).is_some());
        f.id = "ollama-local".into();
        assert!(validate_provider(&f).is_none());
    }

    /// api and ollama-remote have nowhere to connect without one; local and
    /// cloud ollama fall back to their well-known bases.
    #[test]
    fn only_some_types_require_an_endpoint() {
        assert!(validate_provider(&form(ProviderType::Api)).is_some());
        assert!(validate_provider(&form(ProviderType::OllamaRemote)).is_some());
        assert!(validate_provider(&form(ProviderType::OllamaLocal)).is_none());
        assert!(validate_provider(&form(ProviderType::OllamaCloud)).is_none());

        let mut f = form(ProviderType::Api);
        f.endpoint = "https://api.example.com/v1".into();
        assert!(validate_provider(&f).is_none());
    }

    /// The TUI registers cli providers with no path, and run_cli_session then
    /// refuses to run them. Reject the form instead of saving a dead entry.
    #[test]
    fn a_cli_provider_must_have_a_binary_path() {
        let mut f = form(ProviderType::Cli);
        assert!(validate_provider(&f).is_some());
        f.cli_path = "C:/tools/claude.cmd".into();
        assert!(validate_provider(&f).is_none());
    }
}
