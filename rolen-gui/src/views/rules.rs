//! Rules: the routing table, and a dry-run that answers "where would this role
//! go right now?".
//!
//! Editing is still CLI-only. The dry-run is here because it is the part that
//! needs live provider state, and therefore the part a static YAML file cannot
//! answer on its own.

use dear_imgui_rs::{TableFlags, Ui};

use crate::app::RoleNApp;
use crate::dialogs::{ERROR, OK};
use crate::jobs::{self, DryRun};

pub fn show(app: &mut RoleNApp, ui: &Ui) {
    let running = app.jobs.is_running(jobs::DRY_RUN);

    ui.text("Role");
    ui.same_line();
    ui.set_next_item_width(200.0);
    let preview = app.dry_run_role.clone();
    if let Some(combo) = ui.begin_combo("##dry-run-role", preview) {
        // Built-in roles plus anything the rule file actually mentions, so a
        // custom role is still reachable.
        let mut roles: Vec<String> = rolen_core::rules::BUILT_IN_ROLES
            .iter()
            .map(|r| r.to_string())
            .collect();
        for rule in &app.snap.rules {
            if !roles.contains(&rule.role) {
                roles.push(rule.role.clone());
            }
        }
        for role in roles {
            if ui
                .selectable_config(&role)
                .selected(app.dry_run_role == role)
                .build()
            {
                app.dry_run_role = role;
            }
        }
        combo.end();
    }
    ui.same_line();

    ui.with_disabled_if(running, || {
        if ui.button("Dry-run") {
            app.start_dry_run();
        }
    });
    if ui.is_item_hovered() {
        ui.tooltip_text(
            "Health-checks every provider, then evaluates the rules against \
             what is actually reachable.",
        );
    }

    if running {
        ui.same_line();
        if ui.button("Cancel") {
            app.cancel_dry_run();
        }
        if ui.is_item_hovered() {
            ui.tooltip_text(
                "Stops before the next provider. The request already in flight \
                 cannot be aborted, so this can still take up to its timeout.",
            );
        }
        if let Some(progress) = &app.dry_run_progress {
            ui.same_line();
            ui.text_disabled(progress);
        }
    }

    ui.spacing();

    if let Some(outcome) = app.dry_run_result.clone() {
        outcome_ui(ui, &outcome);
        ui.spacing();
        ui.separator();
        ui.spacing();
    }

    if app.snap.rules.is_empty() {
        ui.text_disabled("No rules yet. Seed them from your providers with: rolen rule init");
        return;
    }

    ui.table("rules-grid")
        .flags(TableFlags::RESIZABLE | TableFlags::ROW_BG | TableFlags::BORDERS)
        .column("Rule")
        .done()
        .column("Role")
        .done()
        .column("Prio")
        .done()
        .column("Min quota")
        .done()
        .column("Fallback chain")
        .done()
        .headers(true)
        .build(|ui| {
            for r in &app.snap.rules {
                ui.table_next_row();
                ui.table_next_column();
                ui.text(&r.id);
                if (r.conditions > 0 || r.project_scope.is_some()) && ui.is_item_hovered() {
                    ui.tooltip_text(format!(
                        "{} condition(s){}",
                        r.conditions,
                        match &r.project_scope {
                            Some(s) => format!("; scoped to project '{s}'"),
                            None => String::new(),
                        }
                    ));
                }
                ui.table_next_column();
                ui.text(&r.role);
                ui.table_next_column();
                ui.text(r.priority.to_string());
                ui.table_next_column();
                match r.min_quota_pct {
                    Some(pct) => ui.text(format!("{pct}%")),
                    None => ui.text_disabled("-"),
                }
                ui.table_next_column();
                ui.text(r.chain.join("  ->  "));
            }
        });
}

fn outcome_ui(ui: &Ui, outcome: &DryRun) {
    match outcome {
        DryRun::Decided {
            role,
            rule_id,
            provider,
            model,
            explanation,
            skipped,
        } => {
            ui.text(format!("{role} ->"));
            ui.same_line();
            ui.text_colored(OK, format!("{provider}/{model}"));
            ui.same_line();
            ui.text_disabled(format!("via rule '{rule_id}'"));
            if !explanation.is_empty() {
                // Core writes these strings for a terminal; see crate::text.
                ui.text_wrapped(crate::text::renderable(explanation));
            }
            if !skipped.is_empty() {
                ui.spacing();
                ui.text_disabled("skipped:");
                for (entry, reason) in skipped {
                    ui.text_disabled(crate::text::renderable(&format!("    {entry} - {reason}")));
                }
            }
        }
        DryRun::NoRoute { role, reason } => {
            ui.text_colored(ERROR, format!("no route for '{role}'"));
            ui.text_wrapped(crate::text::renderable(reason));
        }
        DryRun::Cancelled => {
            ui.text_disabled("dry-run cancelled");
        }
    }
}
