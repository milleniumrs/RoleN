//! Settings window (FR-10.7 / TUI-DESIGN.md §6): core sections, edits
//! config.toml. Full section inventory lands progressively; these are the
//! day-to-day knobs.

use appcui::prelude::*;
use maestro_core::config::Config;
use maestro_core::types::{AlertAction, QuestionMode};

#[ModalWindow(events = ButtonEvents)]
pub struct SettingsWindow {
    t_workspace: Handle<TextField>,
    cb_theme: Handle<ComboBox>,
    cb_mode: Handle<ComboBox>,
    t_global_cap: Handle<TextField>,
    t_provider_cap: Handle<TextField>,
    t_warn: Handle<TextField>,
    t_crit: Handle<TextField>,
    l_status: Handle<Label>,
    b_save: Handle<Button>,
    b_cancel: Handle<Button>,
}

impl SettingsWindow {
    pub fn new() -> Self {
        let cfg = Config::load().unwrap_or_default();
        let mut w = Self {
            base: ModalWindow::new("Settings", layout!("a:c,w:68,h:25"), window::Flags::None),
            t_workspace: Handle::None,
            cb_theme: Handle::None,
            cb_mode: Handle::None,
            t_global_cap: Handle::None,
            t_provider_cap: Handle::None,
            t_warn: Handle::None,
            t_crit: Handle::None,
            l_status: Handle::None,
            b_save: Handle::None,
            b_cancel: Handle::None,
        };
        w.add(label!("'Workspace root:',x:2,y:1,w:30"));
        w.t_workspace = w.add(textfield!("x:2,y:2,w:62"));

        w.add(label!("'Colour theme (applied on save):',x:2,y:4,w:46"));
        let mut cb_theme = ComboBox::new(layout!("x:2,y:5,w:34"), combobox::Flags::ShowDescription);
        for (name, desc) in crate::theme::AVAILABLE {
            cb_theme.add_item(combobox::Item::new(name, desc));
        }
        w.cb_theme = w.add(cb_theme);

        w.add(label!(
            "'Question mode (interview thoroughness):',x:2,y:7,w:44"
        ));
        let mut cb = ComboBox::new(layout!("x:2,y:8,w:30"), combobox::Flags::None);
        cb.add_item(combobox::Item::new("thorough", "ask everything (default)"));
        cb.add_item(combobox::Item::new("balanced", "fewer questions"));
        cb.add_item(combobox::Item::new("minimal", "only blockers"));
        w.cb_mode = w.add(cb);

        w.add(label!(
            "'Parallelism: global cap (0 = auto CPU heuristic):',x:2,y:10,w:52"
        ));
        w.t_global_cap = w.add(textfield!("x:2,y:11,w:10"));
        w.add(label!("'per-provider cap:',x:16,y:11,w:18"));
        w.t_provider_cap = w.add(textfield!("x:34,y:11,w:10"));

        w.add(label!("'Quota alerts — warn %:',x:2,y:13,w:24"));
        w.t_warn = w.add(textfield!("x:26,y:13,w:8"));
        w.add(label!("'critical %:',x:38,y:13,w:12"));
        w.t_crit = w.add(textfield!("x:50,y:13,w:8"));

        w.add(label!(
            "'Secrets: OS keychain → age vault fallback (MAESTRO_VAULT_PASSWORD).',x:2,y:15,w:64"
        ));
        w.add(label!(
            "'Force a backend with MAESTRO_SECRETS_BACKEND=vault|keychain.',x:2,y:16,w:64"
        ));

        w.l_status = w.add(label!("'',x:2,y:18,w:60"));
        w.b_save = w.add(button!("'&Save',x:22,y:20,w:12"));
        w.b_cancel = w.add(button!("'&Cancel',x:38,y:20,w:12"));

        // fill current values
        let ws = cfg.general.workspace_root.display().to_string();
        let gc = cfg.parallelism.global_cap.to_string();
        let pc = cfg.parallelism.per_provider_cap.to_string();
        let warn = cfg.quotas.warn_pct.to_string();
        let crit = cfg.quotas.crit_pct.to_string();
        let theme_idx = crate::theme::AVAILABLE
            .iter()
            .position(|(n, _)| {
                *n == crate::theme::to_name(crate::theme::resolve(&cfg.general.theme))
            })
            .unwrap_or(0) as u32;
        let mode_idx = match cfg.general.question_mode {
            QuestionMode::Thorough => 0,
            QuestionMode::Balanced => 1,
            QuestionMode::Minimal => 2,
        };
        let h = w.cb_theme;
        if let Some(cb) = w.control_mut(h) {
            cb.set_index(theme_idx);
        }
        let h = w.t_workspace;
        if let Some(t) = w.control_mut(h) {
            t.set_text(&ws);
        }
        let h = w.t_global_cap;
        if let Some(t) = w.control_mut(h) {
            t.set_text(&gc);
        }
        let h = w.t_provider_cap;
        if let Some(t) = w.control_mut(h) {
            t.set_text(&pc);
        }
        let h = w.t_warn;
        if let Some(t) = w.control_mut(h) {
            t.set_text(&warn);
        }
        let h = w.t_crit;
        if let Some(t) = w.control_mut(h) {
            t.set_text(&crit);
        }
        let h = w.cb_mode;
        if let Some(cb) = w.control_mut(h) {
            cb.set_index(mode_idx);
        }
        w
    }

    fn field(&self, h: Handle<TextField>) -> String {
        self.control(h)
            .map(|t| t.text().trim().to_string())
            .unwrap_or_default()
    }

    fn set_status(&mut self, msg: &str) {
        let h = self.l_status;
        if let Some(l) = self.control_mut(h) {
            l.set_caption(msg);
        }
    }

    fn on_save(&mut self) {
        let mut cfg = Config::load().unwrap_or_default();
        let ws = self.field(self.t_workspace);
        if !ws.is_empty() {
            cfg.general.workspace_root = ws.into();
        }
        cfg.general.theme = self
            .control(self.cb_theme)
            .and_then(|cb| cb.selected_item().map(|i| i.value().to_string()))
            .unwrap_or_else(|| "default".to_string());
        cfg.general.question_mode = match self
            .control(self.cb_mode)
            .and_then(|cb| cb.selected_item().map(|i| i.value().to_string()))
            .as_deref()
        {
            Some("balanced") => QuestionMode::Balanced,
            Some("minimal") => QuestionMode::Minimal,
            _ => QuestionMode::Thorough,
        };
        if let Ok(v) = self.field(self.t_global_cap).parse() {
            cfg.parallelism.global_cap = v;
        }
        if let Ok(v) = self.field(self.t_provider_cap).parse() {
            cfg.parallelism.per_provider_cap = v;
        }
        if let Ok(v) = self.field(self.t_warn).parse::<u8>() {
            cfg.quotas.warn_pct = v.min(100);
        }
        if let Ok(v) = self.field(self.t_crit).parse::<u8>() {
            cfg.quotas.crit_pct = v.min(100);
        }
        if cfg.quotas.warn_pct >= cfg.quotas.crit_pct && cfg.quotas.crit_pct < 100 {
            self.set_status("warn % must be below critical %");
            return;
        }
        cfg.quotas.action = AlertAction::Notify;
        match cfg.save() {
            Ok(()) => {
                // colour theme takes effect without a restart
                crate::theme::apply(&cfg.general.theme);
                self.exit()
            }
            Err(e) => self.set_status(&format!("save failed: {e}")),
        }
    }
}

impl ButtonEvents for SettingsWindow {
    fn on_pressed(&mut self, handle: Handle<Button>) -> EventProcessStatus {
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
