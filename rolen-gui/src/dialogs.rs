//! Modal dialogs.
//!
//! Dialogs hold their own field state and return a value only when the user
//! confirms, so the app never has to reach into half-typed input.
//!
//! ImGui popup modals need `open_popup` exactly once, on the first frame the
//! dialog is shown, and `begin_modal_popup` every frame after that; the token
//! returning `None` means the popup is closed. [`ModalState`] keeps that
//! bookkeeping out of the individual dialogs.

use dear_imgui_rs::popup::ModalPopupToken;
use dear_imgui_rs::{Condition, Ui};
use rolen_core::types::{ProviderType, QuestionMode};

use crate::jobs::{ConfigForm, ProviderForm};

/// open-popup-once / begin-every-frame bookkeeping for a modal.
#[derive(Default)]
pub struct ModalState {
    open: bool,
    pending_open: bool,
}

impl ModalState {
    pub fn open(&mut self) {
        self.open = true;
        self.pending_open = true;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.pending_open = false;
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Start the modal for this frame. `None` means "not open" - either the
    /// dialog was never opened or the popup has been closed and the dialog
    /// state has been reset.
    pub fn begin<'ui>(&mut self, ui: &'ui Ui, id: &str) -> Option<ModalPopupToken<'ui>> {
        if !self.open {
            return None;
        }
        if self.pending_open {
            ui.open_popup(id);
            self.pending_open = false;
        }
        match ui.begin_modal_popup(id) {
            Some(token) => Some(token),
            None => {
                self.open = false;
                None
            }
        }
    }
}

/// What the Add Provider dialog wants done next.
pub enum ProviderRequest {
    Discover(ProviderForm),
    Save(ProviderForm),
}

/// `Providers > Add Provider`.
#[derive(Default)]
pub struct AddProviderDialog {
    modal: ModalState,
    form: ProviderForm,
    error: Option<String>,
    note: Option<String>,
    busy: bool,
}

impl AddProviderDialog {
    pub fn open(&mut self) {
        *self = Self::default();
        self.modal.open();
    }

    /// Open pre-filled to edit an existing provider. The key is never shown -
    /// it lives in the keychain and cannot be read back into the form.
    pub fn edit(&mut self, form: ProviderForm) {
        *self = Self {
            form,
            ..Default::default()
        };
        self.modal.open();
    }

    pub fn is_open(&self) -> bool {
        self.modal.is_open()
    }

    pub fn set_busy(&mut self, busy: bool) {
        self.busy = busy;
    }

    pub fn discovered(&mut self, models: Vec<rolen_core::types::Model>) {
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
        self.modal.close();
    }

