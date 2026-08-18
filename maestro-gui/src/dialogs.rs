//! Modal dialogs.
//!
//! Dialogs hold their own field state and return a value only when the user
//! confirms, so the app never has to reach into half-typed input.

use eframe::egui;
use maestro_core::types::QuestionMode;

use crate::jobs::ConfigForm;

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
}
