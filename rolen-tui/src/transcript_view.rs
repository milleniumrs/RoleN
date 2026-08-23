//! Transcript viewer (FR-9.3): shows a session transcript/log with ANSI
//! escape sequences stripped for readability.

use appcui::prelude::*;

#[ModalWindow(events = ButtonEvents)]
pub struct TranscriptView {
    t_log: Handle<TextArea>,
    b_close: Handle<Button>,
}

impl TranscriptView {
    pub fn new(title: &str, content: &str) -> Self {
        let mut w = Self {
            base: ModalWindow::new(title, layout!("a:c,w:100,h:28"), window::Flags::Sizeable),
            t_log: Handle::None,
            b_close: Handle::None,
        };
        w.t_log = w.add(textarea!(
            "'',l:1,t:0,r:1,b:2,flags: [ReadOnly, ScrollBars]"
        ));
        w.b_close = w.add(button!("'&Close',l:45,b:0,w:12"));
        let cleaned = strip_ansi(content);
        let h = w.t_log;
        if let Some(t) = w.control_mut(h) {
            t.set_text(&cleaned);
        }
        w
    }
}

impl ButtonEvents for TranscriptView {
    fn on_pressed(&mut self, handle: Handle<Button>) -> EventProcessStatus {
        if handle == self.b_close {
            self.exit();
            return EventProcessStatus::Processed;
        }
        EventProcessStatus::Ignored
    }
}

/// Strip CSI (`ESC [ ... letter`) and OSC (`ESC ] ... BEL/ESC\`) sequences.
pub fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            match chars.next() {
                Some('[') => {
                    // CSI: ends with a letter in @..~
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    // OSC: ends with BEL or ESC\
                    let mut prev = '\0';
                    for c in chars.by_ref() {
                        if c == '\u{7}' || (prev == '\u{1b}' && c == '\\') {
                            break;
                        }
                        prev = c;
                    }
                }
                _ => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_csi_and_osc() {
        let raw = "\u{1b}[2J\u{1b}[Hhello \u{1b}[31mred\u{1b}[0m \u{1b}]0;title\u{7}done";
        assert_eq!(strip_ansi(raw), "hello red done");
    }
}
