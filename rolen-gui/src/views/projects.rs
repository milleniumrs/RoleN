//! Projects: the workspace scan as a table, with a detail pane.
//!
//! Creation is wired up; the interview and PRD/DAG build are not, because they
//! are multi-minute LLM calls with no progress or cancellation support in the
//! core yet. The detail pane says so per project rather than offering a button
//! that would freeze on a spinner for ten minutes.

use eframe::egui;
use egui_extras::{Column, TableBuilder};

use crate::app::RoleNApp;
use crate::jobs;

pub fn show(app: &mut RoleNApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        let creating = app.jobs.is_running(jobs::NEW_PROJECT);
        if ui
            .add_enabled(!creating, egui::Button::new("New project"))
            .clicked()
        {
            app.new_project.open();
        }
        if ui.button("Refresh").clicked() {
            app.poller.refresh_now();
        }
        if creating {
            ui.spinner();
        }
        if let Some(root) = &app.snap.workspace_root {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.weak(root.display().to_string());
            });
        }
    });

    ui.add_space(8.0);

    if app.snap.projects.is_empty() {
        ui.weak("No projects in the workspace root yet. Use \"New project\".");
        return;
    }

    let mut selected = app.selected_project.clone();
    let rows = app.snap.projects.clone();

    if let Some(id) = selected.clone() {
        if let Some(p) = rows.iter().find(|p| p.id == id) {
            egui::Panel::right("project-detail")
                .resizable(true)
                .default_size(320.0)
                .min_size(260.0)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        detail(ui, p);
                        ui.add_space(8.0);
                        if ui.button("Close").clicked() {
                            selected = None;
                        }
                    });
                });
        }
    }

    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            TableBuilder::new(ui)
                .striped(true)
                .resizable(true)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(Column::initial(170.0).at_least(100.0))
                .column(Column::initial(150.0).at_least(80.0))
                .column(Column::initial(50.0).at_least(40.0))
                .column(Column::initial(60.0).at_least(50.0))
                .column(Column::initial(90.0).at_least(70.0))
                .column(Column::remainder().at_least(60.0))
                .header(22.0, |mut header| {
                    for title in ["Name", "Stack", "PRD", "AGENTS", "Questions", "Skills"] {
                        header.col(|ui| {
                            ui.strong(title);
                        });
                    }
                })
                .body(|mut body| {
                    for p in &rows {
                        body.row(20.0, |mut row| {
                            row.col(|ui| {
                                let is_sel = selected.as_deref() == Some(p.id.as_str());
                                if ui.selectable_label(is_sel, &p.name).clicked() {
                                    selected = Some(p.id.clone());
                                }
                            });
                            row.col(|ui| {
                                ui.label(p.stack.join(", "));
                            });
                            row.col(|ui| {
                                ui.label(tick(p.has_prd));
                            });
                            row.col(|ui| {
                                ui.label(tick(p.has_agents));
                            });
                            row.col(|ui| {
                                if p.pending > 0 {
                                    ui.label(format!("{} pending", p.pending));
                                } else if p.clarifications > 0 {
                                    ui.weak(format!("{} answered", p.clarifications));
                                } else {
                                    ui.weak("-");
                                }
                            });
                            row.col(|ui| {
                                ui.label(p.skills.to_string());
                            });
                        });
                    }
                });
        });

    app.selected_project = selected;
}

fn tick(present: bool) -> &'static str {
    if present {
        "yes"
    } else {
        "-"
    }
}

fn detail(ui: &mut egui::Ui, p: &crate::state::ProjectRow) {
    ui.heading(&p.name);
    ui.add_space(4.0);

    egui::Grid::new("project-detail-grid")
        .num_columns(2)
        .spacing([12.0, 4.0])
        .show(ui, |ui| {
            ui.weak("id");
            ui.label(&p.id);
            ui.end_row();

            ui.weak("stack");
            if p.stack.is_empty() {
                ui.weak("-");
            } else {
                ui.label(p.stack.join(", "));
            }
            ui.end_row();

            ui.weak("PRD");
            ui.label(if p.has_prd {
                "PRD.md present"
            } else {
                "not generated"
            });
            ui.end_row();

            ui.weak("AGENTS.md");
            ui.label(if p.has_agents {
                "present"
            } else {
                "not generated"
            });
            ui.end_row();

            ui.weak("clarifications");
            ui.label(format!("{} total, {} pending", p.clarifications, p.pending));
            ui.end_row();

            ui.weak("skills");
            ui.label(p.skills.to_string());
            ui.end_row();
        });

    if !p.description.is_empty() {
        ui.add_space(6.0);
        ui.weak("description");
        ui.label(&p.description);
    }

    ui.add_space(6.0);
    ui.weak("path");
    ui.label(p.dir.display().to_string());

    ui.add_space(8.0);
    ui.separator();
    ui.weak("Next steps are CLI-only for now:");
    ui.code(format!("rolen project interview --name {}", p.id));
    ui.code(format!("rolen project build --name {}", p.id));
}
