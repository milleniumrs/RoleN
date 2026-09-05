//! Mission Control — the main window (TUI-DESIGN.md §3).
//!
//! M1 scope: live Providers tab (registry + per-provider tokens today),
//! dashboard summary, health checks, Add-Provider wizard, and a 3-second
//! timer that refreshes the app-bar token/cost label from the ledger.
//! Sessions stay at 0 until the runtime lands in M2.

use appcui::prelude::*;
use appcui::ui::appbar::{MenuButton, Side};
use rolen_core::ledger::Ledger;
use rolen_providers as providers;
use std::collections::HashMap;
use std::time::Duration;

use crate::add_provider::AddProviderDialog;
use crate::rule_editor::RuleEditorDialog;
use crate::{model_prices, provider_detail, quick_chat, settings, theme, transcript_view};

#[derive(ListItem)]
struct ProviderRow {
    #[Column(name: "&Name", width: 16)]
    name: String,
    #[Column(name: "&Type", width: 13)]
    ptype: String,
    #[Column(name: "&Status", width: 14)]
    status: String,
    #[Column(name: "Models", width: 7, align: right)]
    models: String,
    #[Column(name: "&Quota", width: 8, align: right)]
    quota: String,
    #[Column(name: "Tok today", width: 10, align: right)]
    tokens: String,
    #[Column(name: "Burn/day", width: 9, align: right)]
    burn: String,
    #[Column(name: "Empty in", width: 9, align: right)]
    eta: String,
}

#[derive(ListItem)]
struct RuleRow {
    #[Column(name: "&Id", width: 20)]
    id: String,
    #[Column(name: "&Role", width: 14)]
    role: String,
    #[Column(name: "Prio", width: 6, align: right)]
    priority: String,
    #[Column(name: "&Fallback chain", width: 52)]
    chain: String,
}

#[derive(ListItem)]
struct SessionRow {
    #[Column(name: "&Session", width: 22)]
    id: String,
    #[Column(name: "&Role", width: 12)]
    role: String,
    #[Column(name: "&Provider", width: 14)]
    provider: String,
    #[Column(name: "&Model", width: 20)]
    model: String,
    #[Column(name: "&State", width: 11)]
    state: String,
    #[Column(name: "Tokens", width: 9, align: right)]
    tokens: String,
    #[Column(name: "Rate t/m", width: 8, align: right)]
    rate: String,
    #[Column(name: "Elapsed", width: 8, align: right)]
    elapsed: String,
    #[Column(name: "Cost $", width: 8, align: right)]
    cost: String,
}

#[derive(ListItem)]
struct QuestionRow {
    #[Column(name: "&Project", width: 18)]
    project: String,
    #[Column(name: "&Question", width: 56)]
    question: String,
    #[Column(name: "&Status", width: 10)]
    status: String,
}

/// Projects tree node (FR-10.4): project → tasks (tasks.yaml) → sessions.
struct ProjectNode {
    label: String,
    kind: ProjectNodeKind,
}

#[derive(Clone)]
enum ProjectNodeKind {
    /// Index into MissionControl::project_dirs
    Project(usize),
    /// Informational task node
    Task,
    /// Index into MissionControl::project_session_transcripts
    Session(usize),
}

