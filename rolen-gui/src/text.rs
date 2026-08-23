//! Making backend text safe to draw.
//!
//! The GUI displays strings produced by the other crates, and those were
//! written for terminals: `rolen-core/src/rules.rs:247` puts U+2192 in every
//! routing explanation, and the orchestrator's batch events use arrows, check
//! marks and a retry symbol. Dear ImGui's built-in font atlas covers the
//! "default" glyph range (0x20..=0xFF: ASCII plus Latin-1), so anything beyond
//! that would paint as an empty tofu box.
//!
//! Rather than rewrite the core strings - the TUI and CLI render them fine in
//! a terminal - the display layer substitutes an ASCII spelling for anything
//! the default font cannot draw.

/// Readable ASCII for the symbols the core crates actually emit.
/// Anything else outside the drawable range becomes `?`, which is at least
/// visibly wrong rather than silently blank.
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

/// Replace every character the default font atlas cannot draw.
///
/// ASCII and Latin-1 (the atlas' default glyph range, 0x20..=0xFF) are kept;
/// everything else is substituted. ASCII-only input is returned unchanged,
/// which is the overwhelmingly common case.
pub fn renderable(text: &str) -> String {
    if text.is_ascii() {
        return text.to_string();
    }
    text.chars()
        .map(|c| {
            if (c as u32) <= 0xFF {
                c.to_string()
            } else {
                ASCII_FALLBACK
                    .iter()
                    .find(|(from, _)| *from == c)
                    .map_or_else(|| "?".to_string(), |(_, to)| (*to).to_string())
            }
        })
        .collect()
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

    #[test]
    fn ascii_is_passed_through_untouched() {
        assert_eq!(renderable("coder -> kimi/k2"), "coder -> kimi/k2");
    }

    /// The exact shape `rolen_core::rules::decide` produces.
    #[test]
    fn a_core_routing_explanation_becomes_drawable() {
        let raw = "role via rule 'coder' \u{2192} kimi/kimi-for-coding";
        let fixed = renderable(raw);
        assert!(!fixed.contains('\u{2192}'));
        assert!(fixed.contains("->"));
    }

    #[test]
    fn latin1_is_kept_and_known_symbols_get_their_spelling() {
        // é and · are inside the atlas' default glyph range (0x20..=0xFF).
        assert_eq!(renderable("caf\u{e9}"), "caf\u{e9}");
        assert_eq!(renderable("a\u{2713}b"), "aokb");
        assert_eq!(renderable("x\u{2026}y"), "x...y");
    }

    /// A symbol with no entry in the table must not vanish silently.
    #[test]
    fn unknown_missing_glyphs_become_a_question_mark() {
        assert_eq!(renderable("a\u{10A00}b"), "a?b");
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
}