    pub fn show(&mut self, ui: &Ui) -> Option<ProviderRequest> {
        let mut request = None;
        let _modal = self.modal.begin(ui, "Add provider")?;
        // The form is dense; give it room instead of imgui's auto-size.
        ui.set_window_size_with_cond([600.0, 0.0], Condition::FirstUseEver);

        ui.text("Id");
        ui.same_line();
        ui.set_cursor_pos_x(LABEL_COL);
        ui.set_next_item_width(FIELD_W);
        ui.input_text("##provider-id", &mut self.form.id)
            .hint("unique, e.g. kimi or ollama-local")
            .build();

        ui.text("Type");
        ui.same_line();
        ui.set_cursor_pos_x(LABEL_COL);
        ui.set_next_item_width(FIELD_W);
        let mut picked_type = None;
        if let Some(combo) = ui.begin_combo("##provider-type", type_label(self.form.ptype)) {
            for t in [
                ProviderType::Api,
                ProviderType::Cli,
                ProviderType::OllamaLocal,
                ProviderType::OllamaCloud,
                ProviderType::OllamaRemote,
            ] {
                if ui
                    .selectable_config(type_label(t))
                    .selected(self.form.ptype == t)
                    .build()
                {
                    picked_type = Some(t);
                }
            }
            combo.end();
        }
        if let Some(t) = picked_type {
            self.form.ptype = t;
            // Ollama's bases are known, so offer them instead of making the
            // user recall a URL.
            if self.form.endpoint.trim().is_empty() {
                self.form.endpoint = match t {
                    ProviderType::OllamaLocal => rolen_providers::ollama::DEFAULT_LOCAL_BASE.into(),
                    ProviderType::OllamaCloud => rolen_providers::ollama::DEFAULT_CLOUD_BASE.into(),
                    _ => String::new(),
                };
            }
        }

        if self.form.ptype == ProviderType::Cli {
            ui.text("CLI path");
            ui.same_line();
            ui.set_cursor_pos_x(LABEL_COL);
            ui.set_next_item_width(FIELD_W);
            ui.input_text("##provider-cli-path", &mut self.form.cli_path)
                .hint("full path to the agent binary")
                .build();
        } else {
            ui.text("Endpoint");
            ui.same_line();
            ui.set_cursor_pos_x(LABEL_COL);
            ui.set_next_item_width(FIELD_W);
            ui.input_text("##provider-endpoint", &mut self.form.endpoint)
                .hint("https://api.example.com/v1")
                .build();

            ui.text("API key");
            ui.same_line();
            ui.set_cursor_pos_x(LABEL_COL);
            ui.set_next_item_width(FIELD_W);
            ui.input_text("##provider-key", &mut self.form.key)
                .password(true)
                .hint("stored in the OS keychain, never in providers.toml")
                .build();
        }

        ui.text("Models");
        ui.same_line();
        ui.set_cursor_pos_x(LABEL_COL);
        if self.form.models.is_empty() {
            ui.text_disabled("none yet - Discover asks the endpoint");
        } else {
            ui.text(format!("{} discovered", self.form.models.len()));
        }

        if let Some(note) = &self.note {
            ui.spacing();
            ui.text_disabled(note);
        }
        if let Some(err) = &self.error {
            ui.spacing();
            ui.text_colored(ERROR, err);
        }

        ui.spacing();
        ui.separator();
        ui.spacing();

        let is_cli = self.form.ptype == ProviderType::Cli;
        ui.with_disabled_if(is_cli || self.busy, || {
            if ui.button("Discover models") {
                match validate_provider(&self.form) {
                    Some(err) => self.error = Some(err),
                    None => {
                        self.error = None;
                        self.busy = true;
                        request = Some(ProviderRequest::Discover(self.form.clone()));
                    }
                }
            }
        });
        if is_cli
            && ui.is_item_hovered_with_flags(dear_imgui_rs::ItemHoveredFlags::ALLOW_WHEN_DISABLED)
        {
            ui.tooltip_text("a CLI agent has no /models endpoint");
        }
        ui.same_line();

        ui.with_disabled_if(self.busy, || {
            if ui.button("Save") {
                match validate_provider(&self.form) {
                    Some(err) => self.error = Some(err),
                    None => {
                        self.error = None;
                        self.busy = true;
                        request = Some(ProviderRequest::Save(self.form.clone()));
                    }
                }
            }
        });
        ui.same_line();

        if ui.button("Cancel") {
            ui.close_current_popup();
        }
        if self.busy {
            ui.same_line();
            ui.text_disabled("working...");
        }

        request
    }
}

/// Why a provider form cannot be submitted, if it cannot.
///
/// The CLI-path rule is stricter than the TUI, which registers `cli` providers
/// with `cli_path: None` (`rolen-tui/src/add_provider.rs:123`) - and
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

/// Shared form geometry: label column offset and field width.
const LABEL_COL: f32 = 130.0;
const FIELD_W: f32 = 420.0;

/// Error red, as `[r, g, b, a]` floats.
pub const ERROR: [f32; 4] = [0.78, 0.16, 0.16, 1.0];
/// Success green.
pub const OK: [f32; 4] = [0.18, 0.49, 0.20, 1.0];
/// Link-ish blue, for "you" in chat.
pub const ACCENT: [f32; 4] = [0.08, 0.40, 0.75, 1.0];

/// A label row followed by a field, used by every dialog form.
pub fn form_row(ui: &Ui, label: &str, field: impl FnOnce()) {
    ui.text(label);
    ui.same_line();
    ui.set_cursor_pos_x(LABEL_COL);
    field();
}