impl ListItem for ProjectNode {
    fn render_method(&'_ self, column_index: u16) -> Option<RenderMethod<'_>> {
        match column_index {
            0 => Some(RenderMethod::Text(self.label.as_str())),
            _ => None,
        }
    }
    fn columns_count() -> u16 {
        1
    }
    fn column(_: u16) -> Column {
        Column::new("Project / Task / Session", 90, TextAlignment::Left)
    }
}

/// Index of the Activity tab in the tab control built by [`MissionControl::new`].
const ACTIVITY_TAB: usize = 5;

/// Payload delivered by the background CLI-task worker.
pub enum CliTaskMsg {
    /// A chunk of live PTY output, roughly every 80 ms while the agent works.
    Output(String),
    /// The agent's edits reached the write queue.
    Harvested {
        applied: usize,
        rejected: usize,
        paths: usize,
    },
    Finished(String),
}

/// Job slot for the background CLI-task worker (BackgroundTask::run needs a
/// plain fn — parameters travel through this static).
static CLI_JOB: std::sync::Mutex<Option<(String, String, std::path::PathBuf)>> =
    std::sync::Mutex::new(None);

fn cli_task_worker(conector: &BackgroundTaskConector<CliTaskMsg, bool>) {
    let job = CLI_JOB.lock().unwrap().take();
    let Some((provider_id, task, workdir)) = job else {
        conector.notify(CliTaskMsg::Finished("no job".into()));
        return;
    };
    let result = (|| -> anyhow::Result<String> {
        let reg = providers::ProviderRegistry::load()?;
        let p = reg
            .get(&provider_id)
            .ok_or_else(|| anyhow::anyhow!("provider '{provider_id}' not found"))?
            .clone();
        // The adapter streams stdout as it arrives; forwarding it to the
        // window is what makes a long agent run watchable instead of a frozen
        // "started in the background" message followed by silence.
        let mut on_event = |event: rolen_cliadapters::CliEvent| match event {
            rolen_cliadapters::CliEvent::Output(chunk) => {
                conector.notify(CliTaskMsg::Output(chunk));
            }
            rolen_cliadapters::CliEvent::Harvested {
                applied,
                rejected,
                paths,
            } => {
                conector.notify(CliTaskMsg::Harvested {
                    applied,
                    rejected,
                    paths: paths.len(),
                });
            }
        };
        let report = rolen_cliadapters::run_cli_session(&p, &task, &workdir, None, &mut on_event)?;
        Ok(format!(
            "session {} — exit {:?}\nwrites via queue: {} applied / {} rejected\ntranscript: {}",
            report.session_id,
            report.exit_code,
            report.applied,
            report.rejected,
            report.transcript_path.display()
        ))
    })();
    let msg = match result {
        Ok(s) => s,
        Err(e) => format!("CLI task failed: {e}"),
    };
    conector.notify(CliTaskMsg::Finished(msg));
}

#[Window(events: MenuEvents+AppBarEvents+WindowEvents+TimerEvents+ListViewEvents<ProviderRow>+ListViewEvents<RuleRow>+ListViewEvents<SessionRow>+ListViewEvents<QuestionRow>+TreeViewEvents<ProjectNode>+BackgroundTaskEvents<CliTaskMsg,bool>, commands: NewProject+Interview+RunProject+PauseProject+BuildProject+AddProvider+DetectClis+HealthCheck+ModelPrices+NewRule+EditRule+DeleteRule+DryRun+QuickChat+RunCliTask+PauseAll+Settings+ThemeDefault+ThemeDarkGray+ThemeLight+ThemeDark+ThemeHacker+ThemeFancy+ThemeRainbow+ThemeOcean+ThemeAmber+ThemePaper+ThemeSky+ThemeMint+ThemeSand+Doctor+About+Exit)]
pub struct MissionControl {
    // menus (app bar, left side)
    m_file: Handle<MenuButton>,
    m_project: Handle<MenuButton>,
    m_providers: Handle<MenuButton>,
    m_rules: Handle<MenuButton>,
    m_sessions: Handle<MenuButton>,
    m_tools: Handle<MenuButton>,
    m_help: Handle<MenuButton>,
    // status labels (app bar, right side)
    st_sessions: Handle<appbar::Label>,
    st_tokens: Handle<appbar::Label>,
    st_queue: Handle<appbar::Label>,
    st_questions: Handle<appbar::Label>,
    // dashboard tab
    d_providers: Handle<Label>,
    d_today: Handle<Label>,
    d_tickets: Handle<Label>,
    // providers tab
    lv_providers: Handle<ListView<ProviderRow>>,
    // rules tab
    lv_rules: Handle<ListView<RuleRow>>,
    // rule ids parallel to the rules listview rows
    rule_ids: Vec<String>,
    // projects tab (FR-10.4 tree: project → tasks → sessions)
    tv_projects: Handle<TreeView<ProjectNode>>,
    // project dirs parallel to the tree's root items
    project_dirs: Vec<std::path::PathBuf>,
    // transcript paths parallel to session nodes in the projects tree
    project_session_transcripts: Vec<Option<std::path::PathBuf>>,
    // dashboard: recent sessions
    lv_sessions: Handle<ListView<SessionRow>>,
    session_transcripts: Vec<Option<std::path::PathBuf>>,
    // questions tab (interrogation center, FR-10.6)
    lv_questions: Handle<ListView<QuestionRow>>,
    question_refs: Vec<(std::path::PathBuf, String)>, // (project dir, clarification id)
    // provider ids parallel to the providers listview rows
    provider_ids: Vec<String>,
    // providers already alerted at critical level (one-shot, FR-4.5)
    alerted: std::collections::HashSet<String>,
    // the tab control itself, so a starting CLI task can bring its own tab up
    tabs: Handle<Tab>,
    // activity tab: live output from the running CLI agent
    ta_activity: Handle<TextArea>,
    /// Live agent output, ANSI already stripped and capped.
    cli_output: String,
    // events/commands
    bt_cli: Handle<BackgroundTask<CliTaskMsg, bool>>,
    // last health-check results, by provider id
    health: HashMap<String, String>,
}

impl MissionControl {
    pub fn new() -> Self {
        let mut w = Self {
            base: window!("title:'RoleN — Mission Control',d:f"),
            m_file: Handle::None,
            m_project: Handle::None,
            m_providers: Handle::None,
            m_rules: Handle::None,
            m_sessions: Handle::None,
            m_tools: Handle::None,
            m_help: Handle::None,
            st_sessions: Handle::None,
            st_tokens: Handle::None,
            st_queue: Handle::None,
            st_questions: Handle::None,
            d_providers: Handle::None,
            d_today: Handle::None,
            d_tickets: Handle::None,
            lv_providers: Handle::None,
            lv_rules: Handle::None,
            rule_ids: Vec::new(),
            tv_projects: Handle::None,
            project_dirs: Vec::new(),
            project_session_transcripts: Vec::new(),
            lv_sessions: Handle::None,
            session_transcripts: Vec::new(),
            lv_questions: Handle::None,
            question_refs: Vec::new(),
            provider_ids: Vec::new(),
            alerted: std::collections::HashSet::new(),
            tabs: Handle::None,
            ta_activity: Handle::None,
            cli_output: String::new(),
            bt_cli: Handle::None,
            health: HashMap::new(),
        };

        // ---- menus (TUI-DESIGN.md §2 menu map) ----
        w.m_file = w.appbar().add(MenuButton::new(
            "&File",
            menu!(
                "class: MissionControl, items=[
                {'&New Project', Ctrl+N, cmd:NewProject},
                {-},
                {'Config &Doctor', F9, cmd:Doctor},
                {-},
                {'E&xit', Alt+F4, cmd:Exit}
            ]"
            ),
            0,
            Side::Left,
        ));
        w.m_project = w.appbar().add(MenuButton::new(
            "&Project",
            menu!(
                "class: MissionControl, items=[
                {'&New Project', Ctrl+Shift+N, cmd:NewProject},
                {'&Interview', cmd:Interview},
                {'&Build (PRD/AGENTS/skills/DAG)', cmd:BuildProject},
                {-},
                {'&Run', F5, cmd:RunProject},
                {'&Pause', Shift+F5, cmd:PauseProject}
            ]"
            ),
            1,
            Side::Left,
        ));
        w.m_providers = w.appbar().add(MenuButton::new(
            "P&roviders",
            menu!(
                "class: MissionControl, items=[
                {'&Add Provider', cmd:AddProvider},
                {'&Detect CLIs && Ollama', cmd:DetectClis},
                {'&Health Check All', cmd:HealthCheck},
                {-},
                {'Model &Prices', cmd:ModelPrices}
            ]"
            ),
            2,
            Side::Left,
        ));
        w.m_rules = w.appbar().add(MenuButton::new(
            "&Rules",
            menu!(
                "class: MissionControl, items=[
                {'&New Rule', cmd:NewRule},
                {'&Edit Rule', cmd:EditRule},
                {'De&lete Rule', cmd:DeleteRule},
                {-},
                {'&Dry-Run', Ctrl+D, cmd:DryRun}
            ]"
            ),
            3,
            Side::Left,
        ));
        w.m_sessions = w.appbar().add(MenuButton::new(
            "&Sessions",
            menu!(
                "class: MissionControl, items=[
                {'&Quick Chat', Ctrl+Q, cmd:QuickChat},
                {'Run &CLI Task (PTY-wrapped agent)', cmd:RunCliTask},
                {'Pause &All', cmd:PauseAll}
            ]"
            ),
            4,
            Side::Left,
        ));
        w.m_tools = w.appbar().add(MenuButton::new(
            "&Tools",
            menu!(
                "class: MissionControl, items=[
                {'&Settings', F10, cmd:Settings},
                {'&Theme', items=[
                    {'&Dark', items=[
                        {'&Default', cmd:ThemeDefault},
                        {'Dark &Gray', cmd:ThemeDarkGray},
                        {'&Black (white on black)', cmd:ThemeDark},
                        {'&Hacker (green phosphor)', cmd:ThemeHacker},
                        {'&Ocean', cmd:ThemeOcean},
                        {'&Amber (retro CRT)', cmd:ThemeAmber},
                        {'&Rainbow', cmd:ThemeRainbow}
                    ]},
                    {'&Light', items=[
                        {'&Light', cmd:ThemeLight},
                        {'&Paper (white, dark ink)', cmd:ThemePaper},
                        {'&Fancy (pink)', cmd:ThemeFancy},
                        {'&Sky (pale cyan)', cmd:ThemeSky},
                        {'&Mint (pale green)', cmd:ThemeMint},
                        {'S&and (warm)', cmd:ThemeSand}
                    ]}
                ]},
                {-},
                {'Config &Doctor', F9, cmd:Doctor}
            ]"
            ),
            5,
            Side::Left,
        ));
        w.m_help = w.appbar().add(MenuButton::new(
            "&Help",
            menu!(
                "class: MissionControl, items=[
                {'&About', F1, cmd:About}
            ]"
            ),
            6,
            Side::Left,
        ));

        // ---- status labels (app bar, right side) ----
        w.st_sessions = w
            .appbar()
            .add(appbar::Label::new("● 0 sessions", 0, Side::Right));
        w.st_tokens = w
            .appbar()
            .add(appbar::Label::new("0 tok · $0.00 today", 1, Side::Right));
        w.st_queue = w
            .appbar()
            .add(appbar::Label::new("queue: 0", 2, Side::Right));
        w.st_questions = w.appbar().add(appbar::Label::new("❓ 0", 3, Side::Right));

        // ---- the six fixed tabs (TUI-DESIGN.md §3) ----
        // TransparentBackground: the tab body inherits the window colour, so
        // the active theme's palette shows through instead of the tab
        // control's own (theme-private, always grey) surface
        let mut t =
            tab!("tabs:[Dashboard,Projects,Providers,Rules,Questions,Activity],l:0,t:0,r:0,b:0,flags: TransparentBackground");

        // Dashboard
        w.d_providers = t.add(0, label!("'Providers: loading…',x:2,y:0,w:76,h:1"));
        w.d_today = t.add(0, label!("'Today: —',x:2,y:1,w:76,h:1"));
        w.d_tickets = t.add(0, label!("'Write tickets: —',x:2,y:2,w:76,h:1"));
        t.add(0, label!("'Recent sessions:',x:2,y:4,w:40,h:1"));
        w.lv_sessions = t.add(
            0,
            listview!("class: SessionRow,l:1,t:5,r:1,b:1,flags: [ScrollBars]"),
        );

        // Projects (FR-10.4): tree project → tasks → sessions
        w.tv_projects = t.add(
            1,
            TreeView::<ProjectNode>::new(layout!("l:0,t:0,r:0,b:1"), treeview::Flags::ScrollBars),
        );
        t.add(1, label!("'Enter on a project = detail (PRD/AGENTS.md/skills) · Enter on a session = transcript · Project menu: New / Interview / Build',l:1,b:0,r:1"));

        // Providers
        w.lv_providers = t.add(
            2,
            listview!("class: ProviderRow,l:0,t:0,r:0,b:1,flags: [ScrollBars, SearchBar]"),
        );
        t.add(
            2,
            label!(
                "'Providers menu: Add / Detect / Health Check — CLI: rolen provider …',l:1,b:0,r:1"
            ),
        );

        // Rules
        w.lv_rules = t.add(
            3,
            listview!("class: RuleRow,l:0,t:0,r:0,b:1,flags: [ScrollBars, SearchBar]"),
        );
        t.add(
            3,
            label!(
                "'Enter = edit · Rules menu: New / Edit / Delete / Dry-Run (Ctrl+D)',l:1,b:0,r:1"
            ),
        );

        // Questions (interrogation center, FR-10.6): pending clarifications
        // across all projects; Enter/Space answers the selected one.
        w.lv_questions = t.add(
            4,
            listview!("class: QuestionRow,l:0,t:0,r:0,b:1,flags: [ScrollBars, SearchBar]"),
        );
        t.add(4, label!("'Enter on a question to answer it — blocked tasks resume automatically',l:1,b:0,r:1"));

        // Activity: live output from a wrapped CLI agent while it works.
        t.add(
            5,
            label!("'Live agent output — Sessions ▸ Run CLI Task starts one.',l:1,t:0,r:1,h:1"),
        );
        w.ta_activity = t.add(
            5,
            textarea!("'',l:1,t:1,r:1,b:0,flags: [ReadOnly, ScrollBars]"),
        );

        w.tabs = w.add(t);
        w
    }

    // ------------------------------------------------------------- helpers

    fn not_yet(&self, what: &str) {
        dialogs::message(
            "RoleN",
            &format!("'{what}' arrives in a later milestone — see PRD.md §9 roadmap."),
        );
    }

    fn run_doctor(&self) {
        let checks = rolen_core::doctor::run_all();
        let mut text = String::new();
        for c in &checks {
            text.push_str(if c.ok { "[ OK ] " } else { "[FAIL] " });
            text.push_str(c.name);
            text.push_str(" — ");
            text.push_str(&c.detail);
            text.push('\n');
        }
        if rolen_core::doctor::all_ok(&checks) {
            dialogs::message("Config Doctor", &text);
        } else {
            dialogs::error("Config Doctor", &text);
        }
    }

    /// Rebuild the providers table + dashboard summary from registry/ledger.
    fn refresh_providers(&mut self) {
        let reg = providers::ProviderRegistry::load().unwrap_or_default();
        let ledger = Ledger::open_default().ok();

        let mut rows = Vec::new();
        let mut ids = Vec::new();
        let mut total_models = 0usize;
        for p in reg.list() {
            total_models += p.models.len();
            ids.push(p.id.clone());
            let tokens = ledger
                .as_ref()
                .and_then(|l| l.usage_today(Some(&p.id)).ok())
                .map(|u| fmt_tokens(u.total_tokens()))
                .unwrap_or_else(|| "-".into());
            let quota = providers::routing::remaining_pct(&p.id)
                .map(|pct| format!("{pct}%"))
                .unwrap_or_else(|| "—".into());
            // FR-9.2: burn rate + exhaustion forecast for budgeted providers
            let (burn, eta) = providers::routing::burn_rate(&p.id)
                .map(|(per_day, days_left)| {
                    (
                        fmt_tokens(per_day),
                        if days_left >= 30.0 {
                            "30d+".into()
                        } else if days_left >= 1.0 {
                            format!("{days_left:.0}d")
                        } else {
                            format!("{:.0}h", days_left * 24.0)
                        },
                    )
                })
                .unwrap_or_else(|| ("—".into(), "—".into()));
            rows.push(ProviderRow {
                name: p.id.clone(),
                ptype: format!("{:?}", p.ptype),
                status: if p.suspended {
                    "SUSPENDED".into()
                } else {
                    self.health.get(&p.id).cloned().unwrap_or_else(|| {
                        if p.ptype == rolen_core::types::ProviderType::Cli {
                            "cli".into()
                        } else {
                            "not checked".into()
                        }
                    })
                },
                models: p.models.len().to_string(),
                quota,
                tokens,
                burn,
                eta,
            });
        }
        self.provider_ids = ids;
        let provider_count = rows.len();

        let lv = self.lv_providers;
        if let Some(lv) = self.control_mut(lv) {
            lv.clear();
            for row in rows {
                lv.add(row);
            }
        }

        let h = self.d_providers;
        if let Some(l) = self.control_mut(h) {
            l.set_caption(&format!(
                "Providers: {provider_count} registered · {total_models} models discovered"
            ));
        }
        self.refresh_today();
    }

    /// Rebuild the rules table from rules.yaml.
    fn refresh_rules(&mut self) {
        let rules = rolen_core::rules::RuleSet::load().unwrap_or_default();
        self.rule_ids = rules.rules.iter().map(|r| r.id.clone()).collect();
        let rows: Vec<RuleRow> = rules
            .rules
            .iter()
            .map(|r| RuleRow {
                id: r.id.clone(),
                role: r.role.clone(),
                priority: r.priority.to_string(),
                chain: r.fallback_chain.join(" → "),
            })
            .collect();
        let lv = self.lv_rules;
        if let Some(lv) = self.control_mut(lv) {
            lv.clear();
            for row in rows {
                lv.add(row);
            }
        }
    }

    /// Visual rule editor (FR-3.5): `None` creates, `Some(id)` edits in place.
    fn edit_rule_dialog(&mut self, rule_id: Option<String>) {
        let editing = rule_id.and_then(|id| {
            rolen_core::rules::RuleSet::load()
                .ok()
                .and_then(|rs| rs.rules.into_iter().find(|r| r.id == id))
        });
        if RuleEditorDialog::new(editing).show() == Some(true) {
            self.refresh_rules();
        }
    }

    /// Id of the rule selected in the Rules tab, if any.
    fn selected_rule_id(&self) -> Option<String> {
        let lv = self.lv_rules;
        self.control(lv)
            .and_then(|lv| lv.current_item_index())
            .and_then(|i| self.rule_ids.get(i).cloned())
    }

    fn delete_rule(&mut self) {
        let Some(id) = self.selected_rule_id() else {
            dialogs::message("Delete Rule", "Select a rule in the Rules tab first.");
            return;
        };
        if !dialogs::validate("Delete Rule", &format!("Delete rule '{id}'?")) {
            return;
        }
        match rolen_core::rules::RuleSet::load() {
            Ok(mut rules) => {
                rules.rules.retain(|r| r.id != id);
                match rules.save() {
                    Ok(()) => self.refresh_rules(),
                    Err(e) => dialogs::error("Delete Rule", &format!("failed to save: {e}")),
                }
            }
            Err(e) => dialogs::error("Delete Rule", &format!("failed to load rules: {e}")),
        }
    }

    /// FR-3.4 dry-run: evaluate a role against live state and explain.
    fn dry_run_rule(&mut self) {
        let default_role = self
            .selected_rule_id()
            .and_then(|id| {
                rolen_core::rules::RuleSet::load()
                    .ok()
                    .and_then(|rs| rs.rules.into_iter().find(|r| r.id == id))
            })
            .map(|r| r.role)
            .unwrap_or_else(|| "coder".into());
        let Some(role) =
            dialogs::input::<String>("Dry-Run", "Role to evaluate:", Some(default_role), None)
        else {
            return;
        };
        let rules = rolen_core::rules::RuleSet::load().unwrap_or_default();
        let text = match providers::routing::collect(None, None) {
            Ok(ctx) => match rolen_core::rules::decide(&rules, role.trim(), &ctx) {
                Ok(d) => {
                    let mut s = format!(
                        "decision:  {} / {}\nrule:      {}\n",
                        d.provider, d.model, d.rule_id
                    );
                    if !d.skipped.is_empty() {
                        s.push_str("skipped:\n");
                        for (e, why) in &d.skipped {
                            s.push_str(&format!("  {e}  ({why})\n"));
                        }
                    }
                    s
                }
                Err(e) => format!("NO ROUTE:\n{e}"),
            },
            Err(e) => format!("failed to collect provider state: {e}"),
        };
        dialogs::message("Dry-Run", &text);
    }

    /// Update the "today" dashboard line + app-bar token label from the ledger.
    fn refresh_today(&mut self) {
        let today = Ledger::open_default()
            .ok()
            .and_then(|l| l.usage_today(None).ok());
        let (tin, tout, cost, reqs) = today
            .map(|u| (u.tokens_in, u.tokens_out, u.cost, u.requests))
            .unwrap_or((0, 0, 0.0, 0));

        let h = self.d_today;
        if let Some(l) = self.control_mut(h) {
            l.set_caption(&format!(
                "Today: {} in / {} out · ${:.4} · {} requests",
                fmt_tokens(tin),
                fmt_tokens(tout),
                cost,
                reqs
            ));
        }
        let h = self.st_tokens;
        if let Some(l) = self.appbar().get_mut(h) {
            l.set_caption(&format!(
                "{} tok · ${:.2} today",
                fmt_tokens(tin + tout),
                cost
            ));
        }
        let running = Ledger::open_default()
            .ok()
            .and_then(|l| l.count_sessions_by_state("running").ok())
            .unwrap_or(0);
        let h = self.st_sessions;
        if let Some(l) = self.appbar().get_mut(h) {
            l.set_caption(&format!("● {running} sessions"));
        }
        // write-queue journal stats (applied / rejected today)
        let today_start = {
            let now = chrono::Utc::now();
            now.date_naive()
                .and_hms_opt(0, 0, 0)
                .map(|t| chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(t, chrono::Utc))
                .unwrap_or(now)
                .to_rfc3339()
        };
        if let Ok(l) = Ledger::open_default() {
            if let Ok((applied, rejected, _)) = l.ticket_counts_since(&today_start) {
                let h = self.d_tickets;
                if let Some(lbl) = self.control_mut(h) {
                    lbl.set_caption(&format!("Write tickets today: {applied} applied · {rejected} rejected (orchestrator queue)"));
                }
            }
        }
    }

    fn add_provider(&mut self) {
        if AddProviderDialog::new().show() == Some(true) {
            self.refresh_providers();
        }
    }

    fn detect(&mut self) {
        let found = providers::detect::detect_all();
        if found.is_empty() {
            dialogs::message(
                "Detect",
                "Nothing found (looked for ollama at :11434 and claude/codex/gemini/kimi on PATH).",
            );
            return;
        }
        let mut reg = providers::ProviderRegistry::load().unwrap_or_default();
        let mut names = String::new();
        for p in found {
            names.push_str(&format!(
                "• {} ({})\n",
                p.id,
                p.endpoint
                    .clone()
                    .or_else(|| p.cli_path.as_ref().map(|c| c.display().to_string()))
                    .unwrap_or_default()
            ));
            let mut p = p;
            if p.ptype != rolen_core::types::ProviderType::Cli {
                if let Ok(models) = providers::client::list_models(&p) {
                    p.models = models;
                }
            }
            reg.upsert(p);
        }
        match reg.save() {
            Ok(()) => {
                dialogs::message("Detect", &format!("Registered:\n{names}"));
                self.refresh_providers();
            }
            Err(e) => dialogs::error("Detect", &format!("failed to save registry: {e}")),
        }
    }

    // --------------------------------------------------------- projects

    fn workspace_root(&self) -> Option<std::path::PathBuf> {
        let (cfg, _) = rolen_core::config::Config::ensure().ok()?;
        let _ = cfg.ensure_workspace_root();
        Some(cfg.general.workspace_root)
    }

    fn refresh_projects(&mut self) {
        let Some(root) = self.workspace_root() else {
            return;
        };
        let projects = rolen_core::project::list_projects(&root);
        self.project_dirs = projects.iter().map(|(dir, _)| dir.clone()).collect();
        self.project_session_transcripts = Vec::new();

        // sessions by task id, for the per-task session nodes
        let sessions = rolen_core::ledger::Ledger::open_default()
            .and_then(|l| l.recent_sessions(200))
            .unwrap_or_default();

        // build the tree data first (no borrows of self while populating)
        struct TaskNode {
            label: String,
            sessions: Vec<(String, Option<std::path::PathBuf>)>,
        }
        let mut tree: Vec<(String, Vec<TaskNode>)> = Vec::new();
        let mut transcripts: Vec<Option<std::path::PathBuf>> = Vec::new();
        for (dir, m) in projects.iter() {
            let pending = m
                .clarifications
                .iter()
                .filter(|c| c.status != rolen_core::types::ClarificationStatus::Answered)
                .count();
            let marks = format!(
                "{}{}{}",
                if dir.join("PRD.json").exists() {
                    " PRD✓"
                } else {
                    ""
                },
                if dir.join("AGENTS.md").exists() {
                    " AGENTS✓"
                } else {
                    ""
                },
                if pending > 0 {
                    format!(" ❓{pending}")
                } else {
                    String::new()
                },
            );
            let mut task_nodes = Vec::new();
            if let Ok(spec) = rolen_orchestrator::BatchSpec::load(&dir.join("tasks.yaml")) {
                for t in &spec.tasks {
                    let sess: Vec<(String, Option<std::path::PathBuf>)> = sessions
                        .iter()
                        .filter(|s| s.task_id.as_deref() == Some(t.id.as_str()))
                        .map(|s| {
                            (
                                format!(
                                    "↳ {} {}/{} {}",
                                    s.id,
                                    s.provider_id,
                                    s.model,
                                    format!("{:?}", s.state).to_lowercase()
                                ),
                                s.transcript_path.clone(),
                            )
                        })
                        .collect();
                    task_nodes.push(TaskNode {
                        label: format!("⚙ {} — {} [{}]", t.id, t.title, t.role),
                        sessions: sess,
                    });
                }
            }
            tree.push((format!("{} ({}){marks}", m.name, m.id), task_nodes));
        }

        let tv = self.tv_projects;
        if let Some(tv) = self.control_mut(tv) {
            tv.clear();
            for (idx, (label, tasks)) in tree.iter().enumerate() {
                let proot = tv.add_item(treeview::Item::expandable(
                    ProjectNode {
                        label: label.clone(),
                        kind: ProjectNodeKind::Project(idx),
                    },
                    false,
                ));
                for task in tasks {
                    let tnode = tv.add_item_to_parent(
                        treeview::Item::expandable(
                            ProjectNode {
                                label: task.label.clone(),
                                kind: ProjectNodeKind::Task,
                            },
                            true,
                        ),
                        proot,
                    );
                    for (slabel, spath) in &task.sessions {
                        transcripts.push(spath.clone());
                        tv.add_item_to_parent(
                            treeview::Item::non_expandable(ProjectNode {
                                label: slabel.clone(),
                                kind: ProjectNodeKind::Session(transcripts.len() - 1),
                            }),
                            tnode,
                        );
                    }
                }
            }
        }
        self.project_session_transcripts = transcripts;
    }

    fn new_project(&mut self) {
        use crate::new_project::NewProjectDialog;
        let Some(draft) = NewProjectDialog::new().show() else {
            return;
        };
        let Some(root) = self.workspace_root() else {
            return;
        };
        let stack: Vec<String> = draft
            .stack
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        match rolen_core::project::scaffold(&draft.name, &draft.description, stack, &root) {
            Ok((meta, dir)) => {
                dialogs::message(
                    "New Project",
                    &format!(
                        "Project '{}' scaffolded at {}.\n\nNext: Project → Interview (clarification questions),\nthen Project → Build (PRD.md/PRD.json/AGENTS.md/skills/tasks.yaml).",
                        meta.id,
                        dir.display()
                    ),
                );
                self.refresh_projects();
            }
            Err(e) => dialogs::error("New Project", &e.to_string()),
        }
    }

    fn pick_project(
        &self,
        title: &str,
    ) -> Option<(std::path::PathBuf, rolen_core::project::ProjectMeta)> {
        let root = self.workspace_root()?;
        let projects = rolen_core::project::list_projects(&root);
        if projects.is_empty() {
            dialogs::message(
                title,
                "No projects yet — create one with Project → New Project.",
            );
            return None;
        }
        let entries: Vec<(String, String)> = projects
            .iter()
            .map(|(_, m)| (m.id.clone(), m.name.clone()))
            .collect();
        let id = crate::pick_project::PickProjectDialog::new(title, &entries).show()?;
        rolen_core::project::find_project(&root, &id)
    }

    /// FR-6.2 TUI form: question batches as sequential input dialogs.
    fn interview_project(&mut self) {
        let Some((dir, mut meta)) = self.pick_project("Interview") else {
            return;
        };
        let mode = rolen_core::config::Config::load()
            .map(|c| c.general.question_mode)
            .unwrap_or(rolen_core::types::QuestionMode::Thorough);
        let questions = match providers::generate::generate_questions(&meta, mode) {
            Ok(q) => q,
            Err(e) => {
                dialogs::error("Interview", &format!("question generation failed: {e}"));
                return;
            }
        };
        let mut answered = 0;
        for (i, q) in questions.iter().enumerate() {
            // FR-6.2: form controls — radio buttons for options, text field
            // otherwise; "answer later" defers (question stays pending).
            use crate::question_form::{QuestionAnswer, QuestionForm};
            let Some(result) =
                QuestionForm::new(i + 1, questions.len(), &q.question, &q.options).show()
            else {
                break; // dialog cancelled — stop the batch
            };
            let (answer, status) = match result {
                QuestionAnswer::Deferred => {
                    (None, rolen_core::types::ClarificationStatus::Deferred)
                }
                QuestionAnswer::Answered(a) => {
                    (Some(a), rolen_core::types::ClarificationStatus::Answered)
                }
            };
            if status == rolen_core::types::ClarificationStatus::Answered {
                answered += 1;
            }
            meta.clarifications.push(rolen_core::types::Clarification {
                id: format!("q{}", meta.clarifications.len() + 1),
                project_id: meta.id.clone(),
                task_id: None,
                question: q.question.clone(),
                options: q.options.clone(),
                answer,
                status,
                linked_prd_path: None,
                ts: chrono::Utc::now(),
            });
        }
        if let Err(e) = meta.save(&dir) {
            dialogs::error("Interview", &e.to_string());
        } else {
            dialogs::message(
                "Interview",
                &format!("{answered} answered and recorded into the project."),
            );
        }
        self.refresh_projects();
    }

    fn build_project(&mut self) {
        let Some((dir, meta)) = self.pick_project("Build Project") else {
            return;
        };
        let prd = match providers::generate::generate_prd(&meta) {
            Ok(p) => p,
            Err(e) => {
                dialogs::error("Build", &format!("PRD generation failed: {e}"));
                return;
            }
        };

        // FR-5.2/5.3: review before writing — full PRD.md on first build,
        // unified diffs against existing files on rebuilds.
        let new_prd_md = rolen_core::project::render_prd_md(&meta, &prd);
        let new_agents = rolen_core::project::render_agents_md(&meta, &prd);
        let mut preview = String::new();
        let mut changes = false;
        for (file, new) in [("PRD.md", &new_prd_md), ("AGENTS.md", &new_agents)] {
            let path = dir.join(file);
            match std::fs::read_to_string(&path) {
                Ok(old) if old == *new => {
                    preview.push_str(&format!("===== {file}: unchanged =====\n\n"));
                }
                Ok(old) => {
                    changes = true;
                    preview.push_str(&format!(
                        "===== {file}: diff against existing (- old / + new) =====\n{}\n",
                        rolen_core::patch::simple_diff(&old, new)
                    ));
                }
                Err(_) => {
                    changes = true;
                    preview.push_str(&format!("===== {file}: new file =====\n{new}\n\n"));
                }
            }
        }
        if changes
            && !crate::project_view::PreviewApply::new(
                "Build preview — apply these files?",
                &preview,
            )
            .show()
            .unwrap_or(false)
        {
            dialogs::message("Build", "Discarded — nothing was written.");
            return;
        }

        if let Err(e) = rolen_core::project::write_prd(&dir, &meta, &prd) {
            dialogs::error("Build", &e.to_string());
            return;
        }
        let _ = std::fs::write(dir.join("AGENTS.md"), &new_agents);
        let skills = rolen_core::project::suggest_skills(&meta, &prd, 5);
        let dag_note = match rolen_orchestrator::daggen::generate_dag(&meta, &prd) {
            Ok(tasks) => {
                let spec = rolen_orchestrator::BatchSpec { tasks };
                match serde_yaml::to_string(&spec) {
                    Ok(yaml) => {
                        let n = spec.tasks.len();
                        let _ = std::fs::write(dir.join("tasks.yaml"), yaml);
                        format!("tasks.yaml: {n} tasks")
                    }
                    Err(_) => "tasks.yaml: serialize failed".into(),
                }
            }
            Err(e) => format!("tasks.yaml: DAG proposal failed ({e})"),
        };
        dialogs::message(
            "Build",
            &format!(
                "✓ PRD.md + PRD.json ({} features)\n✓ AGENTS.md\n✓ suggested skills: {}\n✓ {}",
                prd.features.len(),
                skills
                    .iter()
                    .map(|s| s.name.clone())
                    .collect::<Vec<_>>()
                    .join(", "),
                dag_note
            ),
        );
        self.refresh_projects();
    }

    /// Rebuild the recent-sessions table from the ledger (FR-9.1).
    fn refresh_sessions(&mut self) {
        let sessions = Ledger::open_default()
            .and_then(|l| l.recent_sessions(12))
            .unwrap_or_default();
        self.session_transcripts = sessions.iter().map(|s| s.transcript_path.clone()).collect();
        let now = chrono::Utc::now();
        let rows: Vec<SessionRow> = sessions
            .into_iter()
            .map(|s| {
                let total = s.tokens_in + s.tokens_out;
                let elapsed_s = (now - s.started).num_seconds().max(0) as u64;
                SessionRow {
                    id: s.id.chars().take(20).collect(),
                    role: if s.role.is_empty() {
                        "—".into()
                    } else {
                        s.role
                    },
                    provider: s.provider_id,
                    model: s.model,
                    state: format!("{:?}", s.state).to_lowercase(),
                    tokens: fmt_tokens(total),
                    // tokens/minute over the session's lifetime (FR-9.1 rate)
                    rate: if elapsed_s >= 60 {
                        format!("{}", total * 60 / elapsed_s)
                    } else {
                        "—".into()
                    },
                    elapsed: if elapsed_s >= 3600 {
                        format!("{}h{}m", elapsed_s / 3600, elapsed_s % 3600 / 60)
                    } else if elapsed_s >= 60 {
                        format!("{}m{}s", elapsed_s / 60, elapsed_s % 60)
                    } else {
                        format!("{elapsed_s}s")
                    },
                    cost: format!("{:.3}", s.cost),
                }
            })
            .collect();
        let lv = self.lv_sessions;
        if let Some(lv) = self.control_mut(lv) {
            lv.clear();
            for row in rows {
                lv.add(row);
            }
        }
    }

    /// Launch a PTY-wrapped CLI agent task in the background (M5).
    fn run_cli_task(&mut self) {
        let Some(provider) = dialogs::input::<String>(
            "Run CLI Task",
            "cli provider id:",
            Some("cli-claude".into()),
            None,
        ) else {
            return;
        };
        let Some(task) = dialogs::input::<String>("Run CLI Task", "task instruction:", None, None)
        else {
            return;
        };
        if task.trim().is_empty() {
            return;
        }
        let default_dir = self
            .workspace_root()
            .map(|r| r.display().to_string())
            .unwrap_or_else(|| ".".into());
        let Some(workdir) =
            dialogs::input::<String>("Run CLI Task", "workdir:", Some(default_dir), None)
        else {
            return;
        };
        *CLI_JOB.lock().unwrap() = Some((
            provider.trim().to_string(),
            task.trim().to_string(),
            workdir.trim().into(),
        ));
        self.cli_output.clear();
        self.append_cli_output(&format!(
            "[rolen] starting {} in {}\n",
            provider.trim(),
            workdir.trim()
        ));
        self.show_activity_tab();
        self.bt_cli = BackgroundTask::<CliTaskMsg, bool>::run(cli_task_worker, self.handle());
    }

    /// Append a chunk of agent output to the Activity tab.
    ///
    /// The buffer is capped, because a chatty agent can emit megabytes and the
    /// whole buffer is handed to the text area on every chunk. Trimming drops
    /// from the front on a char boundary, so a multi-byte character is never
    /// cut in half.
    fn append_cli_output(&mut self, chunk: &str) {
        const CAP: usize = 200_000;
        self.cli_output
            .push_str(&crate::transcript_view::strip_ansi(chunk));
        if self.cli_output.len() > CAP {
            let cut = self.cli_output.len() - CAP;
            let cut = (cut..self.cli_output.len())
                .find(|i| self.cli_output.is_char_boundary(*i))
                .unwrap_or(self.cli_output.len());
            self.cli_output.drain(..cut);
        }
        let handle = self.ta_activity;
        let text = self.cli_output.clone();
        if let Some(area) = self.control_mut(handle) {
            area.set_text(&text);
        }
    }

    /// Bring the Activity tab up, where the live output goes.
    fn show_activity_tab(&mut self) {
        let handle = self.tabs;
        if let Some(tabs) = self.control_mut(handle) {
            tabs.set_current_tab(ACTIVITY_TAB);
        }
    }

    /// Interrogation center (FR-10.6): pending/deferred clarifications across
    /// all projects.
    fn refresh_questions(&mut self) {
        let Some(root) = self.workspace_root() else {
            return;
        };
        let mut rows = Vec::new();
        let mut refs = Vec::new();
        for (dir, meta) in rolen_core::project::list_projects(&root) {
            for c in &meta.clarifications {
                use rolen_core::types::ClarificationStatus::*;
                if matches!(c.status, Pending | Deferred) {
                    rows.push(QuestionRow {
                        project: meta.id.clone(),
                        question: c.question.chars().take(54).collect(),
                        status: format!("{:?}", c.status).to_lowercase(),
                    });
                    refs.push((dir.clone(), c.id.clone()));
                }
            }
        }
        let pending_count = rows.len();
        self.question_refs = refs;
        let lv = self.lv_questions;
        if let Some(lv) = self.control_mut(lv) {
            lv.clear();
            for row in rows {
                lv.add(row);
            }
        }
        let h = self.st_questions;
        if let Some(l) = self.appbar().get_mut(h) {
            l.set_caption(&format!("❓ {pending_count}"));
        }
    }

    fn answer_question(&mut self, index: usize) {
        let Some((dir, qid)) = self.question_refs.get(index).cloned() else {
            return;
        };
        let Ok(mut meta) = rolen_core::project::ProjectMeta::load(&dir) else {
            return;
        };
        let Some(c) = meta.clarifications.iter().find(|c| c.id == qid).cloned() else {
            return;
        };
        // FR-6.2: same form controls as the interview (radio/text + defer)
        use crate::question_form::{QuestionAnswer, QuestionForm};
        let Some(QuestionAnswer::Answered(answer)) =
            QuestionForm::new(1, 1, &c.question, &c.options).show()
        else {
            return; // cancelled or deferred — stays pending
        };
        if let Some(c) = meta.clarifications.iter_mut().find(|c| c.id == qid) {
            c.answer = Some(answer);
            c.status = rolen_core::types::ClarificationStatus::Answered;
        }
        if let Err(e) = meta.save(&dir) {
            dialogs::error("Answer", &e.to_string());
        }
        self.refresh_questions();
    }

    /// Quota threshold alerts (FR-4.5): appbar ⚠ + one-shot dialog at critical.
    fn check_alerts(&mut self) {
        let Ok(cfg) = rolen_core::config::Config::load() else {
            return;
        };
        let Ok(subs) = providers::quota::load() else {
            return;
        };
        let mut worst: Option<(String, u8)> = None;
        for s in &subs {
            if s.plan_limit.is_none() {
                continue;
            }
            if let Some(remaining) = providers::routing::remaining_pct(&s.provider_id) {
                let used = 100u8.saturating_sub(remaining);
                if used >= cfg.quotas.warn_pct {
                    let is_crit = used >= cfg.quotas.crit_pct;
                    if worst.as_ref().map(|(_, u)| used > *u).unwrap_or(true) {
                        worst = Some((s.provider_id.clone(), used));
                    }
                    if is_crit && !self.alerted.contains(&s.provider_id) {
                        self.alerted.insert(s.provider_id.clone());
                        // FR-9.4: optional OS toast in addition to the TUI popup
                        if cfg.general.os_notifications {
                            rolen_core::notify::toast(
                                "RoleN — quota critical",
                                &format!(
                                    "Provider '{}' has used {used}% of its configured budget",
                                    s.provider_id
                                ),
                            );
                        }
                        let (limit, source) = providers::quota::plan_limit(&s.provider_id)
                            .map(|(l, src)| (l.to_string(), format!("{src:?}").to_lowercase()))
                            .unwrap_or_else(|| ("?".into(), "unknown".into()));
                        // FR-4.5: execute the configured alert action
                        use rolen_core::types::AlertAction;
                        let action_note = match cfg.quotas.action {
                            AlertAction::SwitchRule => {
                                if let Ok(mut reg) = providers::ProviderRegistry::load() {
                                    if reg.set_suspended(&s.provider_id, true) {
                                        let _ = reg.save();
                                        self.refresh_providers();
                                    }
                                }
                                format!(
                                    "ACTION (switch-rule): '{0}' is now SUSPENDED — rule routing\n\
                                     skips it and fallback chains engage. Resume with:\n  \
                                     rolen provider resume --id {0}\n",
                                    s.provider_id
                                )
                            }
                            AlertAction::PauseRole => {
                                let roles = rolen_core::rules::RuleSet::load()
                                    .map(|rs| {
                                        let mut roles: Vec<String> = rs
                                            .rules
                                            .iter()
                                            .filter(|r| {
                                                r.fallback_chain.iter().any(|e| {
                                                    e.split('/').next().map(str::trim)
                                                        == Some(s.provider_id.as_str())
                                                })
                                            })
                                            .map(|r| r.role.clone())
                                            .collect();
                                        roles.sort();
                                        roles.dedup();
                                        roles
                                    })
                                    .unwrap_or_default();
                                for role in &roles {
                                    let _ = rolen_core::rules::set_role_paused(role, true);
                                }
                                format!(
                                    "ACTION (pause-role): paused role(s): {}.\n\
                                     Their dispatch fails until resumed, e.g.:\n  \
                                     rolen rule resume --role <role>\n",
                                    if roles.is_empty() {
                                        "(none route through this provider)".to_string()
                                    } else {
                                        roles.join(", ")
                                    }
                                )
                            }
                            AlertAction::Notify => String::new(),
                        };
                        dialogs::alert(
                            "Quota critical",
                            &format!(
                                "Provider '{}' has used {used}% of its CONFIGURED limit\n\
                                 ({limit} tokens, source: {source}).\n\n\
                                 {action_note}\
                                 This is RoleN's own budget setting — not necessarily the\n\
                                 provider's real plan. Adjust or remove it with:\n  \
                                 rolen provider budget --id {} --tokens <N>\n  \
                                 rolen provider budget --id {} --clear",
                                s.provider_id, s.provider_id, s.provider_id
                            ),
                        );
                    }
                    if !is_crit {
                        // recovered below critical → re-arm the alert
                        self.alerted.remove(&s.provider_id);
                    }
                }
            }
        }
        let h = self.st_queue; // queue depth label + alert marker
        if let Some(l) = self.appbar().get_mut(h) {
            // FR-7.8: live queue depth from the cross-process ticket journal
            let depth = rolen_core::ledger::Ledger::open_default()
                .and_then(|l| l.queued_ticket_count())
                .unwrap_or(0);
            match worst {
                Some((pid, used)) => l.set_caption(&format!("⚠ {pid} {used}% · queue: {depth}")),
                None => l.set_caption(&format!("queue: {depth}")),
            }
        }
    }

    /// Switch the colour theme live and remember it for next start.
    fn switch_theme(&mut self, name: &str) {
        theme::apply(name);
        if let Err(e) = theme::persist(name) {
            dialogs::error(
                "Theme",
                &format!("applied, but could not save to config: {e}"),
            );
        }
    }

    fn health_check(&mut self) {
        let reg = providers::ProviderRegistry::load().unwrap_or_default();
        if reg.is_empty() {
            dialogs::message("Health Check", "No providers registered yet.");
            return;
        }
        let mut lines = String::new();
        for p in reg.list() {
            if p.ptype == rolen_core::types::ProviderType::Cli {
                self.health.insert(p.id.clone(), "cli (M5)".into());
                lines.push_str(&format!("{:<18} cli — PTY adapter arrives in M5\n", p.id));
                continue;
            }
            let h = providers::client::health(p);
            if h.ok {
                self.health
                    .insert(p.id.clone(), format!("● ok {}ms", h.latency_ms));
                lines.push_str(&format!(
                    "{:<18} ● ok   {:>5} ms   {} models\n",
                    p.id, h.latency_ms, h.models
                ));
            } else {
                self.health.insert(p.id.clone(), "○ FAIL".into());
                lines.push_str(&format!("{:<18} ○ FAIL {}\n", p.id, h.detail));
            }
        }
        self.refresh_providers();
        dialogs::message("Health Check", &lines);
    }
}

