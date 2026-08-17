//! maestro-tui — AppCUI front-end (PRD FR-10, wireframes in docs/TUI-DESIGN.md).

mod add_provider;
mod mission_control;
mod new_project;
mod provider_detail;
mod quick_chat;
mod settings;
mod transcript_view;

/// Launch the TUI. Blocks until the user exits.
pub fn run() -> Result<(), appcui::system::Error> {
    let mut app = appcui::system::App::new()
        .app_bar()
        .title("Maestro")
        .build()?;
    app.add_window(mission_control::MissionControl::new());
    app.run();
    Ok(())
}
