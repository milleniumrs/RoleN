//! Modal dialogs.
//!
//! Dialogs hold their own field state and return a value only when the user
//! confirms, so the app never has to reach into half-typed input.

use eframe::egui;

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
