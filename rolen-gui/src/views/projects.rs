//! Projects: the workspace scan as a table, with a detail pane.
//!
//! Creation is wired up; the interview and PRD/DAG build are not, because they
//! are multi-minute LLM calls with no progress or cancellation support in the
//! core yet. The detail pane says so per project rather than offering a button
//! that would freeze on a spinner for ten minutes.

use dear_imgui_rs::{SelectableFlags, TableFlags, Ui};

use crate::app::RoleNApp;
use crate::jobs;

pub fn show(app: &mut RoleNApp, ui: &Ui) {
    let creating = app.jobs.is_running(jobs::NEW_PROJECT);
    ui.with_disabled_if(creating, || {
        if ui.button("New project") {
            app.new_project.open();
        }
    });
    ui.same_line();
    if ui.button("Refresh") {
        app.poller.refresh_now();
    }
    if creating {
        ui.same_line();
        ui.text_disabled("working...");
    }
    if let Some(root) = &app.snap.workspace_root {
        ui.same_line();
        let text = root.display().to_string();
        let w = ui.calc_text_size(&text)[0];
        let x =
            (ui.content_region_avail_width() + ui.cursor_pos_x() - w - 8.0).max(ui.cursor_pos_x());
        ui.set_cursor_pos_x(x);
        ui.text_disabled(text);
    }

    ui.spacing();

    if app.snap.projects.is_empty() {
        ui.text_disabled("No projects in the workspace root yet. Use \"New project\".");
        return;
    }

    let mut selected = app.selected_project.clone();
    let rows = app.snap.projects.clone();

    let detail = selected
        .as_ref()
        .and_then(|id| rows.iter().find(|p| &p.id == id));
    let avail_w = ui.content_region_avail_width();
    let detail_w = if detail.is_some() { 340.0 } else { 0.0 };

    ui.child_window("projects-table")
        .size([avail_w - detail_w - 8.0, 0.0])
        .build(ui, || {
            ui.table("projects-grid")
                .flags(TableFlags::RESIZABLE | TableFlags::ROW_BG | TableFlags::BORDERS)
                .column("Name")
                .done()
                .column("Stack")
                .done()
                .column("PRD")
                .done()
                .column("AGENTS")
                .done()
                .column("Questions")
                .done()
                .column("Skills")
                .done()
                .headers(true)
                .build(|ui| {
                    for p in &rows {
                        ui.table_next_row();
                        ui.table_next_column();
                        let is_sel = selected.as_deref() == Some(p.id.as_str());
                        if ui
                            .selectable_config(format!("{}##row-{}", p.name, p.id))
                            .selected(is_sel)
                            .flags(
                                SelectableFlags::SPAN_ALL_COLUMNS | SelectableFlags::ALLOW_OVERLAP,
                            )
                            .build()
                        {
                            selected = Some(p.id.clone());
                        }
                        ui.table_next_column();
                        ui.text(p.stack.join(", "));
                        ui.table_next_column();
                        ui.text(tick(p.has_prd));
                        ui.table_next_column();
                        ui.text(tick(p.has_agents));
                        ui.table_next_column();
                        if p.pending > 0 {
                            ui.text(format!("{} pending", p.pending));
                        } else if p.clarifications > 0 {
                            ui.text_disabled(format!("{} answered", p.clarifications));
                        } else {
                            ui.text_disabled("-");
                        }
                        ui.table_next_column();
                        ui.text(p.skills.to_string());
                    }
                });
        });

    if let Some(p) = detail {
        ui.same_line();
        ui.child_window("project-detail")
            .size([0.0, 0.0])
            .border(true)
            .build(ui, || {
                detail_pane(ui, p);
                ui.spacing();
                if ui.button("Close") {
                    selected = None;
                }
            });
    }

    app.selected_project = selected;
}

fn tick(present: bool) -> &'static str {
    if present {
        "yes"
    } else {
        "-"
    }
}

fn detail_pane(ui: &Ui, p: &crate::state::ProjectRow) {
    ui.text(&p.name);
    ui.separator();
    ui.spacing();

    ui.text_disabled("id");
    ui.same_line();
    ui.text(&p.id);

    ui.text_disabled("stack");
    ui.same_line();
    if p.stack.is_empty() {
        ui.text_disabled("-");
    } else {
        ui.text(p.stack.join(", "));
    }

    ui.text_disabled("PRD");
    ui.same_line();
    ui.text(if p.has_prd {
        "PRD.md present"
    } else {
        "not generated"
    });

    ui.text_disabled("AGENTS.md");
    ui.same_line();
    ui.text(if p.has_agents {
        "present"
    } else {
        "not generated"
    });

    ui.text_disabled("clarifications");
    ui.same_line();
    ui.text(format!("{} total, {} pending", p.clarifications, p.pending));

    ui.text_disabled("skills");
    ui.same_line();
    ui.text(p.skills.to_string());

    if !p.description.is_empty() {
        ui.spacing();
        ui.text_disabled("description");
        ui.text_wrapped(&p.description);
    }

    ui.spacing();
    ui.text_disabled("path");
    ui.text_wrapped(p.dir.display().to_string());

    ui.spacing();
    ui.separator();
    ui.text_disabled("Next steps are CLI-only for now:");
    ui.text(format!("rolen project interview --name {}", p.id));
    ui.text(format!("rolen project build --name {}", p.id));
}
