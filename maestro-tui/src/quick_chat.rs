//! Quick Chat (FR-10.5): ad-hoc conversation with a single provider/model.
//! Sends run on a background task; tokens are ledgered like any session.

use appcui::prelude::*;
use maestro_core::types::ProviderType;
use maestro_providers as providers;
use providers::chat::ChatMessage;

/// Output cap per reply. `ChatRequest::single` caps at 256, which is right for
/// a health probe and far too small for an answer.
const MAX_REPLY_TOKENS: u32 = 2048;

pub enum ChatMsg {
    Done {
        text: String,
        tokens_in: u64,
        tokens_out: u64,
        cost: f64,
        latency_ms: u64,
    },
    Failed(String),
}

/// Everything the worker needs. A struct rather than a tuple because the
/// conversation now travels with it.
struct ChatJob {
    provider_id: String,
    model: String,
    /// The whole conversation, ending with the message just typed.
    history: Vec<ChatMessage>,
    /// Stable for the life of the window, so the ledger shows one session per
    /// conversation instead of one per message.
    session_id: String,
    /// Totals from earlier turns, so the session row accumulates.
    prior_in: u64,
    prior_out: u64,
    prior_cost: f64,
}

static CHAT_JOB: std::sync::Mutex<Option<ChatJob>> = std::sync::Mutex::new(None);

fn chat_worker(conector: &BackgroundTaskConector<ChatMsg, bool>) {
    let job = CHAT_JOB.lock().unwrap().take();
    let Some(job) = job else {
        return;
    };
    let ChatJob {
        provider_id,
        model,
        history,
        session_id,
        prior_in,
        prior_out,
        prior_cost,
    } = job;
    // The whole conversation goes over the wire, not just the last line; the
    // ledger bookkeeping lives in maestro-providers so the GUI shares it.
    let result = providers::conversation::send(
        &provider_id,
        &model,
        history,
        &session_id,
        providers::conversation::Totals {
            tokens_in: prior_in,
            tokens_out: prior_out,
            cost: prior_cost,
        },
        MAX_REPLY_TOKENS,
    );
    conector.notify(match result {
        Ok(turn) => ChatMsg::Done {
            text: turn.text,
            tokens_in: turn.tokens_in,
            tokens_out: turn.tokens_out,
            cost: turn.cost,
            latency_ms: turn.latency_ms,
        },
        Err(e) => ChatMsg::Failed(e.to_string()),
    });
}

#[ModalWindow(events = ButtonEvents+ComboBoxEvents+BackgroundTaskEvents<ChatMsg,bool>)]
pub struct QuickChat {
    cb_provider: Handle<ComboBox>,
    cb_model: Handle<ComboBox>,
    t_log: Handle<TextArea>,
    t_input: Handle<TextField>,
    b_send: Handle<Button>,
    b_close: Handle<Button>,
    l_status: Handle<Label>,
    bt: Handle<BackgroundTask<ChatMsg, bool>>,
    /// The conversation as the model sees it. Without this every send was a
    /// brand-new one-message request and the model never saw what came before.
    history: Vec<ChatMessage>,
    session_id: String,
    /// One send at a time: the worker takes its parameters from a single
    /// global slot, so a second concurrent send would find it empty and be
    /// silently dropped.
    busy: bool,
    total_in: u64,
    total_out: u64,
    total_cost: f64,
}

impl QuickChat {
    pub fn new() -> Self {
        let mut w = Self {
            base: ModalWindow::new(
                "Quick Chat",
                layout!("a:c,w:90,h:26"),
                window::Flags::Sizeable,
            ),
            cb_provider: Handle::None,
            cb_model: Handle::None,
            t_log: Handle::None,
            t_input: Handle::None,
            b_send: Handle::None,
            b_close: Handle::None,
            l_status: Handle::None,
            bt: Handle::None,
            history: Vec::new(),
            session_id: format!(
                "qc-{}",
                chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
            ),
            busy: false,
            total_in: 0,
            total_out: 0,
            total_cost: 0.0,
        };
        w.add(label!("'&Provider:',l:1,t:0,w:10"));
        let mut cbp = ComboBox::new(layout!("l:11,t:0,w:26"), combobox::Flags::None);
        let reg = providers::ProviderRegistry::load().unwrap_or_default();
        for p in reg.list() {
            if p.ptype != ProviderType::Cli {
                cbp.add_item(combobox::Item::new(&p.id, &format!("{:?}", p.ptype)));
            }
        }
        w.cb_provider = w.add(cbp);

        w.add(label!("'&Model:',l:40,t:0,w:7"));
        w.cb_model = w.add(ComboBox::new(
            layout!("l:48,t:0,w:36"),
            combobox::Flags::None,
        ));

        w.l_status = w.add(label!("'—',l:1,t:2,r:1,h:1"));
        w.t_log = w.add(textarea!(
            "'',l:1,t:3,r:1,b:3,flags: [ReadOnly, ScrollBars]"
        ));
        w.t_input = w.add(textfield!("l:1,b:2,r:12,h:1"));
        w.b_send = w.add(button!("'&Send',r:1,b:2,w:10"));
        w.b_close = w.add(button!("'&Close',r:1,b:0,w:10"));

        w.fill_models();
        w
    }

