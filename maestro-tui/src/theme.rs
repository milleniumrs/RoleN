//! Colour themes (FR-10.7): the three AppCUI built-ins plus Maestro's own
//! palettes, config-persisted and switchable live.
//!
//! Custom themes are built by taking a built-in theme and repainting every
//! publicly settable surface from a [`Palette`]. AppCUI keeps the per-control
//! state groups (buttons, tabs, borders, menus — `ControlCharAttributesState`)
//! crate-private, so those are inherited from whichever built-in base fits the
//! palette best; everything that dominates the look (desktop, windows, text,
//! selections, headers, progress, tooltips) is fully repainted.

use appcui::prelude::*;

/// Selectable themes: (config name, human description).
pub const AVAILABLE: &[(&str, &str)] = &[
    ("default", "AppCUI default palette"),
    ("dark-gray", "high-contrast dark grey"),
    ("light", "for light terminals"),
    ("dark", "white on black"),
    ("hacker", "green phosphor on black"),
    ("fancy", "pink background, dark text"),
    ("rainbow", "a different hue per surface"),
    ("ocean", "cyan and deep blue"),
    ("amber", "retro amber CRT"),
];

/// Colours a custom theme is painted from.
struct Palette {
    fg: Color,
    bg: Color,
    dim: Color,
    hot: Color,
    accent: Color,
    win_fg: Color,
    win_bg: Color,
    bar_fg: Color,
    bar_bg: Color,
    sel_fg: Color,
    sel_bg: Color,
    warn: Color,
    err: Color,
    progress: Color,
}

fn attr(fg: Color, bg: Color) -> CharAttribute {
    CharAttribute::with_color(fg, bg)
}

/// Repaint every publicly settable surface of `base` from `p`.
fn paint(base: Themes, p: Palette) -> Theme {
    let mut t = Theme::new(base);

    t.desktop.character = Character::with_attributes(' ', attr(p.dim, p.bg));

    t.text.normal = attr(p.win_fg, p.win_bg);
    t.text.hot_key = attr(p.hot, p.win_bg);
    t.text.inactive = attr(p.dim, p.win_bg);
    t.text.error = attr(p.err, p.win_bg);
    t.text.warning = attr(p.warn, p.win_bg);
    t.text.hovered = attr(p.hot, p.win_bg);
    t.text.focused = attr(p.fg, p.win_bg);
    t.text.highlighted = attr(p.accent, p.win_bg);
    t.text.enphasized_1 = attr(p.accent, p.win_bg);
    t.text.enphasized_2 = attr(p.hot, p.win_bg);
    t.text.enphasized_3 = attr(p.dim, p.win_bg);

    t.symbol.inactive = attr(p.dim, p.win_bg);
    t.symbol.hovered = attr(p.sel_fg, p.sel_bg);
    t.symbol.pressed = attr(p.sel_fg, p.sel_bg);
    t.symbol.checked = attr(p.accent, p.win_bg);
    t.symbol.unchecked = attr(p.dim, p.win_bg);
    t.symbol.unknown = attr(p.warn, p.win_bg);
    t.symbol.arrows = attr(p.accent, p.win_bg);
    t.symbol.close = attr(p.err, p.bar_bg);
    t.symbol.maximized = attr(p.bar_fg, p.bar_bg);
    t.symbol.resize = attr(p.accent, p.win_bg);

    t.tooltip.text = attr(p.sel_fg, p.sel_bg);
    t.tooltip.arrow = attr(p.accent, p.win_bg);

    t.window.normal = attr(p.win_fg, p.win_bg);
    t.window.inactive = attr(p.dim, p.win_bg);
    t.window.error = attr(p.err, p.win_bg);
    t.window.warning = attr(p.warn, p.win_bg);
    t.window.info = attr(p.accent, p.win_bg);
    t.window.bar.focus = attr(p.bar_fg, p.bar_bg);
    t.window.bar.normal = attr(p.dim, p.bar_bg);
    t.window.bar.resizing = attr(p.hot, p.bar_bg);
    t.window.bar.close_button = attr(p.err, p.bar_bg);
    t.window.bar.maximize_button = attr(p.bar_fg, p.bar_bg);
    t.window.bar.tag = attr(p.accent, p.bar_bg);
    t.window.bar.hotkey = attr(p.hot, p.bar_bg);

    t.searchbar.normal = attr(p.dim, p.win_bg);
    t.searchbar.focused = attr(p.fg, p.sel_bg);
    t.searchbar.count = attr(p.accent, p.win_bg);

    t.list_current_item.focus = attr(p.sel_fg, p.sel_bg);
    t.list_current_item.over_inactive = attr(p.dim, p.sel_bg);
    t.list_current_item.over_selection = attr(p.sel_fg, p.sel_bg);
    t.list_current_item.normal = attr(p.win_fg, p.win_bg);
    t.list_current_item.selected = attr(p.accent, p.win_bg);
    t.list_current_item.icon = attr(p.accent, p.win_bg);

    t.markdown.text = attr(p.win_fg, p.win_bg);
    t.markdown.bold = attr(p.accent, p.win_bg);
    t.markdown.italic = attr(p.hot, p.win_bg);
    t.markdown.link = attr(p.accent, p.win_bg);
    t.markdown.code = attr(p.hot, p.win_bg);
    t.markdown.h1 = attr(p.accent, p.win_bg);
    t.markdown.h2 = attr(p.hot, p.win_bg);
    t.markdown.h3 = attr(p.fg, p.win_bg);
    t.markdown.code_block = attr(p.dim, p.win_bg);

    t.progressbar.background = p.dim;
    t.progressbar.progress = p.progress;
    t.progressbar.text = p.win_fg;

    t.hslider.before_line = attr(p.progress, p.win_bg);
    t.hslider.after_line = attr(p.dim, p.win_bg);
    t.hslider.cap = attr(p.accent, p.win_bg);

    t
}

