//! rolen-tui — AppCUI front-end (PRD FR-10, wireframes in docs/TUI-DESIGN.md).

mod add_provider;
mod mission_control;
mod model_prices;
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
    let theme_name = rolen_core::config::Config::load()
        .map(|c| c.general.theme)
        .unwrap_or_else(|_| "default".to_string());

    let mut app = appcui::system::App::new()
        .app_bar()
        .title("RoleN")
        .theme(theme::build(&theme_name))
        .build()?;
    app.add_window(mission_control::MissionControl::new());
    app.run();
    Ok(())
}

/// Render the real Mission Control window offscreen with `theme_name` and
/// print an ANSI dump of the surface.
///
/// This exists so palettes can be verified objectively (which cell has which
/// background) without a terminal — see `examples/theme_dump.rs`.
pub fn debug_render(
    theme_name: &str,
    width: u16,
    height: u16,
) -> Result<(), appcui::system::Error> {
    let script = "
        Paint('rolen theme dump')
    ";
    let mut app = appcui::system::App::debug(width, height, script)
        .app_bar()
        .theme(theme::build(theme_name))
        .build()?;
    app.add_window(mission_control::MissionControl::new());
    app.run();
    Ok(())
}
