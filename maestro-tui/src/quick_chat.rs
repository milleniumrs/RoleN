//! Quick Chat (FR-10.5): ad-hoc conversation with a single provider/model.
//! Sends run on a background task; tokens are ledgered like any session.

use appcui::prelude::*;
use maestro_core::types::{LedgerEntry, ProviderType, Session, SessionState};
use maestro_providers as providers;

pub enum ChatMsg {
    Done {
        text: String,
        tokens_in: u64,
        tokens_out: u64,
        latency_ms: u64,
    },
    Failed(String),
}

static CHAT_JOB: std::sync::Mutex<Option<(String, String, String)>> = std::sync::Mutex::new(None);

fn chat_worker(conector: &BackgroundTaskConector<ChatMsg, bool>) {
    let job = CHAT_JOB.lock().unwrap().take();
    let Some((provider_id, model, prompt)) = job else {
        return;
    };
    let result = (|| -> anyhow::Result<ChatMsg> {
        let reg = providers::ProviderRegistry::load()?;
        let p = reg
            .get(&provider_id)
            .ok_or_else(|| anyhow::anyhow!("provider '{provider_id}' not found"))?
            .clone();
        let req = providers::chat::ChatRequest::single(model.clone(), prompt.clone());
        let started = std::time::Instant::now();
        let resp = providers::client::chat(&p, &req)?;
        let latency = started.elapsed().as_millis() as u64;

        // session + ledger (FR-4.6 / FR-9)
        let session_id = format!(
            "qc-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        if let Ok(ledger) = maestro_core::ledger::Ledger::open_default() {
            let cost = providers::test::estimate_cost(&p, &model, resp.tokens_in, resp.tokens_out);
            let _ = ledger.record(&LedgerEntry {
                id: format!(
                    "le-{}",
                    chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
                ),
                session_id: session_id.clone(),
                provider_id: provider_id.clone(),
                tokens_in: resp.tokens_in,
                tokens_out: resp.tokens_out,
                cost,
                latency_ms: Some(resp.latency_ms),
                ts: chrono::Utc::now(),
            });
            let _ = ledger.upsert_session(&Session {
                id: session_id,
                task_id: None,
                provider_id,
                model,
                state: SessionState::Done,
                tokens_in: resp.tokens_in,
                tokens_out: resp.tokens_out,
                cost,
                started: chrono::Utc::now(),
                transcript_path: None,
            });
        }
        Ok(ChatMsg::Done {
            text: resp.text,
            tokens_in: resp.tokens_in,
            tokens_out: resp.tokens_out,
            latency_ms: latency,
        })
    })();
    conector.notify(match result {
        Ok(m) => m,
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
        *CHAT_JOB.lock().unwrap() = Some((provider.clone(), model.clone(), prompt));
        self.bt = BackgroundTask::<ChatMsg, bool>::run(chat_worker, self.handle());
        self.set_status(&format!("→ {provider}/{model} …"));
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
                latency_ms,
            } => {
                self.append_log(&format!("🤖 {}", text.trim()));
                self.set_status(&format!(
                    "{tokens_in} in / {tokens_out} out · {latency_ms} ms · ledgered"
                ));
            }
            ChatMsg::Failed(e) => self.set_status(&format!("failed: {e}")),
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
