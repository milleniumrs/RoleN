//! Activity: live output from a wrapped CLI agent, and what it wrote.
//!
//! The adapter streams PTY chunks as they arrive, so this is the one place in
//! the app where work is visible while it happens rather than after it ends.

use dear_imgui_rs::Ui;

use super::fmt_tokens;
use crate::app::RoleNApp;
use crate::dialogs::{ERROR, OK};
use crate::jobs;

pub fn show(app: &mut RoleNApp, ui: &Ui) {
    let running = app.jobs.is_running(jobs::CLI_TASK);

    ui.with_disabled_if(running, || {
        if ui.button("Run CLI task") {
            app.open_cli_task();
        }
    });
    if running {
        ui.same_line();
        ui.text_disabled("agent running - output appears as it arrives");
    }
    if !app.cli_output.is_empty() && !running {
        ui.same_line();
        if ui.button("Clear") {
            app.cli_output.clear();
            app.cli_report = None;
        }
    }

    ui.spacing();

    if let Some(report) = &app.cli_report {
        match report.exit_code {
            Some(0) => ui.text_colored(OK, "exit 0"),
            Some(code) => ui.text_colored(ERROR, format!("exit {code}")),
            None => ui.text_disabled("no exit code"),
        }
        ui.same_line();
        ui.text(&report.session_id);
        ui.same_line();
        ui.text_disabled(format!(
            "{} applied - {} rejected - ~{} in / {} out",
            report.applied,
            report.rejected,
            fmt_tokens(report.tokens_in_est),
            fmt_tokens(report.tokens_out_est)
        ));
        if !report.paths.is_empty() {
            ui.text_disabled("files written:");
            for p in &report.paths {
                ui.text(format!("    {p}"));
            }
        }
        ui.text_disabled(format!("transcript: {}", report.transcript.display()));
        ui.spacing();
        ui.separator();
        ui.spacing();
    }

    if app.cli_output.is_empty() {
        ui.text_disabled(
            "No session output yet. \"Run CLI task\" wraps a registered cli agent in a PTY and \
             streams what it prints.",
        );
        return;
    }

    // The buffer is capped (see RoleNApp::append_cli_output), so drawing it
    // whole each frame stays cheap enough.
    ui.child_window("cli-output")
        .size([0.0, 0.0])
        .border(true)
        .build(ui, || {
            ui.text_wrapped(&app.cli_output);
            if running {
                ui.set_scroll_here_y(1.0);
            }
        });
}
