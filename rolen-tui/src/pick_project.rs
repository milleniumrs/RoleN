//! "Pick Project" dialog: shows the existing projects in a list so the user
//! never has to guess the id slug (the old flow was a bare "Project id:"
//! free-text prompt with no discoverability).

use appcui::prelude::*;

/// Modal list of projects; the response is the selected project id.
#[ModalWindow(events = ButtonEvents, response = String)]
pub struct PickProjectDialog {
    ids: Vec<String>,
    lb_projects: Handle<ListBox>,
    b_select: Handle<Button>,
    b_cancel: Handle<Button>,
}

impl PickProjectDialog {
    /// `projects` is `(id, display name)` pairs; must not be empty.
    pub fn new(title: &str, projects: &[(String, String)]) -> Self {
        let mut w = Self {
            base: ModalWindow::new(title, layout!("a:c,w:60,h:18"), window::Flags::None),
            ids: projects.iter().map(|(id, _)| id.clone()).collect(),
            lb_projects: Handle::None,
            b_select: Handle::None,
            b_cancel: Handle::None,
        };
        w.add(label!("'&Project (id — name):',x:2,y:1,w:40"));
        w.lb_projects = w.add(listbox!(
            "x:2,y:2,w:54,h:10,flags: ScrollBars,em:'no projects'"
        ));
        w.b_select = w.add(button!("'&Select',x:16,y:14,w:12"));
        w.b_cancel = w.add(button!("'&Cancel',x:32,y:14,w:12"));
        let h = w.lb_projects;
        if let Some(lb) = w.control_mut(h) {
            for (id, name) in projects {
                if name.is_empty() || name == id {
                    lb.add(id.as_str());
                } else {
                    lb.add(&format!("{id} — {name}"));
                }
            }
            lb.set_index(0);
        }
        w
    }

    fn selected_id(&self) -> Option<String> {
        let i = self.control(self.lb_projects)?.index();
        (i != usize::MAX && i < self.ids.len()).then(|| self.ids[i].clone())
    }
}

impl ButtonEvents for PickProjectDialog {
    fn on_pressed(&mut self, handle: Handle<Button>) -> EventProcessStatus {
        if handle == self.b_cancel {
            self.exit();
            return EventProcessStatus::Processed;
        }
        if handle == self.b_select {
            if let Some(id) = self.selected_id() {
                self.exit_with(id);
            }
            return EventProcessStatus::Processed;
        }
        EventProcessStatus::Ignored
    }
}
