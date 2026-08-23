//! Providers: the registry as a table, a detail pane, and the two actions that
//! must go through the job system because they hit the network.

use dear_imgui_rs::{ItemHoveredFlags, SelectableFlags, TableFlags, Ui};

use super::{fmt_cost, fmt_tokens};
use crate::app::RoleNApp;
use crate::dialogs::{ERROR, OK};
use crate::jobs;

/// Tooltip on the item just drawn.
fn hint(ui: &Ui, text: &str) {
    if ui.is_item_hovered() {
        ui.tooltip_text(text);
    }
}

pub fn show(app: &mut RoleNApp, ui: &Ui) {
    if ui.button("Add provider") {
        app.add_provider.open();
    }
    ui.same_line();

    let detecting = app.jobs.is_running(jobs::DETECT);
    ui.with_disabled_if(detecting, || {
        if ui.button("Detect CLIs & Ollama") {
            app.jobs.spawn(jobs::DETECT, jobs::detect_and_register);
        }
    });
    hint(
        ui,
        "Probes local Ollama and looks for claude/codex/gemini/kimi on PATH, \
         then merges what it finds into the registry.",
    );
    ui.same_line();

    let checking = app.jobs.is_running(jobs::HEALTH_CHECK);
    ui.with_disabled_if(checking, || {
        if ui.button("Health check all") {
            app.jobs.spawn(jobs::HEALTH_CHECK, jobs::health_check_all);
        }
    });
    hint(
        ui,
        "One HTTP round trip per provider. An unreachable endpoint costs \
         up to 30 s, so this runs on a worker thread.",
    );
    ui.same_line();

    if ui.button("Refresh") {
        app.poller.refresh_now();
    }
    if detecting || checking {
        ui.same_line();
        ui.text_disabled("working...");
    }

    ui.spacing();

    if app.snap.providers.is_empty() {
        ui.text_disabled("No providers registered. Try \"Detect CLIs & Ollama\".");
        return;
    }

    // The detail pane is driven by this selection; keep a copy so the table
    // closure can mutate the selection without holding a borrow on the app.
    let mut selected = app.selected_provider.clone();
    let mut remove: Option<String> = None;
    let rows = app.snap.providers.clone();
    let health = app.health.clone();

    let detail = selected
        .as_ref()
        .and_then(|id| rows.iter().find(|p| &p.id == id));
    let avail_w = ui.content_region_avail_width();
    let detail_w = if detail.is_some() { 320.0 } else { 0.0 };

    ui.child_window("providers-table")
        .size([avail_w - detail_w - 8.0, 0.0])
        .build(ui, || {
            ui.table("providers-grid")
                .flags(TableFlags::RESIZABLE | TableFlags::ROW_BG | TableFlags::BORDERS)
                .column("Provider")
                .done()
                .column("Type")
                .done()
                .column("Status")
                .done()
                .column("Models")
                .done()
                .column("Quota")
                .done()
                .column("Today")
                .done()
                .headers(true)
                .build(|ui| {
                    for p in &rows {
                        ui.table_next_row();
                        ui.table_next_column();
                        let is_sel = selected.as_deref() == Some(p.id.as_str());
                        if ui
                            .selectable_config(format!("{}##row-{}", p.id, p.id))
                            .selected(is_sel)
                            .flags(
                                SelectableFlags::SPAN_ALL_COLUMNS | SelectableFlags::ALLOW_OVERLAP,
                            )
                            .build()
                        {
                            selected = Some(p.id.clone());
                        }
                        ui.table_next_column();
                        ui.text(&p.kind);
                        ui.table_next_column();
                        match health.get(&p.id) {
                            Some(h) if h.ok => {
                                ui.text_colored(OK, format!("ok - {} ms", h.latency_ms));
                            }
                            Some(h) => {
                                ui.text_colored(ERROR, "failed");
                                hint(ui, &h.detail);
                            }
                            None => ui.text_disabled("unchecked"),
                        }
                        ui.table_next_column();
                        ui.text(p.models.to_string());
                        ui.table_next_column();
                        match p.quota_pct {
                            Some(pct) => ui.text(format!("{pct}%")),
                            None => ui.text_disabled("-"),
                        }
                        ui.table_next_column();
                        ui.text(fmt_tokens(p.tokens_today));
                    }
                });
        });

    if let Some(p) = detail {
        ui.same_line();
        ui.child_window("provider-detail")
            .size([0.0, 0.0])
            .border(true)
            .build(ui, || {
                detail_pane(ui, p, health.get(&p.id));
                ui.spacing();
                if ui.button("Close") {
                    selected = None;
                }
                ui.same_line();
                let removing = app.jobs.is_running(jobs::REMOVE_PROVIDER);
                ui.with_disabled_if(removing, || {
                    if ui.button("Remove") {
                        remove = Some(p.id.clone());
                    }
                });
                if ui.is_item_hovered_with_flags(ItemHoveredFlags::ALLOW_WHEN_DISABLED) {
                    ui.tooltip_text("Drops it from providers.toml and deletes its stored key.");
                }
            });
    }

    app.selected_provider = selected;

    if let Some(id) = remove {
        app.jobs
            .spawn(jobs::REMOVE_PROVIDER, move || jobs::remove_provider(id));
    }
}

fn detail_pane(ui: &Ui, p: &crate::state::ProviderRow, health: Option<&jobs::HealthRow>) {
    ui.text(&p.id);
    ui.separator();
    ui.spacing();

    ui.text_disabled("type");
    ui.same_line();
    ui.text(&p.kind);

    ui.text_disabled("endpoint");
    ui.same_line();
    match &p.endpoint {
        Some(e) => ui.text_wrapped(e),
        None => ui.text_disabled(if p.is_cli { "n/a (PTY-wrapped)" } else { "-" }),
    }

    ui.text_disabled("key");
    ui.same_line();
    ui.text(if p.has_key {
        "stored in keychain/vault"
    } else {
        "none"
    });

    ui.text_disabled("models");
    ui.same_line();
    ui.text(p.models.to_string());

    ui.text_disabled("quota left");
    ui.same_line();
    match p.quota_pct {
        Some(pct) => ui.text(format!("{pct}%")),
        None => ui.text_disabled("no plan limit set"),
    }

    ui.text_disabled("today");
    ui.same_line();
    ui.text(format!(
        "{} tok - {}",
        fmt_tokens(p.tokens_today),
        fmt_cost(p.cost_today)
    ));

    if let Some(h) = health {
        ui.spacing();
        ui.separator();
        ui.text_disabled("last health check");
        ui.text(format!(
            "{} - {} ms - {} models",
            if h.ok { "ok" } else { "failed" },
            h.latency_ms,
            h.models
        ));
        if !h.detail.is_empty() {
            ui.text_wrapped(&h.detail);
        }
    }
}
