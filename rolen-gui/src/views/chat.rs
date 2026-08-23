//! Chat: a real multi-turn conversation with one provider/model.
//!
//! The whole history is sent on every turn, so the model sees the
//! conversation. There is no token streaming to show, because no provider
//! client in the workspace requests it - `stream` is hard-coded false - so a
//! reply arrives in one piece behind a "sending" note.

use dear_imgui_rs::Ui;

use super::{fmt_cost, fmt_tokens};
use crate::app::RoleNApp;
use crate::dialogs::{ACCENT, OK};
use crate::jobs;

pub fn show(app: &mut RoleNApp, ui: &Ui) {
    let sending = app.jobs.is_running(jobs::CHAT);

    // CLI providers are PTY-wrapped agents, not chat endpoints.
    let choices: Vec<_> = app.snap.providers.iter().filter(|p| !p.is_cli).collect();

    ui.text("Provider");
    ui.same_line();
    ui.set_next_item_width(180.0);
    let provider_preview = app.chat_provider.clone().unwrap_or_else(|| "-".into());
    if let Some(combo) = ui.begin_combo("##chat-provider", provider_preview) {
        for p in &choices {
            if ui
                .selectable_config(&p.id)
                .selected(app.chat_provider.as_deref() == Some(&p.id))
                .build()
            {
                app.chat_provider = Some(p.id.clone());
                // The old model belongs to the old provider.
                app.chat_model = p.model_ids.first().cloned();
            }
        }
        combo.end();
    }
    ui.same_line();

    ui.text("Model");
    ui.same_line();
    ui.set_next_item_width(180.0);
    let models: Vec<String> = app
        .chat_provider
        .as_ref()
        .and_then(|id| choices.iter().find(|p| &p.id == id))
        .map(|p| p.model_ids.clone())
        .unwrap_or_default();
    let model_preview = app.chat_model.clone().unwrap_or_else(|| "-".into());
    if let Some(combo) = ui.begin_combo("##chat-model", model_preview) {
        for m in &models {
            if ui
                .selectable_config(m)
                .selected(app.chat_model.as_deref() == Some(m))
                .build()
            {
                app.chat_model = Some(m.clone());
            }
        }
        combo.end();
    }
    ui.same_line();

    ui.with_disabled_if(app.chat_history.is_empty(), || {
        if ui.button("New chat") {
            app.reset_chat();
        }
    });
    if ui.is_item_hovered() {
        ui.tooltip_text("Clears the history and starts a new ledger session.");
    }

    if app.chat_totals.tokens_in + app.chat_totals.tokens_out > 0 {
        ui.same_line();
        let totals = format!(
            "{} in / {} out - {}",
            fmt_tokens(app.chat_totals.tokens_in),
            fmt_tokens(app.chat_totals.tokens_out),
            fmt_cost(app.chat_totals.cost)
        );
        let w = ui.calc_text_size(&totals)[0];
        let x =
            (ui.content_region_avail_width() + ui.cursor_pos_x() - w - 8.0).max(ui.cursor_pos_x());
        ui.set_cursor_pos_x(x);
        ui.text_disabled(totals);
    }

    ui.spacing();

    // Composer pinned to the bottom, transcript takes what is left.
    let avail = ui.content_region_avail();
    let input_h = ui.frame_height_with_spacing() + 4.0;
    let history_h = (avail[1] - input_h).max(40.0);

    ui.child_window("chat-history")
        .size([0.0, history_h])
        .build(ui, || {
            if app.chat_history.is_empty() {
                ui.text_disabled("No messages yet.");
            } else {
                for msg in &app.chat_history {
                    let (who, colour) = if msg.role == "user" {
                        ("you", ACCENT)
                    } else {
                        ("assistant", OK)
                    };
                    ui.text_colored(colour, who);
                    ui.text_wrapped(crate::text::renderable(&msg.content));
                    ui.spacing();
                }
                // Follow the conversation: jump to the newest message while a
                // reply is arriving or was just appended.
                if sending {
                    ui.set_scroll_here_y(1.0);
                }
            }
        });

    let ready = app.chat_provider.is_some() && app.chat_model.is_some();
    ui.with_disabled_if(sending || !ready, || {
        if ui.button(if sending { "Sending..." } else { "Send" }) && ready && !sending {
            app.send_chat();
        }
    });
    if (sending || !ready)
        && ui.is_item_hovered_with_flags(dear_imgui_rs::ItemHoveredFlags::ALLOW_WHEN_DISABLED)
    {
        ui.tooltip_text(if ready {
            "waiting for the reply"
        } else {
            "pick a provider and model first"
        });
    }
    ui.same_line();
    let width = ui.content_region_avail_width() - 8.0;
    ui.set_next_item_width(width.max(60.0));
    let entered = ui
        .input_text("##chat-input", &mut app.chat_input)
        .hint("Ask something; Enter sends")
        .enter_returns_true(true)
        .build();
    if entered && !sending && ready {
        app.send_chat();
        // Enter sends but keeps focus semantics; refocus the input so the
        // next message can be typed straight away.
        ui.set_keyboard_focus_here();
    }
}