/// `Tools > Settings`.
///
/// Numeric fields are sliders rather than free text: the TUI parses them with
/// `parse()` and silently keeps the old value when parsing fails
/// (`rolen-tui/src/settings.rs:160`), so a typo looks like it saved.
#[derive(Default)]
pub struct SettingsDialog {
    modal: ModalState,
    form: Option<ConfigForm>,
    error: Option<String>,
}

impl SettingsDialog {
    /// Populate from a freshly loaded config and show.
    pub fn populate(&mut self, form: ConfigForm) {
        self.form = Some(form);
        self.error = None;
        self.modal.open();
    }

    pub fn is_open(&self) -> bool {
        self.modal.is_open()
    }

    pub fn close(&mut self) {
        self.modal.close();
    }

    /// Returns the form when Save is pressed and validation passes.
    pub fn show(&mut self, ui: &Ui) -> Option<ConfigForm> {
        self.form.as_ref()?;
        let mut result = None;
        let _modal = self.modal.begin(ui, "Settings")?;
        ui.set_window_size_with_cond([600.0, 0.0], Condition::FirstUseEver);
        // Both guards above proved the form exists.
        let form = self.form.as_mut()?;

        form_row(ui, "Workspace root", || {
            ui.set_next_item_width(FIELD_W);
            ui.input_text("##workspace-root", &mut form.workspace_root)
                .build();
        });

        form_row(ui, "Question mode", || {
            ui.set_next_item_width(FIELD_W);
            if let Some(combo) = ui.begin_combo("##question-mode", mode_label(form.question_mode)) {
                for mode in [
                    QuestionMode::Thorough,
                    QuestionMode::Balanced,
                    QuestionMode::Minimal,
                ] {
                    if ui
                        .selectable_config(mode_label(mode))
                        .selected(form.question_mode == mode)
                        .build()
                    {
                        form.question_mode = mode;
                    }
                }
                combo.end();
            }
        });

        let mut global_cap = form.global_cap as i32;
        form_row(ui, "Global cap", || {
            ui.set_next_item_width(FIELD_W);
            if ui.slider_i32("##global-cap", &mut global_cap, 0, 64) {
                form.global_cap = global_cap as usize;
            }
        });
        ui.same_line();
        ui.text_disabled("0 = automatic (half the logical CPUs, min 2)");

        let mut per_provider_cap = form.per_provider_cap as i32;
        form_row(ui, "Per-provider cap", || {
            ui.set_next_item_width(FIELD_W);
            if ui.slider_i32("##per-provider-cap", &mut per_provider_cap, 1, 64) {
                form.per_provider_cap = per_provider_cap as usize;
            }
        });

        let mut warn_pct = form.warn_pct as i32;
        form_row(ui, "Quota warn %", || {
            ui.set_next_item_width(FIELD_W);
            if ui.slider_i32("##warn-pct", &mut warn_pct, 0, 100) {
                form.warn_pct = warn_pct as u8;
            }
        });

        let mut crit_pct = form.crit_pct as i32;
        form_row(ui, "Quota critical %", || {
            ui.set_next_item_width(FIELD_W);
            if ui.slider_i32("##crit-pct", &mut crit_pct, 0, 100) {
                form.crit_pct = crit_pct as u8;
            }
        });

        ui.spacing();
        ui.text_disabled(
            "The TUI colour theme and the quota alert action are stored in the same file \
             and are left untouched by this form.",
        );

        if let Some(err) = &self.error {
            ui.spacing();
            ui.text_colored(ERROR, err);
        }

        ui.spacing();
        ui.separator();
        ui.spacing();

        if ui.button("Save") {
            match validate_thresholds(form.warn_pct, form.crit_pct) {
                Some(err) => self.error = Some(err),
                None => {
                    result = Some(form.clone());
                    ui.close_current_popup();
                }
            }
        }
        ui.same_line();
        if ui.button("Cancel") {
            ui.close_current_popup();
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
    modal: ModalState,
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
            provider: providers.first().cloned(),
            workdir: default_workdir,
            ..Default::default()
        };
        self.modal.open();
    }