// ------------------------------------------------------------------- events

impl MenuEvents for MissionControl {
    fn on_command(
        &mut self,
        _menu: Handle<Menu>,
        _item: Handle<menu::Command>,
        command: missioncontrol::Commands,
    ) {
        use missioncontrol::Commands::*;
        match command {
            Exit => self.close(),
            // Read from the crate metadata: a hardcoded string here silently
            // went stale for a whole release cycle.
            About => dialogs::message(
                "About RoleN",
                concat!(
                    "RoleN v",
                    env!("CARGO_PKG_VERSION"),
                    "\nA conductor for LLM-powered development.\nMIT License"
                ),
            ),
            Doctor => self.run_doctor(),
            NewProject => self.new_project(),
            Interview => self.interview_project(),
            RunProject => self.not_yet("in-TUI batch runs (M6) — use `rolen batch --spec <project>/tasks.yaml --workdir <project>`"),
            PauseProject => self.not_yet("Pause (M6)"),
            AddProvider => self.add_provider(),
            DetectClis => self.detect(),
            HealthCheck => self.health_check(),
            ModelPrices => {
                model_prices::ModelPrices::new().show();
                // prices feed the cost column, so the dashboard is now stale
                self.refresh_today();
            }
            BuildProject => self.build_project(),
            NewRule => self.edit_rule_dialog(None),
            EditRule => {
                let id = self.selected_rule_id();
                if id.is_none() {
                    dialogs::message("Edit Rule", "Select a rule in the Rules tab first.");
                }
                self.edit_rule_dialog(id);
            }
            DeleteRule => self.delete_rule(),
            DryRun => self.dry_run_rule(),
            QuickChat => {
                quick_chat::QuickChat::new().show();
                self.refresh_today();
                self.refresh_sessions();
            }
            RunCliTask => self.run_cli_task(),
            PauseAll => self.not_yet("Pause All (M6)"),
            Settings => {
                settings::SettingsWindow::new().show();
                // the settings window may have changed the theme
                if let Ok(cfg) = rolen_core::config::Config::load() {
                    theme::apply(&cfg.general.theme);
                }
            }
            ThemeDefault => self.switch_theme("default"),
            ThemeDarkGray => self.switch_theme("dark-gray"),
            ThemeLight => self.switch_theme("light"),
            ThemeDark => self.switch_theme("dark"),
            ThemeHacker => self.switch_theme("hacker"),
            ThemeFancy => self.switch_theme("fancy"),
            ThemeRainbow => self.switch_theme("rainbow"),
            ThemeOcean => self.switch_theme("ocean"),
            ThemeAmber => self.switch_theme("amber"),
            ThemePaper => self.switch_theme("paper"),
            ThemeSky => self.switch_theme("sky"),
            ThemeMint => self.switch_theme("mint"),
            ThemeSand => self.switch_theme("sand"),
        }
    }
}