    fn selected_provider(&self) -> Option<String> {
        self.control(self.cb_provider)
            .and_then(|cb| cb.selected_item().map(|i| i.value().to_string()))
    }

    fn selected_model(&self) -> Option<String> {
        self.control(self.cb_model)
            .and_then(|cb| cb.selected_item().map(|i| i.value().to_string()))
    }

    fn fill_models(&mut self) {
        let models: Vec<String> = self
            .selected_provider()
            .and_then(|pid| {
                providers::ProviderRegistry::load()
                    .ok()?
                    .get(&pid)
                    .map(|p| p.models.iter().map(|m| m.id.clone()).collect())
            })
            .unwrap_or_default();
        let h = self.cb_model;
        if let Some(cb) = self.control_mut(h) {
            cb.clear();
            for m in models {
                cb.add_item(combobox::Item::new(&m, ""));
            }
            if cb.count() > 0 {
                cb.set_index(0);
            }
        }
    }

    fn append_log(&mut self, line: &str) {
        let h = self.t_log;
        if let Some(t) = self.control_mut(h) {
            let mut text = t.text().to_string();
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            text.push_str(line);
            t.set_text(&text);
        }
    }

    fn set_status(&mut self, msg: &str) {
        let h = self.l_status;
        if let Some(l) = self.control_mut(h) {
            l.set_caption(msg);
        }
    }

    fn send(&mut self) {
        if self.busy {
            self.set_status("still waiting for the previous reply …");
            return;
        }
        let (Some(provider), Some(model)) = (self.selected_provider(), self.selected_model())
        else {
            self.set_status("pick provider and model first");
            return;
        };
        let prompt = self
            .control(self.t_input)
            .map(|t| t.text().to_string())
            .unwrap_or_default();
        if prompt.trim().is_empty() {
            return;
        }
        self.append_log(&format!("👤 {prompt}"));
        let h = self.t_input;
        if let Some(t) = self.control_mut(h) {
            t.set_text("");
        }
        self.history.push(ChatMessage::user(prompt));
        *CHAT_JOB.lock().unwrap() = Some(ChatJob {
            provider_id: provider.clone(),
            model: model.clone(),
            history: self.history.clone(),
            session_id: self.session_id.clone(),
            prior_in: self.total_in,
            prior_out: self.total_out,
            prior_cost: self.total_cost,
        });
        self.busy = true;
        self.bt = BackgroundTask::<ChatMsg, bool>::run(chat_worker, self.handle());
        self.set_status(&format!(
            "→ {provider}/{model} … ({} turns)",
            self.history.len()
        ));
    }
}

impl ComboBoxEvents for QuickChat {
    fn on_selection_changed(&mut self, handle: Handle<ComboBox>) -> EventProcessStatus {
        if handle == self.cb_provider {
            self.fill_models();
        }
        EventProcessStatus::Processed
    }
}

impl ButtonEvents for QuickChat {
    fn on_pressed(&mut self, handle: Handle<Button>) -> EventProcessStatus {
        if handle == self.b_send {
            self.send();
            return EventProcessStatus::Processed;
        }
        if handle == self.b_close {
            self.exit();
            return EventProcessStatus::Processed;
        }
        EventProcessStatus::Ignored
    }
}

impl BackgroundTaskEvents<ChatMsg, bool> for QuickChat {
    fn on_update(
        &mut self,
        value: ChatMsg,
        _: &BackgroundTask<ChatMsg, bool>,
    ) -> EventProcessStatus {
        match value {
            ChatMsg::Done {
                text,
                tokens_in,
                tokens_out,
                cost,
                latency_ms,
            } => {
                self.busy = false;
                self.total_in += tokens_in;
                self.total_out += tokens_out;
                self.total_cost += cost;
                self.append_log(&format!("🤖 {}", text.trim()));
                // Feed the reply back so the next turn has the full context.
                self.history.push(ChatMessage::assistant(text.trim()));
                self.set_status(&format!(
                    "{tokens_in} in / {tokens_out} out · {latency_ms} ms · session {} in / {} out",
                    self.total_in, self.total_out
                ));
            }
            ChatMsg::Failed(e) => {
                self.busy = false;
                // Drop the turn that never got an answer: leaving it would send
                // two user messages in a row, which some providers reject.
                self.history.pop();
                self.set_status(&format!("failed: {e} (message not kept in history)"));
            }
        }
        EventProcessStatus::Processed
    }

    fn on_query(&mut self, _: ChatMsg, _: &BackgroundTask<ChatMsg, bool>) -> bool {
        false
    }

    fn on_finish(&mut self, _: &BackgroundTask<ChatMsg, bool>) -> EventProcessStatus {
        EventProcessStatus::Processed
    }
}
