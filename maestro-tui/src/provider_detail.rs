//! Provider detail window (FR-10.2): capability matrix + quota info.

use appcui::prelude::*;
use maestro_providers as providers;

#[derive(ListItem)]
struct ModelRow {
    #[Column(name: "&Model", width: 38)]
    id: String,
    #[Column(name: "Context", width: 10, align: right)]
    ctx: String,
    #[Column(name: "Vision", width: 7)]
    vision: String,
    #[Column(name: "Tools", width: 6)]
    tools: String,
}

#[ModalWindow(events = ButtonEvents)]
pub struct ProviderDetail {
    l_info: Handle<Label>,
    lv: Handle<ListView<ModelRow>>,
    b_close: Handle<Button>,
}

impl ProviderDetail {
    pub fn new(provider_id: &str) -> Self {
        let mut w = Self {
            base: ModalWindow::new(
                &format!("Provider: {provider_id}"),
                layout!("a:c,w:78,h:24"),
                window::Flags::Sizeable,
            ),
            l_info: Handle::None,
            lv: Handle::None,
            b_close: Handle::None,
        };

        let reg = providers::ProviderRegistry::load().unwrap_or_default();
        let mut info = format!("provider '{provider_id}' not found");
        if let Some(p) = reg.get(provider_id) {
            info = format!(
                "type: {:?}  auth: {:?}  key: {}",
                p.ptype,
                p.auth,
                if p.key_ref.is_some() {
                    "stored in keychain"
                } else {
                    "—"
                }
            );
            if let Some(ep) = &p.endpoint {
                info.push_str(&format!("\nendpoint: {ep}"));
            }
            if let Some(t) = &p.tunnel {
                info.push_str(&format!(
                    "\nssh tunnel: {}@{}:{} → 127.0.0.1:{} (remote :{})",
                    t.user, t.host, t.port, t.local_port, t.remote_port
                ));
            }
            if let Some(pct) = providers::routing::remaining_pct(&p.id) {
                info.push_str(&format!("\nquota remaining: {pct}% of plan limit"));
            }
        }
        w.l_info = w.add(label!("x:1,y:0,w:74,h:3,text:''"));
        w.lv = w.add(listview!(
            "class: ModelRow,l:1,t:3,r:1,b:2,flags: [ScrollBars]"
        ));
        w.b_close = w.add(button!("'&Close',l:33,b:0,w:12"));

        let h = w.l_info;
        if let Some(l) = w.control_mut(h) {
            l.set_caption(&info);
        }
        let lv = w.lv;
        if let Some(lv) = w.control_mut(lv) {
            if let Some(p) = reg.get(provider_id) {
                for m in &p.models {
                    lv.add(ModelRow {
                        id: m.id.clone(),
                        ctx: m
                            .context_tokens
                            .map(|c| c.to_string())
                            .unwrap_or_else(|| "?".into()),
                        vision: if m.vision { "✓" } else { "—" }.into(),
                        tools: if m.tools { "✓" } else { "—" }.into(),
                    });
                }
            }
        }
        w
    }
}

impl ButtonEvents for ProviderDetail {
    fn on_pressed(&mut self, handle: Handle<Button>) -> EventProcessStatus {
        if handle == self.b_close {
            self.exit();
            return EventProcessStatus::Processed;
        }
        EventProcessStatus::Ignored
    }
}