impl WindowEvents for MissionControl {
    fn on_activate(&mut self) {
        if let Some(t) = self.timer() {
            t.start(Duration::from_secs(3));
        }
        self.refresh_providers();
        self.refresh_rules();
        self.refresh_projects();
    }
}

impl TimerEvents for MissionControl {
    fn on_update(&mut self, _ticks: u64) -> EventProcessStatus {
        self.refresh_today();
        self.refresh_sessions();
        self.refresh_questions();
        self.check_alerts();
        EventProcessStatus::Processed
    }
}

impl ListViewEvents<ProviderRow> for MissionControl {
    fn on_item_action(
        &mut self,
        _handle: Handle<ListView<ProviderRow>>,
        index: usize,
    ) -> EventProcessStatus {
        if let Some(pid) = self.provider_ids.get(index).cloned() {
            provider_detail::ProviderDetail::new(&pid).show();
        }
        EventProcessStatus::Processed
    }
}

impl ListViewEvents<RuleRow> for MissionControl {
    fn on_item_action(
        &mut self,
        _handle: Handle<ListView<RuleRow>>,
        index: usize,
    ) -> EventProcessStatus {
        let id = self.rule_ids.get(index).cloned();
        self.edit_rule_dialog(id);
        EventProcessStatus::Processed
    }
}

