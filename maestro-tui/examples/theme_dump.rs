//! Offscreen theme renderer: prints an ANSI dump of Mission Control painted
//! with a given theme, so palettes can be inspected without a terminal.
//!
//! Usage: cargo run -p maestro-tui --example theme_dump -- fancy

fn main() -> Result<(), appcui::system::Error> {
    let theme = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "default".to_string());
    let width: u16 = std::env::args()
        .nth(2)
        .and_then(|v| v.parse().ok())
        .unwrap_or(100);
    let height: u16 = std::env::args()
        .nth(3)
        .and_then(|v| v.parse().ok())
        .unwrap_or(26);
    eprintln!("rendering theme '{theme}' at {width}x{height}");
    maestro_tui::debug_render(&theme, width, height)
}
