//! Visual rule editor (PRD FR-3.5, TUI-DESIGN.md §3.4): create/edit a routing
//! rule — role, conditions, ordered fallback chain with up/down, project
//! scope — plus an in-editor dry-run against live provider state.
//!
//! Opened from Mission Control's Rules tab (Rules ▸ New Rule, Enter on a row,
//! or Rules ▸ Edit Rule). Writes through `rolen_core::rules::RuleSet`, so the
//! canonical on-disk format stays `rules.yaml` (decision D2).

use appcui::prelude::*;
use rolen_core::rules::{self, RuleSet, BUILT_IN_ROLES};
use rolen_core::types::{CmpOp, Condition, ConditionField, Rule};
use rolen_providers as providers;

// ------------------------------------------------------------------ helpers

fn op_symbol(op: CmpOp) -> &'static str {
    match op {
        CmpOp::Lt => "<",
        CmpOp::Le => "<=",
        CmpOp::Eq => "==",
        CmpOp::Ne => "!=",
        CmpOp::Ge => ">=",
        CmpOp::Gt => ">",
    }
}

fn field_name(field: ConditionField) -> &'static str {
    match field {
        ConditionField::QuotaRemainingPct => "quota_remaining_pct",
        ConditionField::CostSoFar => "cost_so_far",
        ConditionField::TaskType => "task_type",
        ConditionField::Project => "project",
        ConditionField::TimeOfDay => "time_of_day",
        ConditionField::ProviderHealth => "provider_health",
    }
}

fn fmt_condition(c: &Condition) -> String {
    let prov = c
        .provider
        .as_ref()
        .map(|p| format!(" (provider: {p})"))
        .unwrap_or_default();
    format!(
        "{} {} '{}'{}",
        field_name(c.field),
        op_symbol(c.op),
        c.value,
        prov
    )
}

// ---------------------------------------------------------- chain entry dialog

/// Small picker for one fallback-chain entry: provider combobox + model
/// textfield, with the provider's known models shown as a hint.
#[ModalWindow(events = ButtonEvents+ComboBoxEvents, response = String)]
struct ChainEntryDialog {
    cb_provider: Handle<ComboBox>,
    t_model: Handle<TextField>,
    l_models: Handle<Label>,
    l_status: Handle<Label>,
    b_ok: Handle<Button>,
    b_cancel: Handle<Button>,
    /// (provider id, known models) parallel to the provider combobox items.
    providers: Vec<(String, Vec<String>)>,
}

