//! Making backend text safe to draw.
//!
//! The GUI displays strings produced by the other crates, and those were
//! written for terminals: `rolen-core/src/rules.rs:247` puts U+2192 in every
//! routing explanation, and the orchestrator's batch events use arrows, check
//! marks and a retry symbol. egui only ships Ubuntu-Light plus emoji fonts, so
//! those characters paint as empty tofu boxes.
//!
//! Rather than rewrite the core strings - the TUI and CLI render them fine in
//! a terminal - the display layer substitutes an ASCII spelling for anything
//! the loaded fonts cannot draw.

use eframe::egui;

/// Readable ASCII for the symbols the core crates actually emit.
/// Anything else that is missing becomes `?`, which is at least visibly wrong
/// rather than silently blank.
const ASCII_FALLBACK: &[(char, &str)] = &[
    ('\u{2192}', "->"),    // rules explanations, scheduler task lines
    ('\u{21c4}', "<->"),   // provider migration
    ('\u{27f3}', "retry"), // retry marker
    ('\u{2713}', "ok"),    // check mark
    ('\u{2717}', "failed"),
    ('\u{2705}', "ok"),
    ('\u{274c}', "failed"),
    ('\u{2026}', "..."),
    ('\u{2014}', "-"),
    ('\u{2022}', "*"),
    ('\u{25b6}', ">"),
    ('\u{26a0}', "!"),
];

/// Replace every character the current fonts cannot draw.
///
/// ASCII-only input is returned unchanged without touching the font atlas,
/// which is the overwhelmingly common case.
pub fn renderable(ctx: &egui::Context, text: &str) -> String {
    if text.is_ascii() {
        return text.to_string();
    }
    // Coverage depends on the family, not the size, so the default body font
    // answers for every text style the GUI uses.
    let font = egui::FontId::default();
    ctx.fonts_mut(|fonts| {
        text.chars()
            .map(|c| {
                if c.is_ascii() || fonts.has_glyph(&font, c) {
                    c.to_string()
                } else {
                    ASCII_FALLBACK
                        .iter()
                        .find(|(from, _)| *from == c)
                        .map_or_else(|| "?".to_string(), |(_, to)| (*to).to_string())
                }
            })
            .collect()
    })
}

/// Remove ANSI/VT escape sequences from PTY output.
///
/// A wrapped CLI agent talks to a terminal, so its stream is full of colour
/// codes, cursor moves and title sets. Painted verbatim in a label they are
/// visible garbage, so they are dropped before display. Carriage returns are
/// normalised too: `\r\n` becomes `\n`, and a bare `\r` (a progress line
/// rewriting itself) is dropped rather than shown.
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => match chars.next() {
                // CSI: parameters, then a final byte in @..~
                Some('[') => {
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                // OSC: runs until BEL or the two-byte string terminator.
                Some(']') => {
                    while let Some(c) = chars.next() {
                        if c == '\u{7}' {
                            break;
                        }
                        if c == '\u{1b}' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                // Two-character sequences such as charset selection.
                Some('(' | ')' | '#') => {
                    chars.next();
                }
                // A lone escape, or anything else: drop the pair.
                _ => {}
            },
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                    out.push('\n');
                }
            }
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> egui::Context {
        let ctx = egui::Context::default();
        // Fonts are built lazily; lay out one frame so the atlas exists.
        let mut out = ctx.run_ui(Default::default(), |ui| {
            ui.label("warm up");
        });
        out.textures_delta.clear();
        ctx
    }

    #[test]
    fn ascii_is_passed_through_untouched() {
        let ctx = ctx();
        assert_eq!(renderable(&ctx, "coder -> kimi/k2"), "coder -> kimi/k2");
    }

    /// The exact shape `rolen_core::rules::decide` produces. If egui ever
    /// ships a font covering the arrow this still passes - the point is that
    /// the result is drawable, not that a substitution happened.
    #[test]
    fn a_core_routing_explanation_becomes_drawable() {
        let ctx = ctx();
        let raw = "role via rule 'coder' \u{2192} kimi/kimi-for-coding";
        let fixed = renderable(&ctx, raw);
        let font = egui::FontId::default();
        assert!(
            ctx.fonts_mut(|f| f.has_glyphs(&font, &fixed)),
            "every glyph must be drawable after substitution, got {fixed:?}"
        );
        assert!(!fixed.contains('\u{2192}'));
    }

    #[test]
    fn colour_codes_are_removed_but_the_text_survives() {
        assert_eq!(
            strip_ansi("\u{1b}[32m[mock-agent] done\u{1b}[0m"),
            "[mock-agent] done"
        );
        assert_eq!(strip_ansi("plain"), "plain");
    }

    #[test]
    fn cursor_moves_and_window_titles_are_removed() {
        assert_eq!(strip_ansi("a\u{1b}[2J\u{1b}[Hb"), "ab");
        // OSC terminated by BEL and by the string terminator.
        assert_eq!(strip_ansi("x\u{1b}]0;title\u{7}y"), "xy");
        assert_eq!(strip_ansi("x\u{1b}]0;title\u{1b}\\y"), "xy");
    }

    /// A progress line rewriting itself should not become a jumble; the
    /// newline form of a line break must still survive.
    #[test]
    fn carriage_returns_are_normalised() {
        assert_eq!(strip_ansi("one\r\ntwo"), "one\ntwo");
        assert_eq!(strip_ansi("50%\r100%"), "50%100%");
    }

    /// A symbol with no entry in the table must not vanish silently.
    #[test]
    fn unknown_missing_glyphs_become_a_question_mark() {
        let ctx = ctx();
        let font = egui::FontId::default();
        // Pick something the bundled fonts certainly lack.
        let exotic = '\u{10A00}';
        if !ctx.fonts_mut(|f| f.has_glyph(&font, exotic)) {
            let out = renderable(&ctx, &format!("a{exotic}b"));
            assert_eq!(out, "a?b");
        }
    }
}
