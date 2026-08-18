//! Making backend text safe to draw.
//!
//! The GUI displays strings produced by the other crates, and those were
//! written for terminals: `maestro-core/src/rules.rs:247` puts U+2192 in every
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

    /// The exact shape `maestro_core::rules::decide` produces. If egui ever
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
