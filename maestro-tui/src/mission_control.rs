//! Mission Control — the main window (TUI-DESIGN.md §3).
//!
//! M1 scope: live Providers tab (registry + per-provider tokens today),
//! dashboard summary, health checks, Add-Provider wizard, and a 3-second
//! timer that refreshes the app-bar token/cost label from the ledger.
//! Sessions stay at 0 until the runtime lands in M2.

use appcui::prelude::*;
use appcui::ui::appbar::{MenuButton, Side};
use maestro_core::ledger::Ledger;
use maestro_providers as providers;
use std::collections::HashMap;
use std::time::Duration;

use crate::add_provider::AddProviderDialog;
use crate::{provider_detail, quick_chat, settings, transcript_view};

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
struct ProjectRow {
    #[Column(name: "&Name", width: 20)]
    name: String,
    #[Column(name: "&Stack", width: 16)]
    stack: String,
    #[Column(name: "PRD", width: 6)]
    prd: String,
    #[Column(name: "AGENTS", width: 8)]
    agents: String,
    #[Column(name: "Clarif.", width: 8, align: right)]
    clarifications: String,
    #[Column(name: "Skills", width: 7, align: right)]
    skills: String,
}

#[derive(ListItem)]
struct SessionRow {
    #[Column(name: "&Session", width: 26)]
    id: String,
    #[Column(name: "&Provider", width: 16)]
    provider: String,
    #[Column(name: "&Model", width: 24)]
    model: String,
    #[Column(name: "&State", width: 9)]
    state: String,
    #[Column(name: "Tokens", width: 10, align: right)]
    tokens: String,
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

/// Payload delivered by the background CLI-task worker.
pub enum CliTaskMsg {
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
        let report = maestro_cliadapters::run_cli_session(&p, &task, &workdir, None, &mut |_| {})?;
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

#[Window(events: MenuEvents+AppBarEvents+WindowEvents+TimerEvents+ListViewEvents<ProviderRow>+ListViewEvents<SessionRow>+ListViewEvents<QuestionRow>+BackgroundTaskEvents<CliTaskMsg,bool>, commands: NewProject+Interview+RunProject+PauseProject+BuildProject+AddProvider+DetectClis+HealthCheck+NewRule+DryRun+QuickChat+RunCliTask+PauseAll+Settings+Doctor+About+Exit)]
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
    // projects tab
    lv_projects: Handle<ListView<ProjectRow>>,
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
    // events/commands
    bt_cli: Handle<BackgroundTask<CliTaskMsg, bool>>,
    // last health-check results, by provider id
    health: HashMap<String, String>,
}

impl MissionControl {
    pub fn new() -> Self {
        let mut w = Self {
            base: window!("title:'Maestro — Mission Control',d:f"),
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
            lv_projects: Handle::None,
            lv_sessions: Handle::None,
            session_transcripts: Vec::new(),
            lv_questions: Handle::None,
            question_refs: Vec::new(),
            provider_ids: Vec::new(),
            alerted: std::collections::HashSet::new(),
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
                {'&Health Check All', cmd:HealthCheck}
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
        let mut t =
            tab!("tabs:[Dashboard,Projects,Providers,Rules,Questions,Activity],l:0,t:0,r:0,b:0");

        // Dashboard
        w.d_providers = t.add(0, label!("'Providers: loading…',x:2,y:0,w:76,h:1"));
        w.d_today = t.add(0, label!("'Today: —',x:2,y:1,w:76,h:1"));
        w.d_tickets = t.add(0, label!("'Write tickets: —',x:2,y:2,w:76,h:1"));
        t.add(0, label!("'Recent sessions:',x:2,y:4,w:40,h:1"));
        w.lv_sessions = t.add(
            0,
            listview!("class: SessionRow,l:1,t:5,r:1,b:1,flags: [ScrollBars]"),
        );

        // Projects
        w.lv_projects = t.add(
            1,
            listview!("class: ProjectRow,l:0,t:0,r:0,b:1,flags: [ScrollBars, SearchBar]"),
        );
        t.add(1, label!("'Project menu: New / Interview / Build — running a project: maestro batch --spec <dir>/tasks.yaml',l:1,b:0,r:1"));

        // Providers
        w.lv_providers = t.add(
            2,
            listview!("class: ProviderRow,l:0,t:0,r:0,b:1,flags: [ScrollBars, SearchBar]"),
        );
        t.add(2, label!("'Providers menu: Add / Detect / Health Check — CLI: maestro provider …',l:1,b:0,r:1"));

        // Rules
        w.lv_rules = t.add(
            3,
            listview!("class: RuleRow,l:0,t:0,r:0,b:1,flags: [ScrollBars, SearchBar]"),
        );
        t.add(3, label!("'Rules → Dry-Run evaluates a role live. Edit via CLI: maestro rule add/init/remove',l:1,b:0,r:1"));

        // Questions (interrogation center, FR-10.6): pending clarifications
        // across all projects; Enter/Space answers the selected one.
        w.lv_questions = t.add(
            4,
            listview!("class: QuestionRow,l:0,t:0,r:0,b:1,flags: [ScrollBars, SearchBar]"),
        );
        t.add(4, label!("'Enter on a question to answer it — blocked tasks resume automatically',l:1,b:0,r:1"));

        // Activity
        t.add(5, label!("'Activity: ledger stream — tickets, rule decisions, quota events (M2+).',x:2,y:1,w:70"));

        w.add(t);
        w
    }

    // ------------------------------------------------------------- helpers

    fn not_yet(&self, what: &str) {
        dialogs::message(
            "Maestro",
            &format!("'{what}' arrives in a later milestone — see PRD.md §9 roadmap."),
        );
    }

    fn run_doctor(&self) {
        let checks = maestro_core::doctor::run_all();
        let mut text = String::new();
        for c in &checks {
            text.push_str(if c.ok { "[ OK ] " } else { "[FAIL] " });
            text.push_str(c.name);
            text.push_str(" — ");
            text.push_str(&c.detail);
            text.push('\n');
        }
        if maestro_core::doctor::all_ok(&checks) {
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
            rows.push(ProviderRow {
                name: p.id.clone(),
                ptype: format!("{:?}", p.ptype),
                status: self.health.get(&p.id).cloned().unwrap_or_else(|| {
                    if p.ptype == maestro_core::types::ProviderType::Cli {
                        "cli".into()
                    } else {
                        "not checked".into()
                    }
                }),
                models: p.models.len().to_string(),
                quota,
                tokens,
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
        let rules = maestro_core::rules::RuleSet::load().unwrap_or_default();
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

    /// FR-3.4 dry-run: evaluate a role against live state and explain.
    fn dry_run_rule(&mut self) {
        let Some(role) =
            dialogs::input::<String>("Dry-Run", "Role to evaluate:", Some("coder".into()), None)
        else {
            return;
        };
        let rules = maestro_core::rules::RuleSet::load().unwrap_or_default();
        let text = match providers::routing::collect(None, None) {
            Ok(ctx) => match maestro_core::rules::decide(&rules, role.trim(), &ctx) {
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
            if p.ptype != maestro_core::types::ProviderType::Cli {
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
        let (cfg, _) = maestro_core::config::Config::ensure().ok()?;
        let _ = cfg.ensure_workspace_root();
        Some(cfg.general.workspace_root)
    }

    fn refresh_projects(&mut self) {
        let Some(root) = self.workspace_root() else {
            return;
        };
        let rows: Vec<ProjectRow> = maestro_core::project::list_projects(&root)
            .into_iter()
            .map(|(dir, m)| ProjectRow {
                name: m.id.clone(),
                stack: m.stack.join(","),
                prd: if dir.join("PRD.json").exists() {
                    "✓"
                } else {
                    "—"
                }
                .into(),
                agents: if dir.join("AGENTS.md").exists() {
                    "✓"
                } else {
                    "—"
                }
                .into(),
                clarifications: m.clarifications.len().to_string(),
                skills: m.skills.len().to_string(),
            })
            .collect();
        let lv = self.lv_projects;
        if let Some(lv) = self.control_mut(lv) {
            lv.clear();
            for row in rows {
                lv.add(row);
            }
        }
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
        match maestro_core::project::scaffold(&draft.name, &draft.description, stack, &root) {
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
    ) -> Option<(std::path::PathBuf, maestro_core::project::ProjectMeta)> {
        let root = self.workspace_root()?;
        let name = dialogs::input::<String>(title, "Project id:", None, None)?;
        maestro_core::project::find_project(&root, name.trim())
    }

    /// FR-6.2 TUI form: question batches as sequential input dialogs.
    fn interview_project(&mut self) {
        let Some((dir, mut meta)) = self.pick_project("Interview") else {
            return;
        };
        let mode = maestro_core::config::Config::load()
            .map(|c| c.general.question_mode)
            .unwrap_or(maestro_core::types::QuestionMode::Thorough);
        let questions = match providers::generate::generate_questions(&meta, mode) {
            Ok(q) => q,
            Err(e) => {
                dialogs::error("Interview", &format!("question generation failed: {e}"));
                return;
            }
        };
        let mut answered = 0;
        for (i, q) in questions.iter().enumerate() {
            let mut text = format!("[{}/{}] {}\n", i + 1, questions.len(), q.question);
            for (j, opt) in q.options.iter().enumerate() {
                text.push_str(&format!("  {}) {}\n", j + 1, opt));
            }
            text.push_str("\n(number picks an option; empty defers)");
            let Some(answer) = dialogs::input::<String>("Interview", &text, None, None) else {
                break;
            };
            let (answer, status) = if answer.trim().is_empty() {
                (None, maestro_core::types::ClarificationStatus::Deferred)
            } else {
                let picked = answer
                    .trim()
                    .parse::<usize>()
                    .ok()
                    .and_then(|n| q.options.get(n.saturating_sub(1)))
                    .cloned();
                (
                    Some(picked.unwrap_or_else(|| answer.trim().to_string())),
                    maestro_core::types::ClarificationStatus::Answered,
                )
            };
            if status == maestro_core::types::ClarificationStatus::Answered {
                answered += 1;
            }
            meta.clarifications
                .push(maestro_core::types::Clarification {
                    id: format!("q{}", meta.clarifications.len() + 1),
                    project_id: meta.id.clone(),
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
        if let Err(e) = maestro_core::project::write_prd(&dir, &meta, &prd) {
            dialogs::error("Build", &e.to_string());
            return;
        }
        let agents = maestro_core::project::render_agents_md(&meta, &prd);
        let _ = std::fs::write(dir.join("AGENTS.md"), agents);
        let skills = maestro_core::project::suggest_skills(&meta, &prd, 5);
        let dag_note = match maestro_orchestrator::daggen::generate_dag(&meta, &prd) {
            Ok(tasks) => {
                let spec = maestro_orchestrator::BatchSpec { tasks };
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
        let rows: Vec<SessionRow> = sessions
            .into_iter()
            .map(|s| SessionRow {
                id: s.id.chars().take(24).collect(),
                provider: s.provider_id,
                model: s.model,
                state: format!("{:?}", s.state).to_lowercase(),
                tokens: fmt_tokens(s.tokens_in + s.tokens_out),
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
        self.bt_cli = BackgroundTask::<CliTaskMsg, bool>::run(cli_task_worker, self.handle());
        dialogs::message("Run CLI Task", "CLI agent started in the background (PTY + overlay + write queue).\nThe dashboard session list updates when it finishes.");
    }

    /// Interrogation center (FR-10.6): pending/deferred clarifications across
    /// all projects.
    fn refresh_questions(&mut self) {
        let Some(root) = self.workspace_root() else {
            return;
        };
        let mut rows = Vec::new();
        let mut refs = Vec::new();
        for (dir, meta) in maestro_core::project::list_projects(&root) {
            for c in &meta.clarifications {
                use maestro_core::types::ClarificationStatus::*;
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
        let Ok(mut meta) = maestro_core::project::ProjectMeta::load(&dir) else {
            return;
        };
        let Some(c) = meta.clarifications.iter().find(|c| c.id == qid).cloned() else {
            return;
        };
        let mut text = format!("{}\n", c.question);
        for (j, opt) in c.options.iter().enumerate() {
            text.push_str(&format!("  {}) {}\n", j + 1, opt));
        }
        text.push_str("\n(number picks an option; empty keeps it pending)");
        let Some(answer) = dialogs::input::<String>("Answer clarification", &text, None, None)
        else {
            return;
        };
        if answer.trim().is_empty() {
            return;
        }
        let picked = answer
            .trim()
            .parse::<usize>()
            .ok()
            .and_then(|n| c.options.get(n.saturating_sub(1)))
            .cloned();
        if let Some(c) = meta.clarifications.iter_mut().find(|c| c.id == qid) {
            c.answer = Some(picked.unwrap_or_else(|| answer.trim().to_string()));
            c.status = maestro_core::types::ClarificationStatus::Answered;
        }
        if let Err(e) = meta.save(&dir) {
            dialogs::error("Answer", &e.to_string());
        }
        self.refresh_questions();
    }

    /// Quota threshold alerts (FR-4.5): appbar ⚠ + one-shot dialog at critical.
    fn check_alerts(&mut self) {
        let Ok(cfg) = maestro_core::config::Config::load() else {
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
                        let (limit, source) = providers::quota::plan_limit(&s.provider_id)
                            .map(|(l, src)| (l.to_string(), format!("{src:?}").to_lowercase()))
                            .unwrap_or_else(|| ("?".into(), "unknown".into()));
                        dialogs::alert(
                            "Quota critical",
                            &format!(
                                "Provider '{}' has used {used}% of its CONFIGURED limit\n\
                                 ({limit} tokens, source: {source}).\n\n\
                                 This is Maestro's own budget setting — not necessarily the\n\
                                 provider's real plan. Adjust or remove it with:\n  \
                                 maestro provider budget --id {} --tokens <N>\n  \
                                 maestro provider budget --id {} --clear\n\n\
                                 Rules with quota fallbacks will skip this provider meanwhile.",
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
        let h = self.st_queue; // reuse: shows queue depth; add alert marker
        if let Some(l) = self.appbar().get_mut(h) {
            match worst {
                Some((pid, used)) => l.set_caption(&format!("⚠ {pid} {used}%")),
                None => l.set_caption("queue: 0"),
            }
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
            if p.ptype == maestro_core::types::ProviderType::Cli {
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
            About => dialogs::message(
                "About Maestro",
                "Maestro v0.1.0\nA conductor for LLM-powered development.\nMIT License",
            ),
            Doctor => self.run_doctor(),
            NewProject => self.new_project(),
            Interview => self.interview_project(),
            RunProject => self.not_yet("in-TUI batch runs (M6) — use `maestro batch --spec <project>/tasks.yaml --workdir <project>`"),
            PauseProject => self.not_yet("Pause (M6)"),
            AddProvider => self.add_provider(),
            DetectClis => self.detect(),
            HealthCheck => self.health_check(),
            BuildProject => self.build_project(),
            NewRule => self.not_yet("visual rule editor (M6) — use `maestro rule add` meanwhile"),
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
            }
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

impl BackgroundTaskEvents<CliTaskMsg, bool> for MissionControl {
    fn on_update(
        &mut self,
        value: CliTaskMsg,
        _: &BackgroundTask<CliTaskMsg, bool>,
    ) -> EventProcessStatus {
        match value {
            CliTaskMsg::Finished(msg) => {
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
