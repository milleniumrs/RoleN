//! Dashboard: today's spend, the write queue's ledger totals, and recent
//! sessions. Everything here comes from the poller's snapshot - this function
//! performs no IO of its own.

use eframe::egui;
use egui_extras::{Column, TableBuilder};

use super::{fmt_cost, fmt_tokens};
use crate::app::RoleNApp;

pub fn show(app: &mut RoleNApp, ui: &mut egui::Ui) {
    let snap = &app.snap;

    ui.horizontal_wrapped(|ui| {
        card(ui, "Providers", |ui| {
            ui.heading(snap.providers.len().to_string());
            ui.label(format!("{} models discovered", snap.total_models()));
        });
        card(ui, "Today", |ui| {
            ui.heading(fmt_cost(snap.today.cost));
            ui.label(format!(
                "{} in / {} out",
                fmt_tokens(snap.today.tokens_in),
                fmt_tokens(snap.today.tokens_out)
            ));
            ui.label(format!("{} requests", snap.today.requests));
        });
        card(ui, "Write tickets", |ui| {
            ui.heading(snap.tickets.applied.to_string());
            ui.label(format!("{} rejected", snap.tickets.rejected));
            ui.label(format!("{} queued", snap.tickets.queued));
        });
        card(ui, "Sessions", |ui| {
            ui.heading(snap.running_sessions.to_string());
            ui.label("running");
        });
    });

    ui.add_space(12.0);
    ui.separator();
    ui.add_space(6.0);
    ui.strong("Recent sessions");
    ui.add_space(4.0);

    if snap.sessions.is_empty() {
        ui.weak("No sessions recorded yet.");
        return;
    }

    TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::initial(210.0).at_least(120.0))
        .column(Column::initial(120.0).at_least(80.0))
        .column(Column::initial(190.0).at_least(90.0))
        .column(Column::initial(80.0).at_least(60.0))
        .column(Column::initial(80.0).at_least(60.0))
        .column(Column::remainder().at_least(90.0))
        .header(22.0, |mut header| {
            for title in ["Session", "Provider", "Model", "State", "Tokens", "Started"] {
                header.col(|ui| {
                    ui.strong(title);
                });
            }
        })
        .body(|mut body| {
            for s in &snap.sessions {
                body.row(20.0, |mut row| {
                    row.col(|ui| {
                        ui.label(&s.id);
                    });
                    row.col(|ui| {
                        ui.label(&s.provider_id);
                    });
                    row.col(|ui| {
                        ui.label(&s.model);
                    });
                    row.col(|ui| {
                        ui.label(&s.state);
                    });
                    row.col(|ui| {
                        ui.label(fmt_tokens(s.tokens));
                    });
                    row.col(|ui| {
                        ui.label(s.started.format("%Y-%m-%d %H:%M").to_string());
                    });
                });
            }
        });
}

/// A titled box in the summary strip.
fn card(ui: &mut egui::Ui, title: &str, contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_min_width(170.0);
        ui.vertical(|ui| {
            ui.weak(title);
            contents(ui);
        });
    });
}
