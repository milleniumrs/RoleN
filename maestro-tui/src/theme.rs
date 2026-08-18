//! Colour themes (FR-10.7): config-persisted, switchable live from the menu
//! or the Settings window.

use appcui::prelude::*;

/// Selectable themes: (config name, human description).
pub const AVAILABLE: &[(&str, &str)] = &[
    ("default", "balanced dark palette"),
    ("dark-gray", "high-contrast dark"),
    ("light", "for light terminals"),
];

/// Map a config value to an AppCUI theme. Accepts a few aliases so older
/// configs (which wrote "dark") keep working.
pub fn parse(name: &str) -> Option<Themes> {
    match name.trim().to_ascii_lowercase().replace('_', "-").as_str() {
        "default" | "standard" => Some(Themes::Default),
        "dark" | "dark-gray" | "darkgray" | "gray" | "grey" => Some(Themes::DarkGray),
        "light" | "bright" => Some(Themes::Light),
        _ => None,
    }
}

/// Canonical config name for a theme.
pub fn to_name(theme: Themes) -> &'static str {
    match theme {
        Themes::Default => "default",
        Themes::DarkGray => "dark-gray",
        Themes::Light => "light",
    }
}

/// Theme for a config value, falling back to the default palette.
pub fn resolve(name: &str) -> Themes {
    parse(name).unwrap_or(Themes::Default)
}

/// Apply a theme to the running application.
pub fn apply(name: &str) {
    App::set_theme(Theme::new(resolve(name)));
}

/// Persist the theme in config.toml (so it survives restarts).
pub fn persist(name: &str) -> Result<(), maestro_core::CoreError> {
    let mut cfg = maestro_core::config::Config::load().unwrap_or_default();
    cfg.general.theme = name.to_string();
    cfg.save()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_names_and_aliases() {
        assert!(matches!(parse("default"), Some(Themes::Default)));
        assert!(matches!(parse(" LIGHT "), Some(Themes::Light)));
        assert!(matches!(parse("dark"), Some(Themes::DarkGray)));
        assert!(matches!(parse("dark_gray"), Some(Themes::DarkGray)));
        assert!(matches!(parse("grey"), Some(Themes::DarkGray)));
        assert!(parse("chartreuse").is_none());
    }

    #[test]
    fn names_round_trip() {
        for (name, _) in AVAILABLE {
            let theme = parse(name).unwrap_or_else(|| panic!("unknown theme in AVAILABLE: {name}"));
            assert_eq!(to_name(theme), *name);
        }
    }

    #[test]
    fn unknown_theme_falls_back_to_default() {
        assert!(matches!(resolve("nope"), Themes::Default));
    }
}
