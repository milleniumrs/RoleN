//! Dashboard: today's spend, the write queue's ledger totals, and recent
//! sessions. Everything here comes from the poller's snapshot - this function
//! performs no IO of its own.

use dear_imgui_rs::{TableFlags, Ui};

use super::{fmt_cost, fmt_tokens};
use crate::app::RoleNApp;

pub fn show(app: &mut RoleNApp, ui: &Ui) {
    let snap = &app.snap;

    card(ui, "card-providers", "Providers", || {
        ui.text(snap.providers.len().to_string());
        ui.text_disabled(format!("{} models discovered", snap.total_models()));
    });
    ui.same_line();
    card(ui, "card-today", "Today", || {
        ui.text(fmt_cost(snap.today.cost));
        ui.text_disabled(format!(
            "{} in / {} out",
            fmt_tokens(snap.today.tokens_in),
            fmt_tokens(snap.today.tokens_out)
        ));
        ui.text_disabled(format!("{} requests", snap.today.requests));
    });
    ui.same_line();
    card(ui, "card-tickets", "Write tickets", || {
        ui.text(snap.tickets.applied.to_string());
        ui.text_disabled(format!("{} rejected", snap.tickets.rejected));
        ui.text_disabled(format!("{} queued", snap.tickets.queued));
    });
    ui.same_line();
    card(ui, "card-sessions", "Sessions", || {
        ui.text(snap.running_sessions.to_string());
        ui.text_disabled("running");
    });

    ui.spacing();
    ui.separator();
    ui.spacing();
    ui.text("Recent sessions");
    ui.spacing();

    if snap.sessions.is_empty() {
        ui.text_disabled("No sessions recorded yet.");
        return;
    }

    ui.table("sessions-table")
        .flags(TableFlags::RESIZABLE | TableFlags::ROW_BG | TableFlags::BORDERS)
        .column("Session")
        .done()
        .column("Provider")
        .done()
        .column("Model")
        .done()
        .column("State")
        .done()
        .column("Tokens")
        .done()
        .column("Started")
        .done()
        .headers(true)
        .build(|ui| {
            for s in &snap.sessions {
                ui.table_next_row();
                ui.table_next_column();
                ui.text(&s.id);
                ui.table_next_column();
                ui.text(&s.provider_id);
                ui.table_next_column();
                ui.text(&s.model);
                ui.table_next_column();
                ui.text(&s.state);
                ui.table_next_column();
                ui.text(fmt_tokens(s.tokens));
                ui.table_next_column();
                ui.text(s.started.format("%Y-%m-%d %H:%M").to_string());
            }
        });
}

/// A titled box in the summary strip.
fn card(ui: &Ui, id: &str, title: &str, contents: impl FnOnce()) {
    ui.child_window(id)
        .size([180.0, 92.0])
        .border(true)
        .build(ui, || {
            ui.text_disabled(title);
            ui.separator();
            contents();
        });
}