impl ListViewEvents<SessionRow> for MissionControl {
    fn on_item_action(
        &mut self,
        _handle: Handle<ListView<SessionRow>>,
        index: usize,
    ) -> EventProcessStatus {
        if let Some(Some(path)) = self.session_transcripts.get(index) {
            let path = path.clone();
            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    transcript_view::TranscriptView::new("Transcript", &content).show();
                }
                Err(e) => dialogs::error(
                    "Transcript",
                    &format!("cannot read {}: {e}", path.display()),
                ),
            }
        } else {
            dialogs::message("Transcript", "This session has no transcript file.");
        }
        EventProcessStatus::Processed
    }
}

impl ListViewEvents<QuestionRow> for MissionControl {
    fn on_item_action(
        &mut self,
        _handle: Handle<ListView<QuestionRow>>,
        index: usize,
    ) -> EventProcessStatus {
        self.answer_question(index);
        EventProcessStatus::Processed
    }
}

impl TreeViewEvents<ProjectNode> for MissionControl {
    fn on_item_action(
        &mut self,
        handle: Handle<TreeView<ProjectNode>>,
        item: Handle<treeview::Item<ProjectNode>>,
    ) -> EventProcessStatus {
        // FR-10.4: Enter on a tree node — project → detail window,
        // session → transcript.
        let node = self
            .control(handle)
            .and_then(|tv| tv.item(item).map(|i| i.value().kind.clone()));
        match node {
            Some(ProjectNodeKind::Project(idx)) => {
                if let Some(dir) = self.project_dirs.get(idx).cloned() {
                    match rolen_core::project::ProjectMeta::load(&dir) {
                        Ok(meta) => {
                            crate::project_view::ProjectView::new(&dir, &meta).show();
                        }
                        Err(e) => dialogs::error("Project", &e.to_string()),
                    }
                }
            }
            Some(ProjectNodeKind::Session(idx)) => {
                if let Some(Some(path)) = self.project_session_transcripts.get(idx).cloned() {
                    match std::fs::read_to_string(&path) {
                        Ok(content) => {
                            transcript_view::TranscriptView::new("Transcript", &content).show();
                        }
                        Err(e) => dialogs::error(
                            "Transcript",
                            &format!("cannot read {}: {e}", path.display()),
                        ),
                    }
                } else {
                    dialogs::message("Transcript", "This session has no transcript file.");
                }
            }
            _ => {}
        }
        EventProcessStatus::Processed
    }
}

