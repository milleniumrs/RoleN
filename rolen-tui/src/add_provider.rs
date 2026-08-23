//! "Add Provider" wizard dialog (PRD FR-1.2, TUI-DESIGN.md §3.3).

use appcui::prelude::*;
use rolen_core::types::{Model, Provider, ProviderType};
use rolen_providers as providers;

#[ModalWindow(events = ButtonEvents+RadioBoxEvents, response = bool)]
pub struct AddProviderDialog {
    t_id: Handle<TextField>,
    rb_api: Handle<RadioBox>,
    rb_cli: Handle<RadioBox>,
    rb_ollama_local: Handle<RadioBox>,
    rb_ollama_cloud: Handle<RadioBox>,
    t_endpoint: Handle<TextField>,
    t_key: Handle<Password>,
    l_status: Handle<Label>,
    b_discover: Handle<Button>,
    b_save: Handle<Button>,
    b_cancel: Handle<Button>,
    ptype: ProviderType,
    discovered: Vec<Model>,
}

impl AddProviderDialog {
    pub fn new() -> Self {
        let mut w = Self {
            base: ModalWindow::new(
                "Add Provider",
                layout!("a:c,w:68,h:21"),
                window::Flags::None,
            ),
            t_id: Handle::None,
            rb_api: Handle::None,
            rb_cli: Handle::None,
            rb_ollama_local: Handle::None,
            rb_ollama_cloud: Handle::None,
            t_endpoint: Handle::None,
            t_key: Handle::None,
            l_status: Handle::None,
            b_discover: Handle::None,
            b_save: Handle::None,
            b_cancel: Handle::None,
            ptype: ProviderType::Api,
            discovered: Vec::new(),
        };
        w.add(label!(
            "'&Id (unique, e.g. kimi, claude, ollama-local):',x:2,y:1,w:45"
        ));
        w.t_id = w.add(textfield!("x:2,y:2,w:40"));

        w.add(label!("'Type:',x:2,y:4,w:10"));
        w.rb_api = w.add(radiobox!(
            "'api (OpenAI-compatible / Anthropic)',x:2,y:5,w:40,select:true"
        ));
        w.rb_cli = w.add(radiobox!(
            "'cli (claude / codex / gemini … — PTY adapter in M5)',x:2,y:6,w:52"
        ));
        w.rb_ollama_local = w.add(radiobox!("'ollama-local',x:2,y:7,w:20"));
        w.rb_ollama_cloud = w.add(radiobox!("'ollama-cloud',x:24,y:7,w:20"));

        w.add(label!(
            "'&Endpoint (empty = default for the type):',x:2,y:9,w:45"
        ));
        w.t_endpoint = w.add(textfield!("x:2,y:10,w:62"));

        w.add(label!(
            "'&API key (stored in OS keychain/vault — never in config):',x:2,y:12,w:56"
        ));
        w.t_key = w.add(password!("x:2,y:13,w:62"));

        w.l_status = w.add(label!("'',x:2,y:15,w:62"));

        w.b_discover = w.add(button!("'&Discover models',x:2,y:17,w:20"));
        w.b_save = w.add(button!("'&Save',x:26,y:17,w:14"));
        w.b_cancel = w.add(button!("'&Cancel',x:44,y:17,w:14"));
        w
    }

    fn field_text(&self, h: Handle<TextField>) -> String {
        self.control(h)
            .map(|t| t.text().to_string())
            .unwrap_or_default()
    }

    fn key_text(&self) -> String {
        self.control(self.t_key)
            .map(|t| t.password().to_string())
            .unwrap_or_default()
    }

    fn set_status(&mut self, msg: &str) {
        let h = self.l_status;
        if let Some(l) = self.control_mut(h) {
            l.set_caption(msg);
        }
    }

