//! Project detail window (PRD FR-10.4): PRD.md / AGENTS.md / skills /
//! clarifications tabs for the selected project. Also the text preview +
//! apply dialog used by the Build flow (FR-5.2 review, FR-5.3 diff preview).

use appcui::prelude::*;
use rolen_core::project::ProjectMeta;
use std::path::Path;

#[ModalWindow(events = ButtonEvents)]
pub struct ProjectView {
    ta_prd: Handle<TextArea>,
    ta_agents: Handle<TextArea>,
    ta_skills: Handle<TextArea>,
    ta_clar: Handle<TextArea>,
    b_close: Handle<Button>,
}

fn read_or(dir: &Path, file: &str, hint: &str) -> String {
    match std::fs::read_to_string(dir.join(file)) {
        Ok(text) => text,
        Err(_) => format!("(no {file} yet — {hint})"),
    }
}

fn skills_text(meta: &ProjectMeta) -> String {
    if meta.skills.is_empty() {
        "(no skills installed yet)\n\n\
         Project → Build suggests skills from the PRD; install one with:\n  \
         rolen project skills --name <project> --install <skill>"
            .to_string()
    } else {
        let mut s = String::from("installed skills:\n\n");
        for sk in &meta.skills {
            s.push_str(&format!("  • {sk}\n"));
        }
        s
    }
}

fn clarifications_text(meta: &ProjectMeta) -> String {
    let mut out = String::new();
    for c in &meta.clarifications {
        out.push_str(&format!(
            "[{}] {}\n",
            format!("{:?}", c.status).to_lowercase(),
            c.question
        ));
        if let Some(a) = &c.answer {
            out.push_str(&format!("    → {a}\n"));
        }
        if let Some(task) = &c.task_id {
            out.push_str(&format!("    (raised mid-run by task {task})\n"));
        }
    }
    if out.is_empty() {
        out = "(no clarifications yet — Project → Interview)".into();
    }
    out
}

impl ProjectView {
    pub fn new(dir: &Path, meta: &ProjectMeta) -> Self {
        let mut w = Self {
            base: ModalWindow::new(
                &format!("Project: {} ({})", meta.name, meta.id),
                layout!("a:c,w:110,h:30"),
                window::Flags::Sizeable,
            ),
            ta_prd: Handle::None,
            ta_agents: Handle::None,
            ta_skills: Handle::None,
            ta_clar: Handle::None,
            b_close: Handle::None,
        };
        let mut t = tab!(
            "tabs:[PRD.md,AGENTS.md,Skills,Clarifications],l:0,t:0,r:0,b:2,flags: TransparentBackground"
        );
        w.ta_prd = t.add(
            0,
            textarea!("'',l:0,t:0,r:0,b:0,flags: [ReadOnly, ScrollBars]"),
        );
        w.ta_agents = t.add(
            1,
            textarea!("'',l:0,t:0,r:0,b:0,flags: [ReadOnly, ScrollBars]"),
        );
        w.ta_skills = t.add(
            2,
            textarea!("'',l:0,t:0,r:0,b:0,flags: [ReadOnly, ScrollBars]"),
        );
        w.ta_clar = t.add(
            3,
            textarea!("'',l:0,t:0,r:0,b:0,flags: [ReadOnly, ScrollBars]"),
        );
        w.add(t);
        w.b_close = w.add(button!("'&Close',l:49,b:0,w:12"));

        let prd = read_or(dir, "PRD.md", "Project → Build generates it");
        let agents = read_or(dir, "AGENTS.md", "Project → Build generates it");
        let skills = skills_text(meta);
        let clar = clarifications_text(meta);
        for (handle, text) in [
            (w.ta_prd, prd),
            (w.ta_agents, agents),
            (w.ta_skills, skills),
            (w.ta_clar, clar),
        ] {
            if let Some(ta) = w.control_mut(handle) {
                ta.set_text(&text);
            }
        }
        w
    }
}

impl ButtonEvents for ProjectView {
    fn on_pressed(&mut self, handle: Handle<Button>) -> EventProcessStatus {
        if handle == self.b_close {
            self.exit();
            return EventProcessStatus::Processed;
        }
        EventProcessStatus::Ignored
    }
}

/// FR-5.2/5.3: show generated content (or a diff against the existing file)
/// and let the user apply or discard it. Response: true = apply.
#[ModalWindow(events = ButtonEvents, response = bool)]
pub struct PreviewApply {
    ta: Handle<TextArea>,
    b_apply: Handle<Button>,
    b_discard: Handle<Button>,
}

impl PreviewApply {
    pub fn new(title: &str, content: &str) -> Self {
        let mut w = Self {
            base: ModalWindow::new(title, layout!("a:c,w:110,h:30"), window::Flags::Sizeable),
            ta: Handle::None,
            b_apply: Handle::None,
            b_discard: Handle::None,
        };
        w.ta = w.add(textarea!(
            "'',l:1,t:0,r:1,b:2,flags: [ReadOnly, ScrollBars]"
        ));
        w.b_apply = w.add(button!("'&Apply',l:38,b:0,w:14"));
        w.b_discard = w.add(button!("'&Discard',l:56,b:0,w:14"));
        let h = w.ta;
        if let Some(ta) = w.control_mut(h) {
            ta.set_text(content);
        }
        w
    }
}

impl ButtonEvents for PreviewApply {
    fn on_pressed(&mut self, handle: Handle<Button>) -> EventProcessStatus {
        if handle == self.b_apply {
            self.exit_with(true);
            return EventProcessStatus::Processed;
        }
        if handle == self.b_discard {
            self.exit_with(false);
            return EventProcessStatus::Processed;
        }
        EventProcessStatus::Ignored
    }
}
