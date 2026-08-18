//! The menu bar, mirroring the TUI's app bar one-for-one
//! (`maestro-tui/src/mission_control.rs:198`) so muscle memory carries over.
//!
//! Menus only *emit* an [`Action`]; nothing here touches the backend. Items
//! that are not implemented yet are rendered disabled with a hover note naming
//! the CLI command that does the same job, rather than being hidden or - worse
//! - shown as working and then doing nothing.

use eframe::egui;
// Only `MenuBar` is re-exported at the egui root; the buttons live in the
// containers module.
use eframe::egui::containers::menu::{MenuButton, SubMenuButton};

/// Everything the menu bar can ask the app to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    // File
    NewProject,
    Doctor,
    Exit,
    // Providers
    AddProvider,
    DetectClis,
    HealthCheck,
    // Tools
    Settings,
    Theme(egui::ThemePreference),
    // Help
    About,
}

/// Draw the bar. Returns the action the user picked, if any.
pub fn show(ui: &mut egui::Ui) -> Option<Action> {
    let mut action = None;
    egui::MenuBar::new().ui(ui, |ui| {
        MenuButton::new("File").ui(ui, |ui| {
            if item(ui, "New Project", "Ctrl+N") {
                action = Some(Action::NewProject);
            }
            ui.separator();
            if item(ui, "Config Doctor", "F9") {
                action = Some(Action::Doctor);
            }
            ui.separator();
            if item(ui, "Exit", "Alt+F4") {
                action = Some(Action::Exit);
            }
        });

        MenuButton::new("Project").ui(ui, |ui| {
            if item(ui, "New Project", "Ctrl+Shift+N") {
                action = Some(Action::NewProject);
            }
            unimplemented_item(
                ui,
                "Interview",
                "Runs an LLM interview behind a 900 s timeout with no progress or cancel yet.\n\
                 For now: maestro project interview --name <project>",
            );
            unimplemented_item(
                ui,
                "Build (PRD/AGENTS/skills/DAG)",
                "Two sequential LLM calls, each up to 900 s.\n\
                 For now: maestro project build --name <project>",
            );
            ui.separator();
            unimplemented_item(
                ui,
                "Run",
                "Batch execution through the orchestrator.\n\
                 For now: maestro batch --spec <project>/tasks.yaml",
            );
            unimplemented_item(ui, "Pause", "Arrives with batch run control.");
        });

        MenuButton::new("Providers").ui(ui, |ui| {
            if item(ui, "Add Provider", "") {
                action = Some(Action::AddProvider);
            }
            if item(ui, "Detect CLIs & Ollama", "") {
                action = Some(Action::DetectClis);
            }
            if item(ui, "Health Check All", "") {
                action = Some(Action::HealthCheck);
            }
        });

        MenuButton::new("Rules").ui(ui, |ui| {
            unimplemented_item(
                ui,
                "New Rule",
                "Rule editing is CLI-only so far.\n\
                 For now: maestro rule add / maestro rule init",
            );
            unimplemented_item(
                ui,
                "Dry-Run",
                "Calls routing::collect, which health-checks every provider serially \
                 (30 s timeout each), so it needs a cancellable job first.\n\
                 For now: maestro rule dry-run --role <role>",
            );
        });

        MenuButton::new("Sessions").ui(ui, |ui| {
            unimplemented_item(
                ui,
                "Quick Chat",
                "Planned with real multi-turn history, which the TUI never had.",
            );
            unimplemented_item(
                ui,
                "Run CLI Task (PTY-wrapped agent)",
                "The adapter streams PTY output live; the GUI should show it as it arrives.\n\
                 For now: maestro cli run --provider <id> --task <text>",
            );
            unimplemented_item(ui, "Pause All", "Arrives with session control.");
        });

        MenuButton::new("Tools").ui(ui, |ui| {
            if item(ui, "Settings", "F10") {
                action = Some(Action::Settings);
            }
            SubMenuButton::new("Theme").ui(ui, |ui| {
                for (label, pref) in [
                    ("Dark", egui::ThemePreference::Dark),
                    ("Light", egui::ThemePreference::Light),
                    ("Follow system", egui::ThemePreference::System),
                ] {
                    if item(ui, label, "") {
                        action = Some(Action::Theme(pref));
                    }
                }
            });
            ui.separator();
            if item(ui, "Config Doctor", "F9") {
                action = Some(Action::Doctor);
            }
        });

        MenuButton::new("Help").ui(ui, |ui| {
            if item(ui, "About", "F1") {
                action = Some(Action::About);
            }
        });
    });
    action
}

/// A working menu entry.
fn item(ui: &mut egui::Ui, label: &str, shortcut: &str) -> bool {
    let button = if shortcut.is_empty() {
        egui::Button::new(label)
    } else {
        egui::Button::new(label).shortcut_text(shortcut)
    };
    let clicked = ui.add(button).clicked();
    if clicked {
        ui.close();
    }
    clicked
}

/// An entry that is deliberately not wired up yet.
fn unimplemented_item(ui: &mut egui::Ui, label: &str, why: &str) {
    ui.add_enabled(false, egui::Button::new(label))
        .on_disabled_hover_text(why);
}
