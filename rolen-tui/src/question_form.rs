//! Interview question form (PRD FR-6.2): one question per dialog, with radio
//! buttons when the question offers options and a text field otherwise.
//! "Answer later" defers the question (it stays pending in the project).

use appcui::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuestionAnswer {
    Answered(String),
    Deferred,
}

#[ModalWindow(events = ButtonEvents, response = QuestionAnswer)]
pub struct QuestionForm {
    radios: Vec<Handle<RadioBox>>,
    options: Vec<String>,
    t_answer: Handle<TextField>,
    l_status: Handle<Label>,
    b_ok: Handle<Button>,
    b_defer: Handle<Button>,
}

impl QuestionForm {
    pub fn new(index: usize, total: usize, question: &str, options: &[String]) -> Self {
        let qlines = wrap(question, 76);
        let opt_rows = if options.is_empty() {
            2 // text field
        } else {
            options.len() + 1 // options + "answer later"
        };
        let height = (qlines.len() + opt_rows + 6) as i32;
        let mut w = Self {
            base: ModalWindow::new(
                &format!("Question {index}/{total}"),
                Layout::aligned(Alignment::Center, 84, height as u32),
                window::Flags::None,
            ),
            radios: Vec::new(),
            options: options.to_vec(),
            t_answer: Handle::None,
            l_status: Handle::None,
            b_ok: Handle::None,
            b_defer: Handle::None,
        };

        let mut y = 1i32;
        for line in &qlines {
            w.add(Label::new(line, Layout::absolute(2, y, 78, 1)));
            y += 1;
        }
        y += 1;

        if options.is_empty() {
            w.t_answer = w.add(TextField::new(
                "",
                Layout::absolute(2, y, 78, 1),
                textfield::Flags::None,
            ));
        } else {
            for (i, opt) in options.iter().enumerate() {
                let cap = if opt.chars().count() > 74 {
                    format!("{}…", opt.chars().take(73).collect::<String>())
                } else {
                    opt.clone()
                };
                let h = w.add(RadioBox::new(&cap, Layout::absolute(3, y, 78, 1), i == 0));
                w.radios.push(h);
                y += 1;
            }
            let h = w.add(RadioBox::new(
                "answer later (keep pending)",
                Layout::absolute(3, y, 78, 1),
                false,
            ));
            w.radios.push(h);
        }

        w.l_status = w.add(Label::new("", Layout::absolute(2, height - 3, 78, 1)));
        w.b_ok = w.add(Button::new("&OK", Layout::absolute(28, height - 2, 12, 1)));
        w.b_defer = w.add(Button::new(
            "&Defer",
            Layout::absolute(44, height - 2, 12, 1),
        ));
        w
    }
}

impl ButtonEvents for QuestionForm {
    fn on_pressed(&mut self, handle: Handle<Button>) -> EventProcessStatus {
        if handle == self.b_defer {
            self.exit_with(QuestionAnswer::Deferred);
            return EventProcessStatus::Processed;
        }
        if handle == self.b_ok {
            if self.radios.is_empty() {
                let text = self
                    .control(self.t_answer)
                    .map(|t: &TextField| t.text().trim().to_string())
                    .unwrap_or_default();
                if text.is_empty() {
                    let h = self.l_status;
                    if let Some(l) = self.control_mut(h) {
                        l.set_caption("type an answer, or press Defer");
                    }
                    return EventProcessStatus::Processed;
                }
                self.exit_with(QuestionAnswer::Answered(text));
                return EventProcessStatus::Processed;
            }
            for (i, rh) in self.radios.iter().enumerate() {
                if self
                    .control(*rh)
                    .map(|r: &RadioBox| r.is_selected())
                    .unwrap_or(false)
                {
                    if i < self.options.len() {
                        // radios hold truncated captions; answer with the full text
                        self.exit_with(QuestionAnswer::Answered(self.options[i].clone()));
                    } else {
                        self.exit_with(QuestionAnswer::Deferred);
                    }
                    return EventProcessStatus::Processed;
                }
            }
            return EventProcessStatus::Processed;
        }
        EventProcessStatus::Ignored
    }
}

fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        if !cur.is_empty() && cur.len() + 1 + word.len() > width {
            lines.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}
