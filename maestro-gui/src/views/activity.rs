//! Activity: live output from a wrapped CLI agent, and what it wrote.
//!
//! The adapter streams PTY chunks as they arrive, so this is the one place in
//! the app where work is visible while it happens rather than after it ends.

use eframe::egui;

use super::fmt_tokens;
use crate::app::MaestroApp;
use crate::jobs;

pub fn show(app: &mut MaestroApp, ui: &mut egui::Ui) {
    let running = app.jobs.is_running(jobs::CLI_TASK);

    ui.horizontal(|ui| {
        if ui
            .add_enabled(!running, egui::Button::new("Run CLI task"))
            .clicked()
        {
            app.open_cli_task();
        }
        if running {
            ui.spinner();
            ui.weak("agent running - output appears as it arrives");
        }
        if !app.cli_output.is_empty() && !running && ui.button("Clear").clicked() {
            app.cli_output.clear();
            app.cli_report = None;
        }
    });

    ui.add_space(8.0);

    if let Some(report) = &app.cli_report {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width() - 8.0);
            ui.horizontal(|ui| {
                match report.exit_code {
                    Some(0) => {
                        ui.colored_label(egui::Color32::from_rgb(0x2e, 0x7d, 0x32), "exit 0");
                    }
                    Some(code) => {
                        ui.colored_label(
                            egui::Color32::from_rgb(0xc6, 0x28, 0x28),
                            format!("exit {code}"),
                        );
                    }
                    None => {
                        ui.weak("no exit code");
                    }
                }
                ui.strong(&report.session_id);
                ui.weak(format!(
                    "{} applied · {} rejected · ~{} in / {} out",
                    report.applied,
                    report.rejected,
                    fmt_tokens(report.tokens_in_est),
                    fmt_tokens(report.tokens_out_est)
                ));
            });
            if !report.paths.is_empty() {
                ui.add_space(4.0);
                ui.weak("files written:");
                for p in &report.paths {
                    ui.label(format!("    {p}"));
                }
            }
            ui.add_space(4.0);
            ui.weak(format!("transcript: {}", report.transcript.display()));
        });
        ui.add_space(8.0);
    }

    if app.cli_output.is_empty() {
        ui.weak(
            "No session output yet. \"Run CLI task\" wraps a registered cli agent in a PTY and \
             streams what it prints.",
        );
        return;
    }

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show(ui, |ui| {
            ui.add(
                egui::Label::new(egui::RichText::new(&app.cli_output).monospace())
                    .wrap_mode(egui::TextWrapMode::Extend),
            );
        });
}