impl ChainEntryDialog {
    fn new() -> Self {
        let providers: Vec<(String, Vec<String>)> = providers::ProviderRegistry::load()
            .map(|reg| {
                reg.list()
                    .iter()
                    .map(|p| {
                        (
                            p.id.clone(),
                            p.models.iter().map(|m| m.id.clone()).collect::<Vec<_>>(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();

        let mut w = Self {
            base: ModalWindow::new("Chain Entry", layout!("a:c,w:58,h:14"), window::Flags::None),
            cb_provider: Handle::None,
            t_model: Handle::None,
            l_models: Handle::None,
            l_status: Handle::None,
            b_ok: Handle::None,
            b_cancel: Handle::None,
            providers,
        };
        w.add(label!("'&Provider:',x:2,y:1,w:20"));
        let mut cb = ComboBox::new(layout!("x:2,y:2,w:52"), combobox::Flags::None);
        for (id, _) in &w.providers {
            cb.add(id);
        }
        w.cb_provider = w.add(cb);

        w.add(label!("'&Model:',x:2,y:4,w:20"));
        w.t_model = w.add(textfield!("x:2,y:5,w:52"));
        w.l_models = w.add(label!("'',x:2,y:7,w:52"));
        w.l_status = w.add(label!("'',x:2,y:9,w:52"));
        w.b_ok = w.add(button!("'&Add',x:16,y:11,w:12"));
        w.b_cancel = w.add(button!("'&Cancel',x:32,y:11,w:12"));

        w.update_models_hint();
        if w.providers.is_empty() {
            w.set_status("no providers registered — add one first (Providers ▸ Add)");
        }
        w
    }

    fn set_status(&mut self, msg: &str) {
        let h = self.l_status;
        if let Some(l) = self.control_mut(h) {
            l.set_caption(msg);
        }
    }

    fn selected_provider(&self) -> Option<usize> {
        self.control(self.cb_provider)
            .and_then(|cb| cb.index())
            .map(|i| i as usize)
    }

    /// Show the selected provider's known models under the model field.
    fn update_models_hint(&mut self) {
        let text = self
            .selected_provider()
            .and_then(|i| self.providers.get(i))
            .map(|(_, models)| {
                if models.is_empty() {
                    "models: unknown — type the model id".to_string()
                } else {
                    let hint: String = models
                        .iter()
                        .take(6)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!(
                        "models: {hint}{}",
                        if models.len() > 6 { ", …" } else { "" }
                    )
                }
            })
            .unwrap_or_default();
        let h = self.l_models;
        if let Some(l) = self.control_mut(h) {
            l.set_caption(&text);
        }
    }

    fn on_add(&mut self) {
        let Some(i) = self.selected_provider() else {
            self.set_status("pick a provider");
            return;
        };
        let model = self
            .control(self.t_model)
            .map(|t| t.text().trim().to_string())
            .unwrap_or_default();
        if model.is_empty() {
            self.set_status("model is required (e.g. kimi-for-coding)");
            return;
        }
        // note: '/' is legal in model ids (e.g. ollama's hf.co/bartowski/…);
        // the engine splits chain entries only at the first '/'
        self.exit_with(format!("{}/{model}", self.providers[i].0));
    }
}

impl ComboBoxEvents for ChainEntryDialog {
    fn on_selection_changed(&mut self, handle: Handle<ComboBox>) -> EventProcessStatus {
        if handle == self.cb_provider {
            self.update_models_hint();
            return EventProcessStatus::Processed;
        }
        EventProcessStatus::Ignored
    }
}

impl ButtonEvents for ChainEntryDialog {
    fn on_pressed(&mut self, handle: Handle<Button>) -> EventProcessStatus {
        if handle == self.b_ok {
            self.on_add();
            return EventProcessStatus::Processed;
        }
        if handle == self.b_cancel {
            self.exit();
            return EventProcessStatus::Processed;
        }
        EventProcessStatus::Ignored
    }
}

// ------------------------------------------------------------- condition dialog

/// Builder for one rule condition: field / operator / value / provider.
#[ModalWindow(events = ButtonEvents+ComboBoxEvents, response = Condition)]
struct ConditionDialog {
    cb_field: Handle<ComboBox>,
    cb_op: Handle<ComboBox>,
    t_value: Handle<TextField>,
    cb_provider: Handle<ComboBox>,
    l_hint: Handle<Label>,
    l_status: Handle<Label>,
    b_add: Handle<Button>,
    b_cancel: Handle<Button>,
}

const CONDITION_FIELDS: &[(ConditionField, &str)] = &[
    (
        ConditionField::QuotaRemainingPct,
        "remaining quota % of a provider",
    ),
    (ConditionField::CostSoFar, "cost this billing cycle, in $"),
    (ConditionField::TaskType, "task type string (use == / !=)"),
    (ConditionField::Project, "project id (use == / !=)"),
    (ConditionField::TimeOfDay, "hour of day — value as HH:MM"),
    (ConditionField::ProviderHealth, "value: ok / fail"),
];

const CONDITION_OPS: &[(CmpOp, &str)] = &[
    (CmpOp::Lt, "less than"),
    (CmpOp::Le, "less or equal"),
    (CmpOp::Eq, "equal"),
    (CmpOp::Ne, "not equal"),
    (CmpOp::Ge, "greater or equal"),
    (CmpOp::Gt, "greater than"),
];

impl ConditionDialog {
    fn new() -> Self {
        let mut w = Self {
            base: ModalWindow::new("Condition", layout!("a:c,w:66,h:17"), window::Flags::None),
            cb_field: Handle::None,
            cb_op: Handle::None,
            t_value: Handle::None,
            cb_provider: Handle::None,
            l_hint: Handle::None,
            l_status: Handle::None,
            b_add: Handle::None,
            b_cancel: Handle::None,
        };
        w.add(label!("'&Field:',x:2,y:1,w:12"));
        let mut cb_field = ComboBox::new(layout!("x:2,y:2,w:60"), combobox::Flags::ShowDescription);
        for (field, desc) in CONDITION_FIELDS {
            cb_field.add_item(combobox::Item::new(field_name(*field), desc));
        }
        cb_field.set_index(0);
        w.cb_field = w.add(cb_field);

        w.add(label!("'&Operator:',x:2,y:4,w:12"));
        let mut cb_op = ComboBox::new(layout!("x:2,y:5,w:30"), combobox::Flags::ShowDescription);
        for (op, desc) in CONDITION_OPS {
            cb_op.add_item(combobox::Item::new(op_symbol(*op), desc));
        }
        cb_op.set_index(0);
        w.cb_op = w.add(cb_op);

        w.add(label!("'&Value:',x:2,y:7,w:12"));
        w.t_value = w.add(textfield!("x:2,y:8,w:28"));
        w.l_hint = w.add(label!("'',x:32,y:8,w:32"));

        w.add(label!(
            "'&Provider (only for quota / cost / health):',x:2,y:10,w:44"
        ));
        let mut cb_prov = ComboBox::new(layout!("x:2,y:11,w:40"), combobox::Flags::None);
        cb_prov.add_item(combobox::Item::new("", "(none)"));
        if let Ok(reg) = providers::ProviderRegistry::load() {
            for p in reg.list() {
                cb_prov.add(&p.id);
            }
        }
        cb_prov.set_index(0);
        w.cb_provider = w.add(cb_prov);

        w.l_status = w.add(label!("'',x:2,y:13,w:60"));
        w.b_add = w.add(button!("'&Add',x:22,y:15,w:12"));
        w.b_cancel = w.add(button!("'&Cancel',x:38,y:15,w:12"));
        w.update_hint();
        w
    }

    fn combo_value(&self, h: Handle<ComboBox>) -> String {
        self.control(h)
            .and_then(|cb| cb.selected_item().map(|i| i.value().to_string()))
            .unwrap_or_default()
    }

    fn set_status(&mut self, msg: &str) {
        let h = self.l_status;
        if let Some(l) = self.control_mut(h) {
            l.set_caption(msg);
        }
    }

    /// Field-specific value hint next to the value field.
    fn update_hint(&mut self) {
        let hint = match self.combo_value(self.cb_field).as_str() {
            "quota_remaining_pct" => "percent, 0-100",
            "cost_so_far" => "USD, e.g. 12.50",
            "time_of_day" => "HH:MM (24h)",
            "provider_health" => "ok  or  fail",
            "task_type" => "e.g. code, docs",
            "project" => "project id",
            _ => "",
        };
        let h = self.l_hint;
        if let Some(l) = self.control_mut(h) {
            l.set_caption(hint);
        }
    }

    fn on_add(&mut self) {
        let field = CONDITION_FIELDS
            .iter()
            .find(|(f, _)| field_name(*f) == self.combo_value(self.cb_field))
            .map(|(f, _)| *f)
            .unwrap_or(ConditionField::TaskType);
        let op = CONDITION_OPS
            .iter()
            .find(|(o, _)| op_symbol(*o) == self.combo_value(self.cb_op))
            .map(|(o, _)| *o)
            .unwrap_or(CmpOp::Eq);
        let value = self
            .control(self.t_value)
            .map(|t| t.text().trim().to_string())
            .unwrap_or_default();
        if value.is_empty() {
            self.set_status("value is required");
            return;
        }
        // validate the value the way the rule engine will parse it
        let invalid = match field {
            ConditionField::QuotaRemainingPct | ConditionField::CostSoFar => value
                .parse::<f64>()
                .is_err()
                .then_some("value must be a number for this field"),
            ConditionField::TimeOfDay => {
                let ok = value.split_once(':').is_some_and(|(h, m)| {
                    h.parse::<u32>().is_ok_and(|h| h < 24) && m.parse::<u32>().is_ok_and(|m| m < 60)
                });
                (!ok).then_some("value must be HH:MM (24h)")
            }
            _ => None,
        };
        if let Some(msg) = invalid {
            self.set_status(msg);
            return;
        }
        let provider = match self.combo_value(self.cb_provider).as_str() {
            "" => None,
            id => Some(id.to_string()),
        };
        self.exit_with(Condition {
            field,
            op,
            value,
            provider,
        });
    }
}

impl ComboBoxEvents for ConditionDialog {
    fn on_selection_changed(&mut self, handle: Handle<ComboBox>) -> EventProcessStatus {
        if handle == self.cb_field {
            self.update_hint();
            return EventProcessStatus::Processed;
        }
        EventProcessStatus::Ignored
    }
}

impl ButtonEvents for ConditionDialog {
    fn on_pressed(&mut self, handle: Handle<Button>) -> EventProcessStatus {
        if handle == self.b_add {
            self.on_add();
            return EventProcessStatus::Processed;
        }
        if handle == self.b_cancel {
            self.exit();
            return EventProcessStatus::Processed;
        }
        EventProcessStatus::Ignored
    }
}

// ------------------------------------------------------------------ rule editor

/// The rule editor itself. `editing` = the rule being edited (id immutable);
/// `None` = creating a new rule.
#[ModalWindow(events = ButtonEvents, response = bool)]
pub struct RuleEditorDialog {
    t_id: Handle<TextField>,
    cb_role: Handle<ComboBox>,
    t_priority: Handle<TextField>,
    t_min_quota: Handle<TextField>,
    t_scope: Handle<TextField>,
    lb_chain: Handle<ListBox>,
    lb_cond: Handle<ListBox>,
    b_chain_up: Handle<Button>,
    b_chain_down: Handle<Button>,
    b_chain_add: Handle<Button>,
    b_chain_del: Handle<Button>,
    b_cond_add: Handle<Button>,
    b_cond_del: Handle<Button>,
    b_dry: Handle<Button>,
    l_status: Handle<Label>,
    ta_result: Handle<TextArea>,
    b_save: Handle<Button>,
    b_cancel: Handle<Button>,
    editing: Option<Rule>,
    chain: Vec<String>,
    conditions: Vec<Condition>,
}

impl RuleEditorDialog {
    pub fn new(editing: Option<Rule>) -> Self {
        let title = if editing.is_some() {
            "Edit Rule"
        } else {
            "New Rule"
        };
        // h:25 so the dialog fits a standard 80x25 console, like Settings
        let mut w = Self {
            base: ModalWindow::new(title, layout!("a:c,w:78,h:25"), window::Flags::None),
            t_id: Handle::None,
            cb_role: Handle::None,
            t_priority: Handle::None,
            t_min_quota: Handle::None,
            t_scope: Handle::None,
            lb_chain: Handle::None,
            lb_cond: Handle::None,
            b_chain_up: Handle::None,
            b_chain_down: Handle::None,
            b_chain_add: Handle::None,
            b_chain_del: Handle::None,
            b_cond_add: Handle::None,
            b_cond_del: Handle::None,
            b_dry: Handle::None,
            l_status: Handle::None,
            ta_result: Handle::None,
            b_save: Handle::None,
            b_cancel: Handle::None,
            chain: editing
                .as_ref()
                .map(|r| r.fallback_chain.clone())
                .unwrap_or_default(),
            conditions: editing
                .as_ref()
                .map(|r| r.conditions.clone())
                .unwrap_or_default(),
            editing,
        };

        // ---- identity row ----
        w.add(label!("'&Id:',x:2,y:1,w:5"));
        let editing_id = w.editing.as_ref().map(|r| r.id.clone());
        w.t_id = if editing_id.is_some() {
            // the id is the rule's key — renaming would orphan it, so it is
            // read-only while editing
            w.add(textfield!("x:8,y:1,w:22,flags: Readonly"))
        } else {
            w.add(textfield!("x:8,y:1,w:22"))
        };

        w.add(label!("'&Role:',x:34,y:1,w:6"));
        let mut cb_role = ComboBox::new(layout!("x:42,y:1,w:32"), combobox::Flags::None);
        let mut roles: Vec<String> = BUILT_IN_ROLES.iter().map(|s| s.to_string()).collect();
        // roles already used by other rules stay selectable, even custom ones
        if let Ok(rs) = RuleSet::load() {
            for r in &rs.rules {
                if !roles.iter().any(|known| known == &r.role) {
                    roles.push(r.role.clone());
                }
            }
        }
        if let Some(r) = &w.editing {
            if !roles.iter().any(|known| known == &r.role) {
                roles.push(r.role.clone());
            }
        }
        for role in &roles {
            cb_role.add(role);
        }
        let role_idx = w
            .editing
            .as_ref()
            .and_then(|r| roles.iter().position(|known| known == &r.role))
            .unwrap_or(0) as u32;
        cb_role.set_index(role_idx);
        w.cb_role = w.add(cb_role);

        // ---- numeric / scope row ----
        w.add(label!("'&Priority:',x:2,y:3,w:10"));
        w.t_priority = w.add(textfield!("x:13,y:3,w:6"));
        w.add(label!("'&Min quota %:',x:24,y:3,w:14"));
        w.t_min_quota = w.add(textfield!("x:39,y:3,w:5"));
        w.add(label!("'(skip below; empty = only 0%)',x:46,y:3,w:28"));
        w.add(label!(
            "'Pro&ject scope (empty = all projects):',x:2,y:5,w:39"
        ));
        w.t_scope = w.add(textfield!("x:42,y:5,w:24"));

        // ---- fallback chain ----
        w.add(label!(
            "'Fallback chain — first usable provider/model wins:',x:2,y:7,w:52"
        ));
        w.lb_chain = w.add(listbox!(
            "x:2,y:8,w:56,h:4,flags: ScrollBars,em:'empty — add at least one entry'"
        ));
        w.b_chain_up = w.add(button!("'▲ &Up',x:60,y:8,w:14"));
        w.b_chain_down = w.add(button!("'▼ &Down',x:60,y:9,w:14"));
        w.b_chain_add = w.add(button!("'+ &Add entry',x:60,y:10,w:14"));
        w.b_chain_del = w.add(button!("'− Remo&ve',x:60,y:11,w:14"));

        // ---- conditions ----
        w.add(label!("'Conditions — all must hold:',x:2,y:13,w:52"));
        w.lb_cond = w.add(listbox!(
            "x:2,y:14,w:56,h:3,flags: ScrollBars,em:'no conditions — rule always applies'"
        ));
        w.b_cond_add = w.add(button!("'+ Add &cond.',x:60,y:14,w:14"));
        w.b_cond_del = w.add(button!("'− Re&move',x:60,y:15,w:14"));

        // ---- dry-run ----
        w.b_dry = w.add(button!("'Dry-run ▶',x:2,y:17,w:14"));
        w.l_status = w.add(label!("'',x:18,y:17,w:56"));
        w.ta_result = w.add(textarea!(
            "'',x:2,y:18,w:72,h:3,flags: [ReadOnly, ScrollBars]"
        ));

        w.b_save = w.add(button!("'&Save',x:26,y:22,w:12"));
        w.b_cancel = w.add(button!("'&Cancel',x:42,y:22,w:12"));

        // ---- fill from the rule being edited ----
        if let Some(rule) = w.editing.clone() {
            let priority = rule.priority.to_string();
            let min_quota = rule
                .min_quota_pct
                .map(|q| q.to_string())
                .unwrap_or_default();
            let scope = rule.project_scope.clone().unwrap_or_default();
            let h = w.t_id;
            if let Some(t) = w.control_mut(h) {
                t.set_text(&rule.id);
            }
            let h = w.t_priority;
            if let Some(t) = w.control_mut(h) {
                t.set_text(&priority);
            }
            let h = w.t_min_quota;
            if let Some(t) = w.control_mut(h) {
                t.set_text(&min_quota);
            }
            let h = w.t_scope;
            if let Some(t) = w.control_mut(h) {
                t.set_text(&scope);
            }
        } else {
            let h = w.t_priority;
            if let Some(t) = w.control_mut(h) {
                t.set_text("0");
            }
        }
        w.refresh_chain_list(None);
        w.refresh_cond_list(None);
        w
    }

    // ------------------------------------------------------------- helpers

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

    /// Rebuild the chain listbox from `self.chain`, restoring a selection.
    fn refresh_chain_list(&mut self, select: Option<usize>) {
        // snapshot: the control borrow and the data borrow would overlap
        let chain = self.chain.clone();
        let h = self.lb_chain;
        if let Some(lb) = self.control_mut(h) {
            lb.clear();
            for (i, entry) in chain.iter().enumerate() {
                lb.add(&format!("{}. {entry}", i + 1));
            }
            if !chain.is_empty() {
                lb.set_index(select.unwrap_or(0).min(chain.len() - 1));
            }
        }
    }

    fn refresh_cond_list(&mut self, select: Option<usize>) {
        let lines: Vec<String> = self.conditions.iter().map(fmt_condition).collect();
        let h = self.lb_cond;
        if let Some(lb) = self.control_mut(h) {
            lb.clear();
            for line in &lines {
                lb.add(line);
            }
            if !lines.is_empty() {
                lb.set_index(select.unwrap_or(0).min(lines.len() - 1));
            }
        }
    }

    fn chain_index(&self) -> Option<usize> {
        let i = self.control(self.lb_chain)?.index();
        (i != usize::MAX && i < self.chain.len()).then_some(i)
    }

    fn cond_index(&self) -> Option<usize> {
        let i = self.control(self.lb_cond)?.index();
        (i != usize::MAX && i < self.conditions.len()).then_some(i)
    }

    /// Validate the form and build the rule it describes.
    fn build_rule(&self) -> Result<Rule, String> {
        let id = match &self.editing {
            Some(r) => r.id.clone(),
            None => {
                let id = self.field(self.t_id);
                if id.is_empty() {
                    return Err("id is required".into());
                }
                if id.contains(char::is_whitespace) {
                    return Err("id must not contain whitespace".into());
                }
                id
            }
        };
        let role = self
            .control(self.cb_role)
            .and_then(|cb| cb.selected_item().map(|i| i.value().to_string()))
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "pick a role".to_string())?;
        let priority = match self.field(self.t_priority).as_str() {
            "" => 0,
            s => s
                .parse::<i32>()
                .map_err(|_| "priority must be an integer".to_string())?,
        };
        let min_quota_pct = match self.field(self.t_min_quota).as_str() {
            "" => None,
            s => {
                let v: u8 = s
                    .parse()
                    .map_err(|_| "min quota % must be a number 0-100".to_string())?;
                if v > 100 {
                    return Err("min quota % must be 0-100".into());
                }
                Some(v)
            }
        };
        if self.chain.is_empty() {
            return Err("fallback chain is empty — add at least one provider/model".into());
        }
        let project_scope = match self.field(self.t_scope).as_str() {
            "" => None,
            s => Some(s.to_string()),
        };
        Ok(Rule {
            id,
            role,
            conditions: self.conditions.clone(),
            fallback_chain: self.chain.clone(),
            min_quota_pct,
            priority,
            project_scope,
        })
    }

    // -------------------------------------------------------------- actions

    fn chain_add(&mut self) {
        if let Some(entry) = ChainEntryDialog::new().show() {
            self.chain.push(entry);
            let last = self.chain.len() - 1;
            self.refresh_chain_list(Some(last));
        }
    }

    fn chain_move(&mut self, delta: isize) {
        let Some(i) = self.chain_index() else {
            self.set_status("select a chain entry first");
            return;
        };
        let j = i as isize + delta;
        if j < 0 || j >= self.chain.len() as isize {
            return;
        }
        self.chain.swap(i, j as usize);
        self.refresh_chain_list(Some(j as usize));
    }

    fn chain_del(&mut self) {
        let Some(i) = self.chain_index() else {
            self.set_status("select a chain entry first");
            return;
        };
        self.chain.remove(i);
        self.refresh_chain_list(Some(i));
    }

    fn cond_add(&mut self) {
        if let Some(c) = ConditionDialog::new().show() {
            self.conditions.push(c);
            let last = self.conditions.len() - 1;
            self.refresh_cond_list(Some(last));
        }
    }

    fn cond_del(&mut self) {
        let Some(i) = self.cond_index() else {
            self.set_status("select a condition first");
            return;
        };
        self.conditions.remove(i);
        self.refresh_cond_list(Some(i));
    }

    /// FR-3.4 dry-run, evaluated against live provider state with the form's
    /// current (possibly unsaved) values.
    fn dry_run(&mut self) {
        let rule = match self.build_rule() {
            Ok(r) => r,
            Err(e) => {
                self.set_status(&e);
                return;
            }
        };
        let text = match providers::routing::collect(None, None) {
            Ok(ctx) => {
                let rules = RuleSet { rules: vec![rule] };
                match rules::decide(&rules, &rules.rules[0].role, &ctx) {
                    Ok(d) => {
                        let mut s = format!(
                            "decision:  {} / {}\nexplain:   {}\n",
                            d.provider, d.model, d.explanation
                        );
                        if !d.skipped.is_empty() {
                            s.push_str("skipped:\n");
                            for (e, why) in &d.skipped {
                                s.push_str(&format!("  {e}  ({why})\n"));
                            }
                        }
                        self.set_status("dry-run: routed ✓");
                        s
                    }
                    Err(e) => {
                        self.set_status("dry-run: NO ROUTE");
                        format!("NO ROUTE:\n{e}")
                    }
                }
            }
            Err(e) => format!("failed to collect provider state: {e}"),
        };
        let h = self.ta_result;
        if let Some(t) = self.control_mut(h) {
            t.set_text(&text);
        }
    }

    fn on_save(&mut self) {
        let rule = match self.build_rule() {
            Ok(r) => r,
            Err(e) => {
                self.set_status(&e);
                return;
            }
        };
        let mut rules = match RuleSet::load() {
            Ok(r) => r,
            Err(e) => {
                self.set_status(&format!("failed to load rules.yaml: {e}"));
                return;
            }
        };
        match &self.editing {
            Some(orig) => {
                if let Some(slot) = rules.rules.iter_mut().find(|r| r.id == orig.id) {
                    *slot = rule;
                } else {
                    // rule vanished from disk while editing — re-add it
                    rules.rules.push(rule);
                }
            }
            None => {
                if rules.rules.iter().any(|r| r.id == rule.id) {
                    self.set_status(&format!("rule id '{}' already exists", rule.id));
                    return;
                }
                rules.rules.push(rule);
            }
        }
        match rules.save() {
            Ok(()) => self.exit_with(true),
            Err(e) => self.set_status(&format!("failed to save rules.yaml: {e}")),
        }
    }
}

impl ButtonEvents for RuleEditorDialog {
    fn on_pressed(&mut self, handle: Handle<Button>) -> EventProcessStatus {
        if handle == self.b_chain_add {
            self.chain_add();
        } else if handle == self.b_chain_del {
            self.chain_del();
        } else if handle == self.b_chain_up {
            self.chain_move(-1);
        } else if handle == self.b_chain_down {
            self.chain_move(1);
        } else if handle == self.b_cond_add {
            self.cond_add();
        } else if handle == self.b_cond_del {
            self.cond_del();
        } else if handle == self.b_dry {
            self.dry_run();
        } else if handle == self.b_save {
            self.on_save();
        } else if handle == self.b_cancel {
            self.exit();
        } else {
            return EventProcessStatus::Ignored;
        }
        EventProcessStatus::Processed
    }
}

// ------------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;

    /// Opens the editor modally as soon as the app is up.
    #[Window(events = WindowEvents)]
    struct Launcher {
        edit: bool,
    }

    impl Launcher {
        fn new(edit: bool) -> Self {
            Self {
                base: window!("title:'L',d:f"),
                edit,
            }
        }
    }

    impl WindowEvents for Launcher {
        fn on_activate(&mut self) {
            if self.edit {
                RuleEditorDialog::new(Some(Rule {
                    id: "coder".into(),
                    role: "coder".into(),
                    conditions: vec![Condition {
                        field: ConditionField::QuotaRemainingPct,
                        op: CmpOp::Lt,
                        value: "20".into(),
                        provider: Some("kimi".into()),
                    }],
                    fallback_chain: vec![
                        "kimi/kimi-for-coding".into(),
                        "ollama-local/qwen2.5-coder:7b".into(),
                    ],
                    min_quota_pct: Some(20),
                    priority: 5,
                    project_scope: None,
                }))
                .show();
            } else {
                RuleEditorDialog::new(None).show();
            }
            self.close();
        }
    }

    /// Smoke test: both editor modes build a valid layout, open and close on
    /// Escape without panicking.
    #[test]
    fn editor_opens_and_closes() {
        let script = "
            Paint('new-rule editor open')
            Key.Pressed(Escape)
            Paint('new-rule editor closed')
        ";
        let mut app = App::debug(90, 28, script).build().unwrap();
        app.add_window(Launcher::new(false));
        app.run();

        let script = "
            Paint('edit-rule editor open')
            Key.Pressed(Escape)
            Paint('edit-rule editor closed')
        ";
        let mut app = App::debug(90, 28, script).build().unwrap();
        app.add_window(Launcher::new(true));
        app.run();
    }

    #[test]
    fn condition_formatting_matches_yaml_names() {
        let c = Condition {
            field: ConditionField::TimeOfDay,
            op: CmpOp::Ge,
            value: "09:00".into(),
            provider: None,
        };
        assert_eq!(fmt_condition(&c), "time_of_day >= '09:00'");
        let c = Condition {
            field: ConditionField::ProviderHealth,
            op: CmpOp::Ne,
            value: "ok".into(),
            provider: Some("kimi".into()),
        };
        assert_eq!(
            fmt_condition(&c),
            "provider_health != 'ok' (provider: kimi)"
        );
    }
}