impl BackgroundTaskEvents<CliTaskMsg, bool> for MissionControl {
    fn on_update(
        &mut self,
        value: CliTaskMsg,
        _: &BackgroundTask<CliTaskMsg, bool>,
    ) -> EventProcessStatus {
        match value {
            CliTaskMsg::Output(chunk) => self.append_cli_output(&chunk),
            CliTaskMsg::Harvested {
                applied,
                rejected,
                paths,
            } => {
                self.append_cli_output(&format!(
                    "\n[rolen] harvested {applied} write(s), {rejected} rejected across \
                     {paths} path(s)\n"
                ));
            }
            CliTaskMsg::Finished(msg) => {
                self.append_cli_output(&format!("\n[rolen] {msg}\n"));
                self.refresh_sessions();
                self.refresh_today();
                dialogs::message("CLI Task", &msg);
            }
        }
        EventProcessStatus::Processed
    }

    fn on_query(&mut self, _: CliTaskMsg, _: &BackgroundTask<CliTaskMsg, bool>) -> bool {
        false
    }

    fn on_finish(&mut self, _: &BackgroundTask<CliTaskMsg, bool>) -> EventProcessStatus {
        self.refresh_sessions();
        EventProcessStatus::Processed
    }
}

impl AppBarEvents for MissionControl {
    fn on_update(&self, appbar: &mut AppBar) {
        appbar.show(self.m_file);
        appbar.show(self.m_project);
        appbar.show(self.m_providers);
        appbar.show(self.m_rules);
        appbar.show(self.m_sessions);
        appbar.show(self.m_tools);
        appbar.show(self.m_help);
        appbar.show(self.st_sessions);
        appbar.show(self.st_tokens);
        appbar.show(self.st_queue);
        appbar.show(self.st_questions);
    }
}

fn fmt_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}
