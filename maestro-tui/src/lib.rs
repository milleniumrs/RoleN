//! maestro-tui — AppCUI front-end (PRD FR-10, wireframes in docs/TUI-DESIGN.md).

mod add_provider;
mod mission_control;
mod new_project;
mod provider_detail;
mod quick_chat;
mod settings;
pub mod theme;
mod transcript_view;

/// Launch the TUI. Blocks until the user exits.
pub fn run() -> Result<(), appcui::system::Error> {
    // colour theme comes from config.toml and can be switched live from the
    // Tools > Theme menu or the Settings window
    let theme_name = maestro_core::config::Config::load()
        .map(|c| c.general.theme)
        .unwrap_or_else(|_| "default".to_string());

    let mut app = appcui::system::App::new()
        .app_bar()
        .title("Maestro")
        .theme(theme::build(&theme_name))
        .build()?;
    app.add_window(mission_control::MissionControl::new());
    app.run();
    Ok(())
}
