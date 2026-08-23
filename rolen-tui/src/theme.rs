//! Colour themes (FR-10.7): the three AppCUI built-ins plus RoleN's own
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
    ("paper", "white paper, dark ink"),
    ("sky", "pale cyan, navy ink"),
    ("mint", "pale green, forest ink"),
    ("sand", "warm sand, brown ink"),
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

/// Foreground-only attribute: the background stays transparent so the text
/// inherits whatever surface it is drawn on. AppCUI's own themes do this for
/// every text state; painting an opaque background instead makes each label
/// carry its own colour patch over the container.
fn ink(fg: Color) -> CharAttribute {
    CharAttribute::with_fore_color(fg)
}

/// Repaint every publicly settable surface of `base` from `p`.
///
/// Only genuine *surfaces* get an explicit background (desktop, window bodies
/// and title bars, selections, tooltips, progress); text-like states are
/// foreground-only so they blend into the surface beneath them.
fn paint(base: Themes, p: Palette) -> Theme {
    let mut t = Theme::new(base);

    t.desktop.character = Character::with_attributes(' ', attr(p.dim, p.bg));

    t.text.normal = ink(p.win_fg);
    t.text.hot_key = ink(p.hot);
    t.text.inactive = ink(p.dim);
    t.text.error = ink(p.err);
    t.text.warning = ink(p.warn);
    t.text.hovered = ink(p.hot);
    t.text.focused = ink(p.fg);
    t.text.highlighted = ink(p.accent);
    t.text.enphasized_1 = ink(p.accent);
    t.text.enphasized_2 = ink(p.hot);
    t.text.enphasized_3 = ink(p.dim);

    t.symbol.inactive = ink(p.dim);
    t.symbol.hovered = attr(p.sel_fg, p.sel_bg);
    t.symbol.pressed = attr(p.sel_fg, p.sel_bg);
    t.symbol.checked = ink(p.accent);
    t.symbol.unchecked = ink(p.dim);
    t.symbol.unknown = ink(p.warn);
    t.symbol.arrows = ink(p.accent);
    t.symbol.close = ink(p.err);
    t.symbol.maximized = ink(p.bar_fg);
    t.symbol.resize = ink(p.accent);

    t.tooltip.text = attr(p.sel_fg, p.sel_bg);
    t.tooltip.arrow = ink(p.accent);

    // the window body is the surface every control inherits
    t.window.normal = attr(p.win_fg, p.win_bg);
    t.window.inactive = attr(p.dim, p.win_bg);
    t.window.error = attr(p.err, p.win_bg);
    t.window.warning = attr(p.warn, p.win_bg);
    t.window.info = attr(p.accent, p.win_bg);
    t.window.bar.focus = attr(p.bar_fg, p.bar_bg);
    t.window.bar.normal = attr(p.dim, p.bar_bg);
    t.window.bar.resizing = attr(p.hot, p.bar_bg);
    t.window.bar.close_button = ink(p.err);
    t.window.bar.maximize_button = ink(p.bar_fg);
    t.window.bar.tag = ink(p.accent);
    t.window.bar.hotkey = attr(p.hot, p.bar_bg);

    t.searchbar.normal = ink(p.dim);
    t.searchbar.focused = attr(p.sel_fg, p.sel_bg);
    t.searchbar.count = ink(p.accent);

    // selection is a real surface; unselected rows inherit the container
    t.list_current_item.focus = attr(p.sel_fg, p.sel_bg);
    t.list_current_item.over_inactive = attr(p.dim, p.sel_bg);
    t.list_current_item.over_selection = attr(p.sel_fg, p.sel_bg);
    t.list_current_item.normal = ink(p.win_fg);
    t.list_current_item.selected = ink(p.accent);
    t.list_current_item.icon = ink(p.accent);

    t.markdown.text = ink(p.win_fg);
    t.markdown.bold = ink(p.accent);
    t.markdown.italic = ink(p.hot);
    t.markdown.link = ink(p.accent);
    t.markdown.code = ink(p.hot);
    t.markdown.h1 = ink(p.accent);
    t.markdown.h2 = ink(p.hot);
    t.markdown.h3 = ink(p.fg);
    t.markdown.code_block = ink(p.dim);

    t.progressbar.background = p.dim;
    t.progressbar.progress = p.progress;
    t.progressbar.text = p.win_fg;

    t.hslider.before_line = ink(p.progress);
    t.hslider.after_line = ink(p.dim);
    t.hslider.cap = ink(p.accent);

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
    t.text.enphasized_1 = ink(Color::Green);
    t.text.enphasized_2 = ink(Color::Pink);
    t.text.enphasized_3 = ink(Color::Aqua);
    t.markdown.h1 = ink(Color::Red);
    t.markdown.h2 = ink(Color::Olive);
    t.markdown.h3 = ink(Color::Green);
    t.markdown.link = ink(Color::Aqua);
    t.markdown.code = ink(Color::Pink);
    t.list_current_item.selected = ink(Color::Green);
    t.list_current_item.icon = ink(Color::Pink);
    t.window.info = ink(Color::Aqua);
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

// ---- light palettes: dark ink on a light surface ----

fn paper() -> Theme {
    paint(
        Themes::Light,
        Palette {
            fg: Color::Black,
            bg: Color::Silver,
            dim: Color::Gray,
            hot: Color::DarkRed,
            accent: Color::DarkBlue,
            win_fg: Color::Black,
            win_bg: Color::White,
            bar_fg: Color::Black,
            bar_bg: Color::Silver,
            sel_fg: Color::White,
            sel_bg: Color::DarkBlue,
            warn: Color::DarkRed,
            err: Color::Red,
            progress: Color::DarkBlue,
        },
    )
}

fn sky() -> Theme {
    paint(
        Themes::Light,
        Palette {
            fg: Color::DarkBlue,
            bg: Color::Teal,
            dim: Color::Blue,
            hot: Color::DarkRed,
            accent: Color::DarkBlue,
            win_fg: Color::Black,
            win_bg: Color::Aqua,
            bar_fg: Color::White,
            bar_bg: Color::DarkBlue,
            sel_fg: Color::White,
            sel_bg: Color::DarkBlue,
            warn: Color::DarkRed,
            err: Color::Red,
            progress: Color::DarkBlue,
        },
    )
}

fn mint() -> Theme {
    paint(
        Themes::Light,
        Palette {
            fg: Color::DarkGreen,
            bg: Color::DarkGreen,
            dim: Color::Teal,
            hot: Color::DarkRed,
            accent: Color::DarkGreen,
            win_fg: Color::Black,
            win_bg: Color::Green,
            bar_fg: Color::White,
            bar_bg: Color::DarkGreen,
            sel_fg: Color::White,
            sel_bg: Color::DarkGreen,
            warn: Color::DarkRed,
            err: Color::Red,
            progress: Color::DarkGreen,
        },
    )
}

fn sand() -> Theme {
    paint(
        Themes::Light,
        Palette {
            fg: Color::DarkRed,
            bg: Color::Olive,
            dim: Color::Olive,
            hot: Color::DarkBlue,
            accent: Color::DarkRed,
            win_fg: Color::Black,
            win_bg: Color::Yellow,
            bar_fg: Color::Black,
            bar_bg: Color::Olive,
            sel_fg: Color::Yellow,
            sel_bg: Color::DarkRed,
            warn: Color::DarkRed,
            err: Color::Red,
            progress: Color::DarkRed,
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
        "paper" | "white" | "ink" => Some("paper"),
        "sky" | "cyan" | "azure" => Some("sky"),
        "mint" | "forest" => Some("mint"),
        "sand" | "desert" | "warm" => Some("sand"),
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
        "paper" => paper(),
        "sky" => sky(),
        "mint" => mint(),
        "sand" => sand(),
        _ => Theme::new(Themes::Default),
    }
}

/// Apply a theme to the running application.
pub fn apply(name: &str) {
    App::set_theme(build(name));
}

/// Persist the theme in config.toml (so it survives restarts).
pub fn persist(name: &str) -> Result<(), rolen_core::CoreError> {
    let mut cfg = rolen_core::config::Config::load().unwrap_or_default();
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

    /// Regression guard: text states must never paint their own background,
    /// otherwise every label shows as a coloured patch over the surface it
    /// sits on (the window body colour is what defines a theme).
    #[test]
    fn text_states_keep_a_transparent_background() {
        for (name, _) in AVAILABLE {
            let t = build(name);
            let transparent = |a: CharAttribute| a.background == Color::Transparent;
            assert!(
                transparent(t.text.normal),
                "{name}: text.normal paints a background"
            );
            assert!(
                transparent(t.text.hot_key),
                "{name}: text.hot_key paints a background"
            );
            assert!(
                transparent(t.text.inactive),
                "{name}: text.inactive paints a background"
            );
            assert!(
                transparent(t.markdown.text),
                "{name}: markdown.text paints a background"
            );
            assert!(
                transparent(t.list_current_item.normal),
                "{name}: unselected list rows paint a background"
            );
        }
    }

    /// Light themes must genuinely paint a light window surface — verified
    /// end-to-end by docs/theme_report.py, pinned here so a palette edit
    /// cannot silently turn a light theme dark.
    #[test]
    fn light_themes_have_light_surfaces() {
        let light_bg = [
            Color::White,
            Color::Silver,
            Color::Aqua,
            Color::Green,
            Color::Yellow,
            Color::Pink,
        ];
        for name in ["light", "fancy", "paper", "sky", "mint", "sand"] {
            let bg = build(name).window.normal.background;
            assert!(
                light_bg.contains(&bg),
                "theme '{name}' claims to be light but paints {bg:?}"
            );
        }
        for name in ["dark", "hacker", "amber", "ocean", "rainbow"] {
            let bg = build(name).window.normal.background;
            assert!(
                !light_bg.contains(&bg),
                "theme '{name}' claims to be dark but paints {bg:?}"
            );
        }
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
        assert_eq!(h.text.normal, CharAttribute::with_fore_color(Color::Green));
        assert_eq!(h.progressbar.progress, Color::Green);
        assert_eq!(
            h.window.normal,
            CharAttribute::with_color(Color::Green, Color::Black)
        );

        let f = build("fancy");
        assert_eq!(
            f.window.normal,
            CharAttribute::with_color(Color::Black, Color::Pink)
        );
    }
}
