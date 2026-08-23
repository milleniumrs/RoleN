//! Chat: a real multi-turn conversation with one provider/model.
//!
//! The whole history is sent on every turn, so the model sees the
//! conversation. There is no token streaming to show, because no provider
//! client in the workspace requests it - `stream` is hard-coded false - so a
//! reply arrives in one piece behind a spinner.

use eframe::egui;

use super::{fmt_cost, fmt_tokens};
use crate::app::RoleNApp;
use crate::jobs;

pub fn show(app: &mut RoleNApp, ui: &mut egui::Ui) {
    let sending = app.jobs.is_running(jobs::CHAT);

    ui.horizontal(|ui| {
        // CLI providers are PTY-wrapped agents, not chat endpoints.
        let choices: Vec<_> = app.snap.providers.iter().filter(|p| !p.is_cli).collect();

        ui.label("Provider");
        egui::ComboBox::from_id_salt("chat-provider")
            .selected_text(app.chat_provider.clone().unwrap_or_else(|| "-".into()))
            .show_ui(ui, |ui| {
                for p in &choices {
                    if ui
                        .selectable_label(app.chat_provider.as_deref() == Some(&p.id), &p.id)
                        .clicked()
                    {
                        app.chat_provider = Some(p.id.clone());
                        // The old model belongs to the old provider.
                        app.chat_model = p.model_ids.first().cloned();
                    }
                }
            });

        ui.label("Model");
        let models: Vec<String> = app
            .chat_provider
            .as_ref()
            .and_then(|id| choices.iter().find(|p| &p.id == id))
            .map(|p| p.model_ids.clone())
            .unwrap_or_default();
        egui::ComboBox::from_id_salt("chat-model")
            .selected_text(app.chat_model.clone().unwrap_or_else(|| "-".into()))
            .show_ui(ui, |ui| {
                for m in &models {
                    ui.selectable_value(&mut app.chat_model, Some(m.clone()), m);
                }
            });

        if ui
            .add_enabled(!app.chat_history.is_empty(), egui::Button::new("New chat"))
            .on_hover_text("Clears the history and starts a new ledger session.")
            .clicked()
        {
            app.reset_chat();
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if app.chat_totals.tokens_in + app.chat_totals.tokens_out > 0 {
                ui.weak(format!(
                    "{} in / {} out · {}",
                    fmt_tokens(app.chat_totals.tokens_in),
                    fmt_tokens(app.chat_totals.tokens_out),
                    fmt_cost(app.chat_totals.cost)
                ));
            }
        });
    });

    ui.add_space(6.0);

    // Composer pinned to the bottom, transcript takes what is left.
    egui::Panel::bottom("chat-composer")
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let ready = app.chat_provider.is_some() && app.chat_model.is_some();
                let send = ui
                    .add_enabled(
                        !sending && ready,
                        egui::Button::new(if sending { "Sending..." } else { "Send" }),
                    )
                    .on_disabled_hover_text(if ready {
                        "waiting for the reply"
                    } else {
                        "pick a provider and model first"
                    });
                if sending {
                    ui.spinner();
                }
                let input = ui.add_sized(
                    [ui.available_width(), 24.0],
                    egui::TextEdit::singleline(&mut app.chat_input)
                        .hint_text("Ask something; Enter sends"),
                );
                let entered = input.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (send.clicked() || entered) && !sending && ready {
                    app.send_chat();
                }
            });
        });

    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            if app.chat_history.is_empty() {
                ui.weak("No messages yet.");
                return;
            }
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for msg in &app.chat_history {
                        let (who, colour) = if msg.role == "user" {
                            ("you", egui::Color32::from_rgb(0x15, 0x65, 0xc0))
                        } else {
                            ("assistant", egui::Color32::from_rgb(0x2e, 0x7d, 0x32))
                        };
                        ui.colored_label(colour, who);
                        ui.label(crate::text::renderable(ui.ctx(), &msg.content));
                        ui.add_space(8.0);
                    }
                });
        });
}
