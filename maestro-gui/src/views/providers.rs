//! Providers: the registry as a table, a detail pane, and the two actions that
//! must go through the job system because they hit the network.

use eframe::egui;
use egui_extras::{Column, TableBuilder};

use super::{fmt_cost, fmt_tokens};
use crate::app::MaestroApp;
use crate::jobs;

pub fn show(app: &mut MaestroApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        let detecting = app.jobs.is_running(jobs::DETECT);
        if ui
            .add_enabled(!detecting, egui::Button::new("Detect CLIs & Ollama"))
            .on_hover_text(
                "Probes local Ollama and looks for claude/codex/gemini/kimi on PATH, \
                 then merges what it finds into the registry.",
            )
            .clicked()
        {
            app.jobs.spawn(jobs::DETECT, jobs::detect_and_register);
        }

        let checking = app.jobs.is_running(jobs::HEALTH_CHECK);
        if ui
            .add_enabled(!checking, egui::Button::new("Health check all"))
            .on_hover_text(
                "One HTTP round trip per provider. An unreachable endpoint costs \
                 up to 30 s, so this runs on a worker thread.",
            )
            .clicked()
        {
            app.jobs.spawn(jobs::HEALTH_CHECK, jobs::health_check_all);
        }

        if ui.button("Refresh").clicked() {
            app.poller.refresh_now();
        }

        if detecting || checking {
            ui.spinner();
        }
    });

    ui.add_space(8.0);

    if app.snap.providers.is_empty() {
        ui.weak("No providers registered. Try \"Detect CLIs & Ollama\".");
        return;
    }

    // The detail pane is driven by this selection; keep a copy so the table
    // closure can mutate the selection without holding a borrow on the snapshot.
    let mut selected = app.selected_provider.clone();
    let rows = app.snap.providers.clone();
    let health = app.health.clone();

    // The detail pane is a real nested side panel rather than a second column
    // in a horizontal layout: the table's remainder column expands to fill
    // whatever width it is given, so a sibling column gets pushed off-screen.
    // Side panels reserve their space first, which is exactly what is wanted.
    if let Some(id) = selected.clone() {
        if let Some(p) = rows.iter().find(|p| p.id == id) {
            egui::Panel::right("provider-detail")
                .resizable(true)
                .default_size(300.0)
                .min_size(240.0)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        detail(ui, p, health.get(&id));
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
                .column(Column::initial(150.0).at_least(90.0))
                .column(Column::initial(100.0).at_least(70.0))
                .column(Column::initial(150.0).at_least(90.0))
                .column(Column::initial(70.0).at_least(50.0))
                .column(Column::initial(70.0).at_least(50.0))
                .column(Column::remainder().at_least(80.0))
                .header(22.0, |mut header| {
                    for title in ["Provider", "Type", "Status", "Models", "Quota", "Today"] {
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
                                if ui.selectable_label(is_sel, &p.id).clicked() {
                                    selected = Some(p.id.clone());
                                }
                            });
                            row.col(|ui| {
                                ui.label(&p.kind);
                            });
                            row.col(|ui| match health.get(&p.id) {
                                Some(h) if h.ok => {
                                    ui.colored_label(
                                        egui::Color32::from_rgb(0x2e, 0x7d, 0x32),
                                        format!("ok · {} ms", h.latency_ms),
                                    );
                                }
                                Some(h) => {
                                    ui.colored_label(
                                        egui::Color32::from_rgb(0xc6, 0x28, 0x28),
                                        "failed",
                                    )
                                    .on_hover_text(&h.detail);
                                }
                                None => {
                                    ui.weak("unchecked");
                                }
                            });
                            row.col(|ui| {
                                ui.label(p.models.to_string());
                            });
                            row.col(|ui| match p.quota_pct {
                                Some(pct) => {
                                    ui.label(format!("{pct}%"));
                                }
                                None => {
                                    ui.weak("-");
                                }
                            });
                            row.col(|ui| {
                                ui.label(fmt_tokens(p.tokens_today));
                            });
                        });
                    }
                });
        });

    app.selected_provider = selected;
}

fn detail(ui: &mut egui::Ui, p: &crate::state::ProviderRow, health: Option<&jobs::HealthRow>) {
    ui.heading(&p.id);
    ui.add_space(4.0);

    egui::Grid::new("provider-detail")
        .num_columns(2)
        .spacing([12.0, 4.0])
        .show(ui, |ui| {
            ui.weak("type");
            ui.label(&p.kind);
            ui.end_row();

            ui.weak("endpoint");
            match &p.endpoint {
                Some(e) => {
                    ui.label(e);
                }
                None => {
                    ui.weak(if p.is_cli { "n/a (PTY-wrapped)" } else { "-" });
                }
            }
            ui.end_row();

            ui.weak("key");
            ui.label(if p.has_key {
                "stored in keychain/vault"
            } else {
                "none"
            });
            ui.end_row();

            ui.weak("models");
            ui.label(p.models.to_string());
            ui.end_row();

            ui.weak("quota left");
            match p.quota_pct {
                Some(pct) => {
                    ui.label(format!("{pct}%"));
                }
                None => {
                    ui.weak("no plan limit set");
                }
            }
            ui.end_row();

            ui.weak("today");
            ui.label(format!(
                "{} tok · {}",
                fmt_tokens(p.tokens_today),
                fmt_cost(p.cost_today)
            ));
            ui.end_row();
        });

    if let Some(h) = health {
        ui.add_space(6.0);
        ui.separator();
        ui.weak("last health check");
        ui.label(format!(
            "{} · {} ms · {} models",
            if h.ok { "ok" } else { "failed" },
            h.latency_ms,
            h.models
        ));
        if !h.detail.is_empty() {
            ui.label(&h.detail);
        }
    }
}