fn dark() -> Theme {
    paint(
        Themes::DarkGray,
        Palette {
            fg: Color::White,
            bg: Color::Black,
            dim: Color::Gray,
            hot: Color::Yellow,
            accent: Color::White,
            win_fg: Color::White,
            win_bg: Color::Black,
            bar_fg: Color::White,
            bar_bg: Color::DarkBlue,
            sel_fg: Color::Black,
            sel_bg: Color::Silver,
            warn: Color::Yellow,
            err: Color::Red,
            progress: Color::Silver,
        },
    )
}

fn hacker() -> Theme {
    paint(
        Themes::DarkGray,
        Palette {
            fg: Color::Green,
            bg: Color::Black,
            dim: Color::DarkGreen,
            hot: Color::Aqua,
            accent: Color::Green,
            win_fg: Color::Green,
            win_bg: Color::Black,
            bar_fg: Color::Black,
            bar_bg: Color::DarkGreen,
            sel_fg: Color::Black,
            sel_bg: Color::Green,
            warn: Color::Yellow,
            err: Color::Red,
            progress: Color::Green,
        },
    )
}

fn fancy() -> Theme {
    paint(
        Themes::Light,
        Palette {
            fg: Color::Black,
            bg: Color::Magenta,
            dim: Color::DarkRed,
            hot: Color::DarkBlue,
            accent: Color::DarkRed,
            win_fg: Color::Black,
            win_bg: Color::Pink,
            bar_fg: Color::White,
            bar_bg: Color::Magenta,
            sel_fg: Color::White,
            sel_bg: Color::Magenta,
            warn: Color::DarkRed,
            err: Color::Red,
            progress: Color::Magenta,
        },
    )
}

fn rainbow() -> Theme {
    // deliberately multi-hued: every surface gets its own colour
    let mut t = paint(
        Themes::DarkGray,
        Palette {
            fg: Color::White,
            bg: Color::DarkBlue,
            dim: Color::Teal,
            hot: Color::Yellow,
            accent: Color::Aqua,
            win_fg: Color::White,
            win_bg: Color::Black,
            bar_fg: Color::Black,
            bar_bg: Color::Magenta,
            sel_fg: Color::Black,
            sel_bg: Color::Yellow,
            warn: Color::Olive,
            err: Color::Red,
            progress: Color::Green,
        },
    );
    t.text.enphasized_1 = attr(Color::Green, Color::Black);
    t.text.enphasized_2 = attr(Color::Pink, Color::Black);
    t.text.enphasized_3 = attr(Color::Aqua, Color::Black);
    t.markdown.h1 = attr(Color::Red, Color::Black);
    t.markdown.h2 = attr(Color::Olive, Color::Black);
    t.markdown.h3 = attr(Color::Green, Color::Black);
    t.markdown.link = attr(Color::Aqua, Color::Black);
    t.markdown.code = attr(Color::Pink, Color::Black);
    t.list_current_item.selected = attr(Color::Green, Color::Black);
    t.list_current_item.icon = attr(Color::Pink, Color::Black);
    t.window.info = attr(Color::Aqua, Color::Black);
    t.tooltip.text = attr(Color::Black, Color::Aqua);
    t
}

