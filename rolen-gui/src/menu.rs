//! The menu bar, mirroring the TUI's app bar one-for-one
//! (`rolen-tui/src/mission_control.rs:198`) so muscle memory carries over.
//!
//! Menus only *emit* an [`Action`]; nothing here touches the backend. Items
//! that are not implemented yet are rendered disabled with a tooltip naming
//! the CLI command that does the same job, rather than being hidden or - worse
//! - shown as working and then doing nothing.

use dear_imgui_rs::{ItemHoveredFlags, ThemePreset, Ui};

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
    // Rules
    DryRun,
    // Sessions
    QuickChat,
    RunCliTask,
    // Tools
    Settings,
    Theme(ThemePreset),
    // Help
    About,
}

/// Draw the bar (call inside a window with `WindowFlags::MENU_BAR`).
/// Returns the action the user picked, if any.
pub fn show(ui: &Ui) -> Option<Action> {
    let mut action = None;
    let _bar = ui.begin_menu_bar()?;

    ui.menu("File", || {
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

    ui.menu("Project", || {
        if item(ui, "New Project", "Ctrl+Shift+N") {
            action = Some(Action::NewProject);
        }
        unimplemented_item(
            ui,
            "Interview",
            "Runs an LLM interview behind a 900 s timeout with no progress or cancel yet.\n\
             For now: rolen project interview --name <project>",
        );
        unimplemented_item(
            ui,
            "Build (PRD/AGENTS/skills/DAG)",
            "Two sequential LLM calls, each up to 900 s.\n\
             For now: rolen project build --name <project>",
        );
        ui.separator();
        unimplemented_item(
            ui,
            "Run",
            "Batch execution through the orchestrator.\n\
             For now: rolen batch --spec <project>/tasks.yaml",
        );
        unimplemented_item(ui, "Pause", "Arrives with batch run control.");
    });

    ui.menu("Providers", || {
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

    ui.menu("Rules", || {
        unimplemented_item(
            ui,
            "New Rule",
            "Rule editing is CLI-only so far.\n\
             For now: rolen rule add / rolen rule init",
        );
        if item(ui, "Dry-Run", "Ctrl+D") {
            action = Some(Action::DryRun);
        }
    });

    ui.menu("Sessions", || {
        if item(ui, "Quick Chat", "Ctrl+Q") {
            action = Some(Action::QuickChat);
        }
        if item(ui, "Run CLI Task (PTY-wrapped agent)", "") {
            action = Some(Action::RunCliTask);
        }
        unimplemented_item(ui, "Pause All", "Arrives with session control.");
    });

    ui.menu("Tools", || {
        if item(ui, "Settings", "F10") {
            action = Some(Action::Settings);
        }
        ui.menu("Theme", || {
            for (label, preset) in [
                ("Dark", ThemePreset::Dark),
                ("Light", ThemePreset::Light),
                ("Classic", ThemePreset::Classic),
            ] {
                if item(ui, label, "") {
                    action = Some(Action::Theme(preset));
                }
            }
        });
        ui.separator();
        if item(ui, "Config Doctor", "F9") {
            action = Some(Action::Doctor);
        }
    });

    ui.menu("Help", || {
        if item(ui, "About", "F1") {
            action = Some(Action::About);
        }
    });

    action
}

/// A working menu entry.
fn item(ui: &Ui, label: &str, shortcut: &str) -> bool {
    if shortcut.is_empty() {
        ui.menu_item(label)
    } else {
        ui.menu_item_with_shortcut(label, shortcut)
    }
}

/// An entry that is deliberately not wired up yet.
///
/// Disabled items are not hoverable by default, so the explanation needs the
/// allow-when-disabled flag to show at all.
fn unimplemented_item(ui: &Ui, label: &str, why: &str) {
    ui.menu_item_enabled_selected_no_shortcut(label, false, false);
    if ui.is_item_hovered_with_flags(ItemHoveredFlags::ALLOW_WHEN_DISABLED) {
        ui.tooltip_text(why);
    }
}
