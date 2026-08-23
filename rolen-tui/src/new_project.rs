//! "New Project" dialog (PRD FR-5.1, TUI-DESIGN.md §7.2): name, description,
//! stack — then scaffolds the project directory.

use appcui::prelude::*;

/// Result of the New Project dialog.
pub struct ProjectDraft {
    pub name: String,
    pub description: String,
    pub stack: String,
}

#[ModalWindow(events = ButtonEvents, response = ProjectDraft)]
pub struct NewProjectDialog {
    t_name: Handle<TextField>,
    t_description: Handle<TextField>,
    t_stack: Handle<TextField>,
    l_status: Handle<Label>,
    b_create: Handle<Button>,
    b_cancel: Handle<Button>,
}

impl NewProjectDialog {
    pub fn new() -> Self {
        let mut w = Self {
            base: ModalWindow::new("New Project", layout!("a:c,w:66,h:16"), window::Flags::None),
            t_name: Handle::None,
            t_description: Handle::None,
            t_stack: Handle::None,
            l_status: Handle::None,
            b_create: Handle::None,
            b_cancel: Handle::None,
        };
        w.add(label!("'&Name:',x:2,y:1,w:20"));
        w.t_name = w.add(textfield!("x:2,y:2,w:60"));
        w.add(label!(
            "'&Description (one or two sentences):',x:2,y:4,w:50"
        ));
        w.t_description = w.add(textfield!("x:2,y:5,w:60"));
        w.add(label!(
            "'&Stack (comma-separated, e.g. rust,appcui):',x:2,y:7,w:50"
        ));
        w.t_stack = w.add(textfield!("x:2,y:8,w:60"));
        w.l_status = w.add(label!("'',x:2,y:10,w:60"));
        w.b_create = w.add(button!("'&Create',x:20,y:12,w:12"));
        w.b_cancel = w.add(button!("'C&ancel',x:36,y:12,w:12"));
        w
    }

    fn field(&self, h: Handle<TextField>) -> String {
        self.control(h)
            .map(|t| t.text().trim().to_string())
            .unwrap_or_default()
    }

    fn set_status(&mut self, msg: &str) {
        let h = self.l_status;
        if let Some(l) = self.control_mut(h) {
            l.set_caption(msg);
        }
    }
}

impl ButtonEvents for NewProjectDialog {
    fn on_pressed(&mut self, handle: Handle<Button>) -> EventProcessStatus {
        if handle == self.b_cancel {
            self.exit();
            return EventProcessStatus::Processed;
        }
        if handle == self.b_create {
            let name = self.field(self.t_name);
            if name.is_empty() {
                self.set_status("name is required");
                return EventProcessStatus::Processed;
            }
            let description = self.field(self.t_description);
            let stack = self.field(self.t_stack);
            self.exit_with(ProjectDraft {
                name,
                description,
                stack,
            });
            return EventProcessStatus::Processed;
        }
        EventProcessStatus::Ignored
    }
}
