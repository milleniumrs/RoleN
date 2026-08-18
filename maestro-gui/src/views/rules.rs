//! Rules: the routing table, and a dry-run that answers "where would this role
//! go right now?".
//!
//! Editing is still CLI-only. The dry-run is here because it is the part that
//! needs live provider state, and therefore the part a static YAML file cannot
//! answer on its own.

use eframe::egui;
use egui_extras::{Column, TableBuilder};

use crate::app::MaestroApp;
use crate::jobs::{self, DryRun};

pub fn show(app: &mut MaestroApp, ui: &mut egui::Ui) {
    let running = app.jobs.is_running(jobs::DRY_RUN);

    ui.horizontal(|ui| {
        ui.label("Role");
        egui::ComboBox::from_id_salt("dry-run-role")
            .selected_text(app.dry_run_role.clone())
            .show_ui(ui, |ui| {
                // Built-in roles plus anything the rule file actually mentions,
                // so a custom role is still reachable.
                let mut roles: Vec<String> = maestro_core::rules::BUILT_IN_ROLES
                    .iter()
                    .map(|r| r.to_string())
                    .collect();
                for rule in &app.snap.rules {
                    if !roles.contains(&rule.role) {
                        roles.push(rule.role.clone());
                    }
                }
                for role in roles {
                    ui.selectable_value(&mut app.dry_run_role, role.clone(), role);
                }
            });

        if ui
            .add_enabled(!running, egui::Button::new("Dry-run"))
            .on_hover_text(
                "Health-checks every provider, then evaluates the rules against \
                 what is actually reachable.",
            )
            .clicked()
        {
            app.start_dry_run();
        }

        if running {
            ui.spinner();
            if ui
                .button("Cancel")
                .on_hover_text(
                    "Stops before the next provider. The request already in flight \
                     cannot be aborted, so this can still take up to its timeout.",
                )
                .clicked()
            {
                app.cancel_dry_run();
            }
            if let Some(progress) = &app.dry_run_progress {
                ui.weak(progress);
            }
        }
    });

    ui.add_space(8.0);

    if let Some(outcome) = app.dry_run_result.clone() {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width() - 8.0);
            outcome_ui(ui, &outcome);
        });
        ui.add_space(8.0);
    }

    if app.snap.rules.is_empty() {
        ui.weak("No rules yet. Seed them from your providers with: maestro rule init");
        return;
    }

    TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::initial(160.0).at_least(100.0))
        .column(Column::initial(110.0).at_least(80.0))
        .column(Column::initial(50.0).at_least(40.0))
        .column(Column::initial(70.0).at_least(50.0))
        .column(Column::remainder().at_least(180.0))
        .header(22.0, |mut header| {
            for title in ["Rule", "Role", "Prio", "Min quota", "Fallback chain"] {
                header.col(|ui| {
                    ui.strong(title);
                });
            }
        })
        .body(|mut body| {
            for r in &app.snap.rules {
                body.row(20.0, |mut row| {
                    row.col(|ui| {
                        let label = ui.label(&r.id);
                        if r.conditions > 0 || r.project_scope.is_some() {
                            label.on_hover_text(format!(
                                "{} condition(s){}",
                                r.conditions,
                                match &r.project_scope {
                                    Some(s) => format!("; scoped to project '{s}'"),
                                    None => String::new(),
                                }
                            ));
                        }
                    });
                    row.col(|ui| {
                        ui.label(&r.role);
                    });
                    row.col(|ui| {
                        ui.label(r.priority.to_string());
                    });
                    row.col(|ui| match r.min_quota_pct {
                        Some(pct) => {
                            ui.label(format!("{pct}%"));
                        }
                        None => {
                            ui.weak("-");
                        }
                    });
                    row.col(|ui| {
                        ui.label(r.chain.join("  ->  "));
                    });
                });
            }
        });
}

fn outcome_ui(ui: &mut egui::Ui, outcome: &DryRun) {
    // Core writes these strings for a terminal; see crate::text. The context is
    // cloned so the closure does not hold a borrow on `ui`.
    let fctx = ui.ctx().clone();
    let fix = |s: &str| crate::text::renderable(&fctx, s);
    match outcome {
        DryRun::Decided {
            role,
            rule_id,
            provider,
            model,
            explanation,
            skipped,
        } => {
            ui.horizontal(|ui| {
                ui.strong(format!("{role} ->"));
                ui.colored_label(
                    egui::Color32::from_rgb(0x2e, 0x7d, 0x32),
                    format!("{provider}/{model}"),
                );
                ui.weak(format!("via rule '{rule_id}'"));
            });
            if !explanation.is_empty() {
                ui.label(fix(explanation));
            }
            if !skipped.is_empty() {
                ui.add_space(4.0);
                ui.weak("skipped:");
                for (entry, reason) in skipped {
                    ui.weak(fix(&format!("    {entry} - {reason}")));
                }
            }
        }
        DryRun::NoRoute { role, reason } => {
            ui.colored_label(
                egui::Color32::from_rgb(0xc6, 0x28, 0x28),
                format!("no route for '{role}'"),
            );
            ui.label(fix(reason));
        }
        DryRun::Cancelled => {
            ui.weak("dry-run cancelled");
        }
    }
}