fn ocean() -> Theme {
    paint(
        Themes::DarkGray,
        Palette {
            fg: Color::Aqua,
            bg: Color::DarkBlue,
            dim: Color::Teal,
            hot: Color::White,
            accent: Color::Aqua,
            win_fg: Color::Silver,
            win_bg: Color::DarkBlue,
            bar_fg: Color::White,
            bar_bg: Color::Teal,
            sel_fg: Color::Black,
            sel_bg: Color::Aqua,
            warn: Color::Yellow,
            err: Color::Pink,
            progress: Color::Aqua,
        },
    )
}

fn amber() -> Theme {
    paint(
        Themes::DarkGray,
        Palette {
            fg: Color::Yellow,
            bg: Color::Black,
            dim: Color::Olive,
            hot: Color::White,
            accent: Color::Yellow,
            win_fg: Color::Yellow,
            win_bg: Color::Black,
            bar_fg: Color::Black,
            bar_bg: Color::Olive,
            sel_fg: Color::Black,
            sel_bg: Color::Yellow,
            warn: Color::White,
            err: Color::Red,
            progress: Color::Olive,
        },
    )
}

/// Canonical config name for a theme value, accepting aliases (older configs
/// wrote "dark" when only the three built-ins existed).
pub fn canonical(name: &str) -> Option<&'static str> {
    match name.trim().to_ascii_lowercase().replace('_', "-").as_str() {
        "default" | "standard" => Some("default"),
        "dark-gray" | "darkgray" | "gray" | "grey" => Some("dark-gray"),
        "light" | "bright" => Some("light"),
        "dark" | "black" => Some("dark"),
        "hacker" | "matrix" | "terminal" => Some("hacker"),
        "fancy" | "pink" => Some("fancy"),
        "rainbow" | "colorful" | "colourful" => Some("rainbow"),
        "ocean" | "blue" | "sea" => Some("ocean"),
        "amber" | "retro" | "crt" => Some("amber"),
        _ => None,
    }
}

/// Build the theme for a config value; unknown values fall back to "default".
pub fn build(name: &str) -> Theme {
    match canonical(name).unwrap_or("default") {
        "dark-gray" => Theme::new(Themes::DarkGray),
        "light" => Theme::new(Themes::Light),
        "dark" => dark(),
        "hacker" => hacker(),
        "fancy" => fancy(),
        "rainbow" => rainbow(),
        "ocean" => ocean(),
        "amber" => amber(),
        _ => Theme::new(Themes::Default),
    }
}

/// Apply a theme to the running application.
pub fn apply(name: &str) {
    App::set_theme(build(name));
}

/// Persist the theme in config.toml (so it survives restarts).
pub fn persist(name: &str) -> Result<(), maestro_core::CoreError> {
    let mut cfg = maestro_core::config::Config::load().unwrap_or_default();
    cfg.general.theme = canonical(name).unwrap_or("default").to_string();
    cfg.save()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_names_and_aliases() {
        assert_eq!(canonical("default"), Some("default"));
        assert_eq!(canonical(" LIGHT "), Some("light"));
        assert_eq!(canonical("dark"), Some("dark"));
        assert_eq!(canonical("dark_gray"), Some("dark-gray"));
        assert_eq!(canonical("matrix"), Some("hacker"));
        assert_eq!(canonical("pink"), Some("fancy"));
        assert_eq!(canonical("crt"), Some("amber"));
        assert_eq!(canonical("chartreuse"), None);
    }

    #[test]
    fn every_listed_theme_is_resolvable_and_unique() {
        let mut seen = Vec::new();
        for (name, _) in AVAILABLE {
            assert_eq!(canonical(name), Some(*name), "not canonical: {name}");
            let t = build(name);
            // desktop + window body must differ between themes, otherwise the
            // entry is a duplicate that only pretends to be a new theme
            let fingerprint = (
                t.desktop.character,
                t.window.normal,
                t.list_current_item.focus,
            );
            assert!(
                !seen.contains(&fingerprint),
                "theme '{name}' looks identical to an earlier one"
            );
            seen.push(fingerprint);
        }
        assert_eq!(seen.len(), AVAILABLE.len());
    }

    #[test]
    fn unknown_theme_falls_back_to_default() {
        let fallback = build("nope");
        let default = Theme::new(Themes::Default);
        assert_eq!(fallback.desktop.character, default.desktop.character);
    }

    #[test]
    fn custom_palettes_are_actually_applied() {
        let h = build("hacker");
        assert_eq!(
            h.text.normal,
            CharAttribute::with_color(Color::Green, Color::Black)
        );
        assert_eq!(h.progressbar.progress, Color::Green);

        let f = build("fancy");
        assert_eq!(
            f.window.normal,
            CharAttribute::with_color(Color::Black, Color::Pink)
        );
    }
}