    pub fn is_open(&self) -> bool {
        self.modal.is_open()
    }

    pub fn show(&mut self, ui: &Ui, providers: &[String]) -> Option<CliTaskRequest> {
        let mut request = None;
        let _modal = self.modal.begin(ui, "Run CLI task")?;
        ui.set_window_size_with_cond([620.0, 0.0], Condition::FirstUseEver);

        if providers.is_empty() {
            ui.text_colored(ERROR, "No cli providers are registered with a binary path.");
        }

        form_row(ui, "Agent", || {
            ui.set_next_item_width(FIELD_W);
            let preview = self.provider.clone().unwrap_or_else(|| "-".into());
            if let Some(combo) = ui.begin_combo("##cli-task-provider", preview) {
                for id in providers {
                    if ui
                        .selectable_config(id)
                        .selected(self.provider.as_deref() == Some(id))
                        .build()
                    {
                        self.provider = Some(id.clone());
                    }
                }
                combo.end();
            }
        });

        form_row(ui, "Working dir", || {
            ui.set_next_item_width(FIELD_W);
            ui.input_text("##cli-task-workdir", &mut self.workdir)
                .build();
        });

        ui.text("Task");
        ui.set_next_item_width(FIELD_W);
        ui.input_text_multiline(
            "##cli-task-task",
            &mut self.task,
            [FIELD_W + LABEL_COL, 110.0],
        )
        .build();

        ui.spacing();
        ui.text_disabled(
            "The workspace is copied to a staging directory, the agent runs there, and its \
             changes come back through the write queue.",
        );

        if let Some(err) = &self.error {
            ui.spacing();
            ui.text_colored(ERROR, err);
        }

        ui.spacing();
        ui.separator();
        ui.spacing();

        if ui.button("Run") {
            match self.validate() {
                Some(err) => self.error = Some(err),
                None => {
                    request = Some(CliTaskRequest {
                        provider: self.provider.clone().unwrap_or_default(),
                        task: self.task.trim().to_string(),
                        workdir: std::path::PathBuf::from(self.workdir.trim()),
                    });
                    ui.close_current_popup();
                }
            }
        }
        ui.same_line();
        if ui.button("Cancel") {
            ui.close_current_popup();
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
    modal: ModalState,
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
        *self = Self::default();
        self.modal.open();
    }

    pub fn is_open(&self) -> bool {
        self.modal.is_open()
    }

    /// Draw the modal. Returns the draft only when Create was pressed and the
    /// name is non-empty.
    pub fn show(&mut self, ui: &Ui) -> Option<NewProjectDraft> {
        let mut result = None;
        let _modal = self.modal.begin(ui, "New project")?;
        ui.set_window_size_with_cond([560.0, 0.0], Condition::FirstUseEver);

        form_row(ui, "Name", || {
            ui.set_next_item_width(FIELD_W);
            ui.input_text("##project-name", &mut self.name)
                .hint("My Thing")
                .build();
        });
        form_row(ui, "Description", || {
            ui.set_next_item_width(FIELD_W);
            ui.input_text("##project-description", &mut self.description)
                .hint("one or two sentences")
                .build();
        });
        form_row(ui, "Stack", || {
            ui.set_next_item_width(FIELD_W);
            ui.input_text("##project-stack", &mut self.stack)
                .hint("comma separated, e.g. rust,imgui")
                .build();
        });

        if let Some(err) = &self.error {
            ui.spacing();
            ui.text_colored(ERROR, err);
        }

        ui.spacing();
        ui.text_disabled(
            "Creates the directory, writes rolen-project.yaml and runs git init. \
             The clarification interview is a separate step.",
        );
        ui.spacing();
        ui.separator();
        ui.spacing();

        if ui.button("Create") {
            if self.name.trim().is_empty() {
                self.error = Some("name is required".to_string());
            } else {
                result = Some(NewProjectDraft {
                    name: self.name.trim().to_string(),
                    description: self.description.trim().to_string(),
                    stack: self.stack.trim().to_string(),
                });
                ui.close_current_popup();
            }
        }
        ui.same_line();
        if ui.button("Cancel") {
            ui.close_current_popup();
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