    fn build_provider(&self) -> Result<Provider, String> {
        let id = self.field_text(self.t_id).trim().to_string();
        if id.is_empty() {
            return Err("id is required".into());
        }
        if id.contains(char::is_whitespace) {
            return Err("id must not contain whitespace".into());
        }
        let endpoint = {
            let e = self.field_text(self.t_endpoint).trim().to_string();
            if e.is_empty() {
                match self.ptype {
                    ProviderType::OllamaLocal => Some(providers::ollama::DEFAULT_LOCAL_BASE.into()),
                    ProviderType::OllamaCloud => Some(providers::ollama::DEFAULT_CLOUD_BASE.into()),
                    _ => None,
                }
            } else {
                Some(e)
            }
        };
        if self.ptype == ProviderType::Api && endpoint.is_none() {
            return Err(
                "endpoint is required for api providers (e.g. https://api.openai.com/v1)".into(),
            );
        }
        Ok(Provider {
            id,
            ptype: self.ptype,
            auth: Default::default(),
            tunnel: None,
            endpoint,
            cli_path: None,
            key_ref: None,
            models: self.discovered.clone(),
        })
    }

    fn on_discover(&mut self) {
        match self.build_provider() {
            Ok(p) => {
                let key = self.key_text();
                let key = if key.is_empty() { None } else { Some(key) };
                self.set_status("discovering…");
                match providers::client::list_models_with_key(&p, key.as_deref()) {
                    Ok(models) => {
                        self.discovered = models;
                        self.set_status(&format!(
                            "found {} models — they will be saved with the provider",
                            self.discovered.len()
                        ));
                    }
                    Err(e) => self.set_status(&format!("discovery failed: {e}")),
                }
            }
            Err(e) => self.set_status(&e),
        }
    }

    fn on_save(&mut self) {
        let mut provider = match self.build_provider() {
            Ok(p) => p,
            Err(e) => {
                self.set_status(&e);
                return;
            }
        };
        // store the key if one was entered
        let key = self.key_text();
        if !key.is_empty() {
            let kref = providers::registry::key_ref_for(&provider.id);
            if let Err(e) = rolen_core::secrets::set_secret(&kref, &key) {
                self.set_status(&format!("failed to store key: {e}"));
                return;
            }
            provider.key_ref = Some(kref);
        }
        // best-effort discovery if none ran yet
        if provider.models.is_empty() && provider.ptype != ProviderType::Cli {
            if let Ok(models) = providers::client::list_models(&provider) {
                provider.models = models;
            }
        }
        match providers::ProviderRegistry::load() {
            Ok(mut reg) => {
                reg.upsert(provider);
                if let Err(e) = reg.save() {
                    self.set_status(&format!("failed to save registry: {e}"));
                    return;
                }
                self.exit_with(true);
            }
            Err(e) => self.set_status(&format!("failed to load registry: {e}")),
        }
    }
}

impl RadioBoxEvents for AddProviderDialog {
    fn on_selected(&mut self, handle: Handle<RadioBox>) -> EventProcessStatus {
        let (ptype, endpoint_hint) = if handle == self.rb_api {
            (ProviderType::Api, None)
        } else if handle == self.rb_cli {
            (ProviderType::Cli, None)
        } else if handle == self.rb_ollama_local {
            (
                ProviderType::OllamaLocal,
                Some(providers::ollama::DEFAULT_LOCAL_BASE),
            )
        } else {
            (
                ProviderType::OllamaCloud,
                Some(providers::ollama::DEFAULT_CLOUD_BASE),
            )
        };
        self.ptype = ptype;
        if let Some(hint) = endpoint_hint {
            let h = self.t_endpoint;
            if let Some(t) = self.control_mut(h) {
                t.set_text(hint);
            }
        }
        if ptype == ProviderType::Cli {
            self.set_status(
                "cli providers: only registration is stored in M1; PTY wrapping arrives in M5",
            );
        }
        EventProcessStatus::Processed
    }
}

impl ButtonEvents for AddProviderDialog {
    fn on_pressed(&mut self, handle: Handle<Button>) -> EventProcessStatus {
        if handle == self.b_discover {
            self.on_discover();
            return EventProcessStatus::Processed;
        }
        if handle == self.b_save {
            self.on_save();
            return EventProcessStatus::Processed;
        }
        if handle == self.b_cancel {
            self.exit();
            return EventProcessStatus::Processed;
        }
        EventProcessStatus::Ignored
    }
}
