//! `rolen` — headless CLI + TUI launcher (PRD FR-11).
//! Running with no subcommand opens the TUI.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use rolen_core::types::{Provider, ProviderType};
use rolen_providers as providers;

#[derive(Parser)]
#[command(
    name = "rolen",
    version,
    about = "RoleN — a conductor for LLM-powered development"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Launch the TUI (default when no subcommand is given)
    Tui,
    /// Manage and diagnose configuration
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Manage LLM providers (PRD FR-1)
    Provider {
        #[command(subcommand)]
        action: ProviderAction,
    },
    /// Show token usage / quota information (PRD FR-4)
    Quota {
        /// Restrict to one provider id
        #[arg(long)]
        provider: Option<String>,
        /// Machine-readable JSON output
        #[arg(long)]
        json: bool,
    },
    /// Manage routing rules (PRD FR-3) — canonical YAML at rules.yaml
    Rule {
        #[command(subcommand)]
        action: RuleAction,
    },
    /// Run an agent task headless through the built-in runtime (PRD FR-12)
    Run {
        /// Role to route (e.g. coder, tool-runner) — required unless --project
        #[arg(long)]
        role: Option<String>,
        /// Task description — or the task ID from tasks.yaml when --project is set
        /// (omit with --project to run the project's whole DAG)
        #[arg(long, default_value = "")]
        task: String,
        /// Workspace directory the agent is sandboxed to (project dir when --project)
        #[arg(long, default_value = ".")]
        workdir: String,
        /// Project id: run inside that project (FR-11.2)
        #[arg(long)]
        project: Option<String>,
        /// Accepted for CI parity — run is always headless (FR-11.2)
        #[arg(long)]
        headless: bool,
        /// Bypass rules: use this provider directly (requires --model)
        #[arg(long)]
        provider: Option<String>,
        /// Override the routed model
        #[arg(long)]
        model: Option<String>,
        /// Max agent steps (0 = default 30)
        #[arg(long, default_value = "0")]
        max_steps: usize,
        /// Comma-separated shell programs the agent may run (empty = disabled)
        #[arg(long, default_value = "")]
        allow_shell: String,
        /// Machine-readable JSON result (events still print to stderr)
        #[arg(long)]
        json: bool,
        /// Resume from a context snapshot written by a cancelled/paused session (FR-8.4)
        #[arg(long)]
        resume: Option<String>,
    },
    /// Run a batch of tasks as a parallel DAG through the orchestrator (PRD FR-7/FR-8)
    Batch {
        /// YAML spec: tasks with id/role/title/task/deps/claimed_paths
        #[arg(long)]
        spec: String,
        /// Workspace directory (shared; writes go through the write queue)
        #[arg(long, default_value = ".")]
        workdir: String,
        /// Max parallel tasks (0 = config heuristic)
        #[arg(long, default_value = "0")]
        max_parallel: usize,
        /// Comma-separated shell programs agents may run
        #[arg(long, default_value = "")]
        allow_shell: String,
        /// NDJSON event stream on stdout (FR-11.4)
        #[arg(long)]
        watch: bool,
    },
    /// Manage projects: scaffold, interview, PRD/AGENTS.md/skills (PRD FR-5/FR-6)
    Project {
        #[command(subcommand)]
        action: ProjectAction,
    },
    /// Validate a PRD.json file
    Prd {
        #[arg(long)]
        validate: String,
    },
    /// Run a task through a wrapped CLI agent in a PTY (PRD FR-13)
    Cli {
        #[command(subcommand)]
        action: CliAction,
    },
    /// List agent sessions from the ledger (PRD FR-9)
    Sessions {
        /// Max rows (0 = all)
        #[arg(long, default_value = "50")]
        limit: usize,
        /// Machine-readable JSON
        #[arg(long)]
        json: bool,
    },
    /// Export the ledger (entries, sessions or tickets) as CSV/JSON (FR-4.6)
    Export {
        /// ledger | sessions | tickets
        #[arg(long, default_value = "ledger")]
        what: String,
        /// csv | json
        #[arg(long, default_value = "json")]
        format: String,
        /// Output file (default: stdout)
        #[arg(long)]
        out: Option<String>,
    },
}

#[derive(Subcommand)]
enum CliAction {
    /// Run a CLI provider session: overlay → PTY → harvest via write queue
    Run {
        /// Provider id of type cli (e.g. cli-claude)
        #[arg(long)]
        provider: String,
        /// Task instruction for the CLI agent
        #[arg(long)]
        task: String,
        /// Workspace directory (the CLI runs in an overlay copy of it)
        #[arg(long, default_value = ".")]
        workdir: String,
    },
}

#[derive(Subcommand)]
enum ProjectAction {
    /// Scaffold a new project and run the clarification interview
    New {
        #[arg(long)]
        name: String,
        #[arg(long, default_value = "")]
        description: String,
        /// Comma-separated stack, e.g. "rust,appcui"
        #[arg(long, default_value = "")]
        stack: String,
        /// Question thoroughness: thorough | balanced | minimal
        #[arg(long)]
        mode: Option<String>,
        /// Skip the interview
        #[arg(long)]
        no_interview: bool,
    },
    /// List projects in the workspace root
    List,
    /// (Re)run the clarification interview for a project
    Interview {
        #[arg(long)]
        name: String,
        #[arg(long)]
        mode: Option<String>,
    },
    /// Generate PRD.md/PRD.json, AGENTS.md, skill suggestions and tasks.yaml
    Build {
        #[arg(long)]
        name: String,
    },
    /// Suggest or install skills for a project
    Skills {
        #[arg(long)]
        name: String,
        /// Install this skill into the project instead of just listing
        #[arg(long)]
        install: Option<String>,
    },
}

#[derive(Subcommand)]
enum RuleAction {
    /// List rules
    List,
    /// Seed default rules from the registered providers
    Init,
    /// Add a rule
    Add {
        #[arg(long)]
        role: String,
        /// Comma-separated provider/model chain, e.g. "kimi/k3,ollama-cloud/glm-5.2"
        #[arg(long)]
        chain: String,
        #[arg(long)]
        id: Option<String>,
        #[arg(long, default_value = "0")]
        priority: i32,
        /// Skip chain entries below this remaining-quota %
        #[arg(long)]
        min_quota_pct: Option<u8>,
        /// Restrict to a project
        #[arg(long)]
        project: Option<String>,
    },
    /// Remove a rule by id
    Remove {
        #[arg(long)]
        id: String,
    },
    /// Evaluate routing for a role right now and explain the decision (FR-3.4)
    DryRun {
        #[arg(long)]
        role: String,
        #[arg(long)]
        task_type: Option<String>,
        #[arg(long)]
        project: Option<String>,
    },
    /// Pause a role: dispatch fails until resumed (FR-4.5 pause-role)
    Pause {
        #[arg(long)]
        role: String,
    },
    /// Resume a paused role
    Resume {
        #[arg(long)]
        role: String,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Create a default config if none exists
    Init,
    /// Run environment diagnostics (dirs, config, secrets, sqlite)
    Doctor,
    /// Print the effective configuration
    Show,
    /// Print config/data file locations
    Path,
    /// Export the whole setup (secrets excluded) as a JSON bundle (FR-14.4)
    Export {
        /// Output file (default: stdout)
        #[arg(long)]
        out: Option<String>,
    },
    /// Import a setup bundle; existing files are backed up as .bak (FR-14.4)
    Import {
        #[arg(long)]
        from: String,
    },
}

#[derive(Subcommand)]
enum ProviderAction {
    /// List registered providers
    List,
    /// Register a provider (guided wizard in the TUI; flags here)
    Add {
        /// Unique id, e.g. "kimi", "ollama-local"
        #[arg(long)]
        id: String,
        /// api | cli | ollama-local | ollama-cloud
        #[arg(long, value_name = "TYPE")]
        ptype: String,
        /// Base URL (default per type: ollama-local http://localhost:11434,
        /// ollama-cloud https://ollama.com, oauth https://api.anthropic.com)
        #[arg(long)]
        endpoint: Option<String>,
        /// Path to the CLI binary (type=cli only)
        #[arg(long)]
        cli_path: Option<String>,
        /// API key — stored in the OS keychain/vault, never in config files
        #[arg(long)]
        key: Option<String>,
        /// Import Anthropic OAuth subscription tokens from opencode's
        /// auth.json ("auto" = default location)
        #[arg(long, value_name = "PATH", num_args = 0..=1, default_missing_value = "auto")]
        oauth_import: Option<String>,
        /// SSH tunnel to a remote ollama: "user@host[:port]" (keys from ~/.ssh)
        #[arg(long, value_name = "USER@HOST[:PORT]")]
        tunnel: Option<String>,
        /// Local loopback port for the tunnel (default 11435)
        #[arg(long)]
        tunnel_local_port: Option<u16>,
        /// Remote ollama port seen from the ssh server (default 11434)
        #[arg(long)]
        tunnel_remote_port: Option<u16>,
        /// Explicit ssh identity file
        #[arg(long)]
        identity: Option<String>,
        /// Skip automatic model discovery
        #[arg(long)]
        no_discover: bool,
    },
    /// Remove a provider and its stored secret
    Remove {
        #[arg(long)]
        id: String,
    },
    /// Refresh the model list / capability matrix of a provider
    Models {
        #[arg(long)]
        id: String,
    },
    /// Send a test prompt and ledger the token usage (M1 exit criteria)
    Test {
        #[arg(long)]
        id: String,
        #[arg(long)]
        model: Option<String>,
        #[arg(long, default_value = "Reply with exactly: OK")]
        prompt: String,
    },
    /// Detect ollama and known CLI agents on this machine
    Detect {
        /// Register everything found, not just print it
        #[arg(long)]
        register: bool,
    },
    /// Health check one or all providers
    Health {
        #[arg(long)]
        id: Option<String>,
    },
    /// Set or clear a manual token budget for a provider (FR-4.2)
    Budget {
        #[arg(long)]
        id: String,
        /// Tokens per billing cycle
        #[arg(long)]
        tokens: Option<u64>,
        /// Remove the budget (quota becomes unknown: no alerts, optimistic routing)
        #[arg(long)]
        clear: bool,
        /// Billing cycle start, YYYY-MM-DD (FR-4.4)
        #[arg(long)]
        cycle_start: Option<String>,
        /// Renewal date, YYYY-MM-DD (FR-4.4)
        #[arg(long)]
        renewal: Option<String>,
    },
    /// Pull quota numbers from the provider: billing endpoint (api, needs
    /// quota_url in providers.toml) or the adapter's quota probe (cli) (FR-4.1/4.2)
    SyncQuota {
        #[arg(long)]
        id: String,
    },
    /// Take a provider out of routing rotation (fallback chains skip it)
    Suspend {
        #[arg(long)]
        id: String,
    },
    /// Put a suspended provider back into routing rotation
    Resume {
        #[arg(long)]
        id: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        None | Some(Commands::Tui) => rolen_tui::run()?,
        Some(Commands::Config { action }) => config_cmd(action)?,
        Some(Commands::Provider { action }) => provider_cmd(action)?,
        Some(Commands::Quota { provider, json }) => quota_cmd(provider, json)?,
        Some(Commands::Rule { action }) => rule_cmd(action)?,
        Some(Commands::Run {
            role,
            task,
            workdir,
            project,
            headless,
            provider,
            model,
            max_steps,
            allow_shell,
            json,
            resume,
        }) => run_cmd(RunArgs {
            role,
            task,
            workdir,
            project,
            headless,
            provider,
            model,
            max_steps,
            allow_shell,
            json,
            resume,
        })?,
        Some(Commands::Batch {
            spec,
            workdir,
            max_parallel,
            allow_shell,
            watch,
        }) => batch_cmd(spec, workdir, max_parallel, allow_shell, watch)?,
        Some(Commands::Project { action }) => project_cmd(action)?,
        Some(Commands::Cli { action }) => cli_cmd(action)?,
        Some(Commands::Sessions { limit, json }) => sessions_cmd(limit, json)?,
        Some(Commands::Export { what, format, out }) => export_cmd(&what, &format, out)?,
        Some(Commands::Prd { validate }) => {
            let problems = rolen_core::project::validate_prd_json(std::path::Path::new(&validate))
                .map_err(|e| anyhow::anyhow!(e))?;
            if problems.is_empty() {
                println!(
                    "{validate}: valid PRD.json (schema v{})",
                    rolen_core::project::PRD_JSON_SCHEMA
                );
            } else {
                for p in &problems {
                    println!("✗ {p}");
                }
                std::process::exit(1);
            }
        }
    }
    Ok(())
}

// -------------------------------------------------------------------- cli

fn cli_cmd(action: CliAction) -> Result<()> {
    match action {
        CliAction::Run {
            provider,
            task,
            workdir,
        } => {
            let reg = providers::ProviderRegistry::load()?;
            let p = reg
                .get(&provider)
                .with_context(|| format!("provider '{provider}' not found"))?
                .clone();
            if p.ptype != ProviderType::Cli {
                anyhow::bail!(
                    "provider '{provider}' is not a cli provider (type {:?})",
                    p.ptype
                );
            }
            let workdir = std::path::PathBuf::from(&workdir);
            std::fs::create_dir_all(&workdir)?;
            println!(
                "▶ wrapping '{}' in PTY (overlay + harvest via write queue)…",
                p.id
            );
            let mut last_print = std::time::Instant::now();
            let report = rolen_cliadapters::run_cli_session(
                &p,
                &task,
                &workdir,
                None,
                &mut |ev| match ev {
                    rolen_cliadapters::CliEvent::Output(_) => {
                        // throttle raw PTY output to one heartbeat per second
                        if last_print.elapsed().as_secs() >= 1 {
                            last_print = std::time::Instant::now();
                            println!("  … cli working …");
                        }
                    }
                    rolen_cliadapters::CliEvent::Harvested {
                        applied,
                        rejected,
                        paths,
                    } => {
                        println!("⇣ harvest: {applied} applied, {rejected} rejected");
                        for p in paths {
                            println!("    • {p}");
                        }
                    }
                },
            )?;
            println!("\n=== cli session {} ===", report.session_id);
            println!("exit code: {:?}", report.exit_code);
            println!(
                "writes applied via queue: {} (rejected: {})",
                report.applied, report.rejected
            );
            println!(
                "tokens (estimated): {} in / {} out",
                report.tokens_in_est, report.tokens_out_est
            );
            println!("transcript: {}", report.transcript_path.display());
            if report.exit_code != Some(0) {
                std::process::exit(1);
            }
        }
    }
    Ok(())
}

// ----------------------------------------------------------------- project

fn workspace_root() -> Result<std::path::PathBuf> {
    let (cfg, _) = rolen_core::config::Config::ensure()?;
    cfg.ensure_workspace_root()?;
    Ok(cfg.general.workspace_root)
}

fn question_mode(arg: Option<String>) -> Result<rolen_core::types::QuestionMode> {
    use rolen_core::types::QuestionMode;
    match arg.as_deref() {
        None => Ok(rolen_core::config::Config::load()
            .map(|c| c.general.question_mode)
            .unwrap_or(QuestionMode::Thorough)),
        Some("thorough") => Ok(QuestionMode::Thorough),
        Some("balanced") => Ok(QuestionMode::Balanced),
        Some("minimal") => Ok(QuestionMode::Minimal),
        Some(other) => anyhow::bail!("unknown mode '{other}' (thorough | balanced | minimal)"),
    }
}

fn read_answer() -> String {
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
    line.trim().to_string()
}

/// FR-6.2: run the interview; answers land in meta.clarifications.
fn run_interview(
    meta: &mut rolen_core::project::ProjectMeta,
    mode: rolen_core::types::QuestionMode,
) -> Result<usize> {
    use rolen_core::types::{Clarification, ClarificationStatus};
    println!("generating clarifying questions (interrogator role, mode {mode:?})…");
    let questions = providers::generate::generate_questions(meta, mode)?;
    let mut answered = 0;
    for (i, q) in questions.iter().enumerate() {
        println!("\n[{}/{}] {}", i + 1, questions.len(), q.question);
        for (j, opt) in q.options.iter().enumerate() {
            println!("    {}) {}", j + 1, opt);
        }
        print!("> ");
        let _ = std::io::Write::flush(&mut std::io::stdout());
        let line = read_answer();
        let (answer, status) = if line.is_empty() {
            (None, ClarificationStatus::Deferred)
        } else {
            // numeric answer picks an option; anything else is free text
            let picked = line
                .parse::<usize>()
                .ok()
                .and_then(|n| q.options.get(n.saturating_sub(1)))
                .cloned();
            (Some(picked.unwrap_or(line)), ClarificationStatus::Answered)
        };
        if status == ClarificationStatus::Answered {
            answered += 1;
        }
        meta.clarifications.push(Clarification {
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
    Ok(answered)
}

fn project_cmd(action: ProjectAction) -> Result<()> {
    use rolen_core::project as proj;
    match action {
        ProjectAction::New {
            name,
            description,
            stack,
            mode,
            no_interview,
        } => {
            let root = workspace_root()?;
            let description = if description.is_empty() {
                println!("Describe the project in one or two sentences:");
                print!("> ");
                let _ = std::io::Write::flush(&mut std::io::stdout());
                read_answer()
            } else {
                description
            };
            let stack: Vec<String> = stack
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            let (mut meta, dir) = proj::scaffold(&name, &description, stack, &root)?;
            println!("scaffolded '{}' at {}", meta.id, dir.display());
            if !no_interview {
                let answered = run_interview(&mut meta, question_mode(mode)?)?;
                meta.save(&dir)?;
                println!(
                    "\ninterview done: {answered} answered, {} deferred",
                    meta.clarifications.len() - answered
                );
            }
            if no_interview {
                println!("next: rolen project interview --name {}", meta.id);
            }
            println!("next: rolen project build --name {}", meta.id);
        }
        ProjectAction::List => {
            let root = workspace_root()?;
            let projects = proj::list_projects(&root);
            if projects.is_empty() {
                println!(
                    "no projects in {} — create one with `rolen project new --name …`",
                    root.display()
                );
                return Ok(());
            }
            for (dir, m) in projects {
                let prd = if dir.join("PRD.json").exists() {
                    "PRD ✓"
                } else {
                    "PRD —"
                };
                let agents = if dir.join("AGENTS.md").exists() {
                    "AGENTS ✓"
                } else {
                    "AGENTS —"
                };
                println!(
                    "{:<20} {:<24} {:<8} {:<9} clarifications: {:<3} {}",
                    m.id,
                    m.name,
                    prd,
                    agents,
                    m.clarifications.len(),
                    dir.display()
                );
            }
        }
        ProjectAction::Interview { name, mode } => {
            let root = workspace_root()?;
            let (dir, mut meta) = proj::find_project(&root, &name).ok_or_else(|| {
                anyhow::anyhow!(
                    "project '{name}' not found in {} — run `rolen project list` to see project ids",
                    root.display()
                )
            })?;
            let answered = run_interview(&mut meta, question_mode(mode)?)?;
            meta.save(&dir)?;
            println!("\ninterview done: {answered} answered");
        }
        ProjectAction::Build { name } => {
            let root = workspace_root()?;
            let (dir, meta) = proj::find_project(&root, &name).ok_or_else(|| {
                anyhow::anyhow!("project '{name}' not found in {}", root.display())
            })?;

            println!("drafting PRD content (doc-writer role)…");
            let prd = providers::generate::generate_prd(&meta)?;
            proj::write_prd(&dir, &meta, &prd)?;
            println!("✓ PRD.md + PRD.json ({} features)", prd.features.len());

            let skills = proj::suggest_skills(&meta, &prd, 5);
            if !skills.is_empty() {
                println!(
                    "✓ suggested skills: {}",
                    skills
                        .iter()
                        .map(|s| s.name.clone())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                println!(
                    "  install with: rolen project skills --name {} --install <skill>",
                    meta.id
                );
            }

            let agents = proj::render_agents_md(&meta, &prd);
            std::fs::write(dir.join("AGENTS.md"), agents)?;
            println!("✓ AGENTS.md");

            println!("proposing task DAG (planner role)…");
            match rolen_orchestrator::daggen::generate_dag(&meta, &prd) {
                Ok(tasks) => {
                    let spec = rolen_orchestrator::BatchSpec { tasks };
                    let yaml = serde_yaml::to_string(&spec)?;
                    std::fs::write(dir.join("tasks.yaml"), yaml)?;
                    println!("✓ tasks.yaml ({} tasks) — run with:", spec.tasks.len());
                    println!(
                        "  rolen batch --spec {} --workdir {}",
                        dir.join("tasks.yaml").display(),
                        dir.display()
                    );
                }
                Err(e) => println!(
                    "⚠ DAG proposal failed ({e}); retry `rolen project build --name {}`",
                    meta.id
                ),
            }

            meta.save(&dir)?;
        }
        ProjectAction::Skills { name, install } => {
            let root = workspace_root()?;
            let (dir, mut meta) = proj::find_project(&root, &name)
                .ok_or_else(|| anyhow::anyhow!("project '{name}' not found"))?;
            match install {
                Some(skill) => {
                    let dst = proj::install_skill(&dir, &skill)?;
                    if !meta.skills.contains(&skill) {
                        meta.skills.push(skill.clone());
                        meta.save(&dir)?;
                    }
                    println!("installed skill '{skill}' → {}", dst.display());
                }
                None => {
                    let prd = proj::PrdContent::default(); // match on meta only
                    for s in proj::suggest_skills(&meta, &prd, 10) {
                        let mark = if meta.skills.contains(&s.name) {
                            " (installed)"
                        } else {
                            ""
                        };
                        println!("{:<18} {}{}", s.name, s.description, mark);
                    }
                }
            }
        }
    }
    Ok(())
}

// ------------------------------------------------------------------- batch

fn batch_cmd(
    spec_path: String,
    workdir: String,
    max_parallel: usize,
    allow_shell: String,
    watch: bool,
) -> Result<()> {
    let spec = rolen_orchestrator::BatchSpec::load(std::path::Path::new(&spec_path))?;
    if !watch {
        println!("batch: {} task(s), workdir {workdir}", spec.tasks.len());
    }
    exec_batch(spec, workdir.into(), max_parallel, &allow_shell, watch)
}

/// Shared DAG execution for `rolen batch` and `rolen run --project` (FR-11.2).
/// CI-safe exit codes: 0 = all tasks done, 1 = any task failed.
fn exec_batch(
    spec: rolen_orchestrator::BatchSpec,
    workdir: std::path::PathBuf,
    max_parallel: usize,
    allow_shell: &str,
    watch: bool,
) -> Result<()> {
    let opts = rolen_orchestrator::BatchOptions {
        workdir,
        max_parallel,
        shell_allow: allow_shell
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        cancel: Some(cancel_on_ctrlc()),
        pause: None,
    };
    let report = rolen_orchestrator::run_batch(&spec, &opts, &mut |ev| {
        print_batch_event(ev, watch);
    })?;
    if !report.failed.is_empty() {
        std::process::exit(1);
    }
    Ok(())
}

fn print_batch_event(ev: rolen_orchestrator::BatchEvent, watch: bool) {
    use rolen_orchestrator::BatchEvent::*;
    if watch {
        // FR-11.4: NDJSON event stream for external tooling
        let line = match &ev {
            TaskStarted { id, role } => {
                serde_json::json!({"event": "task_started", "id": id, "role": role})
            }
            Waiting { id, reason } => {
                serde_json::json!({"event": "waiting", "id": id, "reason": reason})
            }
            TaskDone { id, tokens, steps } => {
                serde_json::json!({"event": "task_done", "id": id, "tokens": tokens, "steps": steps})
            }
            TaskFailed { id, error } => {
                serde_json::json!({"event": "task_failed", "id": id, "error": error})
            }
            Agent { id, line } => serde_json::json!({"event": "agent", "id": id, "line": line}),
            AllDone { done, failed } => {
                serde_json::json!({"event": "all_done", "done": done, "failed": failed})
            }
        };
        println!("{line}");
        return;
    }
    match ev {
        TaskStarted { id, role } => println!("▶ {id} started (role {role})"),
        Waiting { id, reason } => println!("… {id} waiting: {reason}"),
        TaskDone { id, tokens, steps } => {
            println!("✅ {id} done ({steps} steps, {tokens} tokens, checkpoint committed)")
        }
        TaskFailed { id, error } => println!("❌ {id} failed: {error}"),
        Agent { id, line } => println!("[{id}] {line}"),
        AllDone { done, failed } => {
            println!("\n=== batch finished: {done} done, {failed} failed ===")
        }
    }
}

// ------------------------------------------------------------------- rules

fn rule_cmd(action: RuleAction) -> Result<()> {
    use rolen_core::rules::RuleSet;
    use rolen_core::types::Rule;
    match action {
        RuleAction::List => {
            let rules = RuleSet::load()?;
            if rules.rules.is_empty() {
                println!("no rules — seed defaults with `rolen rule init`");
                return Ok(());
            }
            for r in &rules.rules {
                let scope = r
                    .project_scope
                    .as_ref()
                    .map(|p| format!(" [project: {p}]"))
                    .unwrap_or_default();
                let minq = r
                    .min_quota_pct
                    .map(|q| format!(" (min quota {q}%)"))
                    .unwrap_or_default();
                println!(
                    "{:<18} role={:<12} prio={:<4}{}{}\n    {}",
                    r.id,
                    r.role,
                    r.priority,
                    scope,
                    minq,
                    r.fallback_chain.join(" → ")
                );
            }
        }
        RuleAction::Init => {
            let reg = providers::ProviderRegistry::load()?;
            let has = |id: &str| reg.get(id).is_some();
            let mut rules = RuleSet::load()?;
            let mut added = 0;
            let mut seed = |id: &str, role: &str, chain: Vec<(&str, &str)>| {
                let chain: Vec<String> = chain
                    .into_iter()
                    .filter(|(p, _)| has(p))
                    .map(|(p, m)| format!("{p}/{m}"))
                    .collect();
                if chain.is_empty() || rules.rules.iter().any(|r| r.id == id) {
                    return;
                }
                rules.rules.push(Rule {
                    id: id.into(),
                    role: role.into(),
                    conditions: vec![],
                    fallback_chain: chain,
                    min_quota_pct: None,
                    priority: 0,
                    project_scope: None,
                });
                added += 1;
            };
            seed(
                "planner",
                "planner",
                vec![
                    ("kimi", "k3"),
                    ("anthropic", "claude-sonnet-5"),
                    ("ollama-cloud", "glm-5.2"),
                ],
            );
            seed(
                "summarizer",
                "summarizer",
                vec![
                    ("kimi", "k3"),
                    ("anthropic", "claude-haiku-4-5-20251001"),
                    ("ollama-local", "phi3:mini"),
                ],
            );
            seed(
                "coder",
                "coder",
                vec![
                    ("kimi", "kimi-for-coding"),
                    ("ollama-cloud", "kimi-k2.7-code"),
                    ("ollama-local", "qwen2.5-coder:7b"),
                ],
            );
            seed(
                "tool-runner",
                "tool-runner",
                vec![
                    ("ollama-local", "qwen2.5-coder:7b"),
                    ("ollama-local", "mistral:latest"),
                    ("ollama-cloud", "qwen3.5:397b"),
                ],
            );
            seed(
                "reviewer",
                "reviewer",
                vec![("anthropic", "claude-sonnet-5"), ("kimi", "k3")],
            );
            seed(
                "image-reader",
                "image-reader",
                vec![
                    ("ollama-local", "mistral-small3.2:latest"),
                    ("ollama-vc", "qwen3.8:27b"),
                ],
            );
            seed(
                "doc-reader",
                "doc-reader",
                vec![
                    ("ollama-local", "qwen3:30b"),
                    ("ollama-cloud", "deepseek-v4-flash:0731"),
                ],
            );
            seed(
                "doc-writer",
                "doc-writer",
                vec![
                    ("ollama-local", "glm-4.7-flash:q4_K_M"),
                    ("ollama-cloud", "deepseek-v4-flash:0731"),
                ],
            );
            seed(
                "interrogator",
                "interrogator",
                vec![("kimi", "k3"), ("anthropic", "claude-haiku-4-5-20251001")],
            );
            rules.save()?;
            println!(
                "seeded {added} rule(s) at {}",
                rolen_core::config::rules_file()?.display()
            );
        }
        RuleAction::Add {
            role,
            chain,
            id,
            priority,
            min_quota_pct,
            project,
        } => {
            let fallback_chain: Vec<String> = chain
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if fallback_chain.is_empty() {
                anyhow::bail!("empty chain");
            }
            for e in &fallback_chain {
                if !e.contains('/') {
                    anyhow::bail!("chain entry '{e}' must look like provider/model");
                }
            }
            let id = id.unwrap_or_else(|| format!("{role}-{}", fallback_chain.len()));
            let mut rules = RuleSet::load()?;
            if rules.rules.iter().any(|r| r.id == id) {
                anyhow::bail!("rule id '{id}' already exists");
            }
            rules.rules.push(Rule {
                id: id.clone(),
                role,
                conditions: vec![],
                fallback_chain,
                min_quota_pct,
                priority,
                project_scope: project,
            });
            rules.save()?;
            println!("rule '{id}' added");
        }
        RuleAction::Remove { id } => {
            let mut rules = RuleSet::load()?;
            let before = rules.rules.len();
            rules.rules.retain(|r| r.id != id);
            if rules.rules.len() == before {
                anyhow::bail!("rule '{id}' not found");
            }
            rules.save()?;
            println!("rule '{id}' removed");
        }
        RuleAction::DryRun {
            role,
            task_type,
            project,
        } => {
            let rules = RuleSet::load()?;
            println!("collecting provider state (health, quotas)…");
            let ctx = providers::routing::collect(task_type, project)?;
            match rolen_core::rules::decide(&rules, &role, &ctx) {
                Ok(d) => {
                    println!("decision:  {}/{}", d.provider, d.model);
                    println!("rule:      {}", d.rule_id);
                    if !d.skipped.is_empty() {
                        println!("skipped:");
                        for (e, why) in &d.skipped {
                            println!("  {e:<40} {why}");
                        }
                    }
                }
                Err(e) => {
                    println!("NO ROUTE: {e}");
                    std::process::exit(1);
                }
            }
        }
        RuleAction::Pause { role } => {
            rolen_core::rules::set_role_paused(&role, true)?;
            println!(
                "role '{role}' paused — dispatch fails until `rolen rule resume --role {role}`"
            );
        }
        RuleAction::Resume { role } => {
            rolen_core::rules::set_role_paused(&role, false)?;
            println!("role '{role}' resumed");
        }
    }
    Ok(())
}

// --------------------------------------------------------------------- run

/// FR-8.4: Ctrl+C requests a graceful cancel — agents stop between steps and
/// write a context snapshot (resumable via `rolen run --resume`).
fn cancel_on_ctrlc() -> std::sync::Arc<std::sync::atomic::AtomicBool> {
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    let cancel = Arc::new(AtomicBool::new(false));
    if signal_hook::flag::register(signal_hook::consts::SIGINT, cancel.clone()).is_ok() {
        eprintln!("(Ctrl+C cancels gracefully between agent steps)");
    }
    cancel
}

struct RunArgs {
    role: Option<String>,
    task: String,
    workdir: String,
    project: Option<String>,
    #[allow(dead_code)] // accepted for CI parity (FR-11.2); run is always headless
    headless: bool,
    provider: Option<String>,
    model: Option<String>,
    max_steps: usize,
    allow_shell: String,
    json: bool,
    resume: Option<String>,
}

fn run_cmd(a: RunArgs) -> Result<()> {
    // FR-11.2: with --project, run inside the project workspace and --task
    // selects a task id from tasks.yaml (omit it to run the whole DAG).
    if let Some(project) = &a.project {
        let root = workspace_root()?;
        let (dir, meta) = rolen_core::project::find_project(&root, project).ok_or_else(|| {
            anyhow::anyhow!(
                "project '{project}' not found in {} — run `rolen project list`",
                root.display()
            )
        })?;
        let spec_path = dir.join("tasks.yaml");
        let spec = rolen_orchestrator::BatchSpec::load(&spec_path).map_err(|e| {
            anyhow::anyhow!("{e} — run `rolen project build --name {}` first", meta.id)
        })?;
        if a.task.trim().is_empty() {
            return exec_batch(spec, dir, 0, &a.allow_shell, a.json);
        }
        let t = spec
            .tasks
            .iter()
            .find(|t| t.id == a.task)
            .cloned()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "no task '{}' in {} — ids: {}",
                    a.task,
                    spec_path.display(),
                    spec.tasks
                        .iter()
                        .map(|t| t.id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?;
        return run_resolved(ResolvedRun {
            workdir: dir,
            role: t.role,
            task: t.task,
            expected_paths: t.claimed_paths,
            task_id: Some(t.id),
            provider: a.provider.clone().or(t.provider),
            model: a.model.clone().or(t.model),
            max_steps: a.max_steps,
            allow_shell: &a.allow_shell,
            json: a.json,
            resume: a.resume.clone(),
        });
    }

    let role = a
        .role
        .clone()
        .ok_or_else(|| anyhow::anyhow!("--role is required (or use --project)"))?;
    if a.task.trim().is_empty() {
        anyhow::bail!("--task is required (or use --project)");
    }
    run_resolved(ResolvedRun {
        workdir: a.workdir.clone().into(),
        role,
        task: a.task.clone(),
        expected_paths: vec![],
        task_id: None,
        provider: a.provider.clone(),
        model: a.model.clone(),
        max_steps: a.max_steps,
        allow_shell: &a.allow_shell,
        json: a.json,
        resume: a.resume.clone(),
    })
}

struct ResolvedRun<'a> {
    workdir: std::path::PathBuf,
    role: String,
    task: String,
    expected_paths: Vec<String>,
    task_id: Option<String>,
    provider: Option<String>,
    model: Option<String>,
    max_steps: usize,
    allow_shell: &'a str,
    json: bool,
    resume: Option<String>,
}

fn run_resolved(r: ResolvedRun) -> Result<()> {
    let mut opts = rolen_runtime::AgentOptions {
        workdir: r.workdir,
        role: r.role,
        task: r.task,
        provider_override: r.provider,
        model_override: r.model,
        max_steps: r.max_steps,
        expected_paths: r.expected_paths,
        task_id: r.task_id,
        resume: r.resume.map(Into::into),
        cancel: Some(cancel_on_ctrlc()),
        shell_allow: r
            .allow_shell
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        ..Default::default()
    };
    // in --json mode, human-readable events go to stderr, result JSON to stdout
    let json = r.json;
    let log = move |msg: String| {
        if json {
            eprintln!("{msg}")
        } else {
            println!("{msg}")
        }
    };
    let report = rolen_runtime::agent::run(&mut opts, &mut |ev| {
        use rolen_runtime::AgentEvent::*;
        let line = match ev {
            Routed {
                provider,
                model,
                explanation,
                ..
            } => format!("→ routed: {provider}/{model}\n  {explanation}"),
            Text(t) => format!("💬 {}", t.trim()),
            ToolCall { name, summary } => format!("🔧 {name} {}", summary),
            ToolDone {
                name,
                is_error,
                summary,
            } => format!(
                "{} {name}: {}",
                if is_error { "✗" } else { "✓" },
                summary.trim()
            ),
            Compacted {
                dropped,
                summarized,
            } => format!(
                "… (context compacted: {dropped} messages {})",
                if summarized { "summarized" } else { "dropped" }
            ),
            Paused => "⏸ paused (context snapshot written)".to_string(),
            Resumed => "▶ resumed".to_string(),
            Retrying { attempt, reason } => format!("⟳ provider retry {attempt}: {reason}"),
            Migrated { from, to, model } => {
                format!("⇄ migrated {from} → {to}/{model} (quota/overload fallback)")
            }
            Done(_) => return,
        };
        log(line);
    });
    let report = match report {
        Ok(r) => r,
        Err(rolen_runtime::error::RuntimeError::Cancelled) => {
            eprintln!("cancelled — session interrupted; context snapshot kept.");
            std::process::exit(130);
        }
        Err(e) => return Err(e.into()),
    };
    if json {
        println!(
            "{}",
            serde_json::json!({
                "session_id": report.session_id,
                "provider": report.provider,
                "model": report.model,
                "steps": report.steps,
                "tokens_in": report.tokens_in,
                "tokens_out": report.tokens_out,
                "cost": report.cost,
                "final_text": report.final_text,
            })
        );
    } else {
        println!("\n=== done in {} steps ===", report.steps);
        println!("session:  {}", report.session_id);
        println!("provider: {}/{}", report.provider, report.model);
        println!(
            "tokens:   {} in / {} out (${:.4})",
            report.tokens_in, report.tokens_out, report.cost
        );
    }
    Ok(())
}

// ------------------------------------------------------------------- config

fn config_cmd(action: ConfigAction) -> Result<()> {
    use rolen_core::config;
    match action {
        ConfigAction::Init => {
            let (_, created) = config::Config::ensure()?;
            if created {
                println!(
                    "created default config at {}",
                    config::config_file()?.display()
                );
            } else {
                println!(
                    "config already exists at {}",
                    config::config_file()?.display()
                );
            }
        }
        ConfigAction::Doctor => {
            let checks = rolen_core::doctor::run_all();
            for c in &checks {
                println!(
                    "{} {:<18} {}",
                    if c.ok { "[ OK ]" } else { "[FAIL]" },
                    c.name,
                    c.detail
                );
            }
            if !rolen_core::doctor::all_ok(&checks) {
                std::process::exit(1);
            }
        }
        ConfigAction::Show => {
            let (cfg, _) = config::Config::ensure()?;
            println!("{}", toml::to_string_pretty(&cfg)?);
        }
        ConfigAction::Path => {
            println!("config dir:      {}", config::config_dir()?.display());
            println!("data dir:        {}", config::data_dir()?.display());
            println!("config.toml:     {}", config::config_file()?.display());
            println!("providers.toml:  {}", config::providers_file()?.display());
            println!("rules.yaml:      {}", config::rules_file()?.display());
            println!(
                "subscriptions:   {}",
                config::subscriptions_file()?.display()
            );
            println!("ledger:          {}", config::ledger_file()?.display());
        }
        ConfigAction::Export { out } => {
            let bundle = config::export_setup()?;
            match out {
                Some(path) => {
                    std::fs::write(&path, &bundle)?;
                    println!("setup exported → {path} (secrets excluded)");
                }
                None => println!("{bundle}"),
            }
        }
        ConfigAction::Import { from } => {
            let bundle = std::fs::read_to_string(&from)?;
            let written = config::import_setup(&bundle)?;
            println!(
                "imported: {} (existing files backed up as .bak; re-enter API keys)",
                written.join(", ")
            );
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- providers

fn parse_ptype(s: &str) -> Result<ProviderType> {
    match s {
        "api" => Ok(ProviderType::Api),
        "cli" => Ok(ProviderType::Cli),
        "ollama-local" => Ok(ProviderType::OllamaLocal),
        "ollama-cloud" => Ok(ProviderType::OllamaCloud),
        "ollama-remote" => Ok(ProviderType::OllamaRemote),
        other => anyhow::bail!("unknown provider type '{other}' (api | cli | ollama-local | ollama-cloud | ollama-remote)"),
    }
}

fn provider_cmd(action: ProviderAction) -> Result<()> {
    match action {
        ProviderAction::List => {
            let reg = providers::ProviderRegistry::load()?;
            if reg.is_empty() {
                println!("no providers registered — try `rolen provider detect --register` or `rolen provider add --help`");
                return Ok(());
            }
            println!(
                "{:<18} {:<14} {:<40} {:<7} KEY",
                "ID", "TYPE", "ENDPOINT/CLI", "MODELS"
            );
            for p in reg.list() {
                let target = p
                    .endpoint
                    .clone()
                    .or_else(|| p.cli_path.as_ref().map(|c| c.display().to_string()))
                    .unwrap_or_else(|| "-".into());
                println!(
                    "{:<18} {:<14} {:<40} {:<7} {}",
                    p.id,
                    format!("{:?}", p.ptype),
                    target,
                    p.models.len(),
                    if p.suspended {
                        "SUSPENDED"
                    } else if p.key_ref.is_some() {
                        "stored"
                    } else {
                        "-"
                    }
                );
            }
        }
        ProviderAction::Add {
            id,
            ptype,
            endpoint,
            cli_path,
            key,
            oauth_import,
            tunnel,
            tunnel_local_port,
            tunnel_remote_port,
            identity,
            no_discover,
        } => {
            let ptype = parse_ptype(&ptype)?;

            // SSH tunnel spec (ollama across ssh)
            let tunnel = match &tunnel {
                Some(spec) => {
                    let mut t = rolen_core::types::TunnelSpec::parse(spec)
                        .map_err(|e| anyhow::anyhow!(e))?;
                    if let Some(p) = tunnel_local_port {
                        t.local_port = p;
                    }
                    if let Some(p) = tunnel_remote_port {
                        t.remote_port = p;
                    }
                    t.identity_file = identity.map(Into::into);
                    Some(t)
                }
                None => None,
            };

            // Anthropic OAuth subscription import (opencode auth.json)
            let mut auth = rolen_core::types::AuthKind::Key;
            let oauth_key_ref = if let Some(path) = &oauth_import {
                let path = if path == "auto" {
                    providers::oauth::default_opencode_auth()
                        .ok_or_else(|| anyhow::anyhow!("could not locate opencode auth.json"))?
                } else {
                    path.into()
                };
                let tokens = providers::oauth::import_from_opencode(&path)
                    .with_context(|| format!("importing OAuth tokens from {}", path.display()))?;
                let kref = providers::registry::key_ref_for(&id);
                providers::oauth::store_tokens(&kref, &tokens)
                    .context("storing OAuth tokens in keychain/vault")?;
                auth = rolen_core::types::AuthKind::OAuth;
                println!("imported OAuth subscription tokens from {}", path.display());
                Some(kref)
            } else {
                None
            };

            let endpoint = endpoint.or(match ptype {
                ProviderType::OllamaLocal if tunnel.is_none() => {
                    Some(providers::ollama::DEFAULT_LOCAL_BASE.into())
                }
                ProviderType::OllamaCloud => Some(providers::ollama::DEFAULT_CLOUD_BASE.into()),
                ProviderType::OllamaRemote if tunnel.is_none() => {
                    anyhow::bail!("ollama-remote requires --tunnel user@host[:port]")
                }
                ProviderType::Api if auth == rolen_core::types::AuthKind::OAuth => {
                    Some(providers::anthropic::DEFAULT_BASE.into())
                }
                _ => None,
            });
            let key_ref = if let Some(kref) = oauth_key_ref {
                Some(kref)
            } else if let Some(key) = &key {
                let kref = providers::registry::key_ref_for(&id);
                rolen_core::secrets::set_secret(&kref, key)
                    .context("storing API key in keychain/vault")?;
                Some(kref)
            } else {
                None
            };
            let mut provider = Provider {
                id: id.clone(),
                ptype,
                auth,
                tunnel,
                endpoint,
                cli_path: cli_path.map(Into::into),
                key_ref,
                models: Vec::new(),
                suspended: false,
                quota_url: None,
                quota_json_path: None,
            };
            if !no_discover && ptype != ProviderType::Cli {
                match providers::client::list_models(&provider) {
                    Ok(models) => {
                        println!("discovered {} models", models.len());
                        provider.models = models;
                    }
                    Err(e) => {
                        println!("warning: model discovery failed ({e}); registered without models")
                    }
                }
            }
            let mut reg = providers::ProviderRegistry::load()?;
            reg.upsert(provider);
            reg.save()?;
            println!("provider '{id}' registered");
        }
        ProviderAction::Remove { id } => {
            let mut reg = providers::ProviderRegistry::load()?;
            if reg.remove(&id) {
                reg.save()?;
                let _ = rolen_core::secrets::delete_secret(&providers::registry::key_ref_for(&id));
                println!("provider '{id}' removed");
            } else {
                anyhow::bail!("provider '{id}' not found");
            }
        }
        ProviderAction::Models { id } => {
            let n = providers::client::refresh_models(&id)?;
            let reg = providers::ProviderRegistry::load()?;
            let p = reg
                .get(&id)
                .with_context(|| format!("provider '{id}' not found"))?;
            println!("{} models for '{id}':", n);
            for m in &p.models {
                let caps: Vec<&str> = [
                    if m.vision { Some("vision") } else { None },
                    if m.tools { Some("tools") } else { None },
                    m.context_tokens.map(|_| "ctx"),
                ]
                .into_iter()
                .flatten()
                .collect();
                let ctx = m
                    .context_tokens
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "?".into());
                println!("  {:<40} ctx:{:<8} {}", m.id, ctx, caps.join(","));
            }
        }
        ProviderAction::Test { id, model, prompt } => {
            println!("sending test prompt to '{id}'…");
            let r = providers::test::test_prompt(&id, model.as_deref(), &prompt)?;
            println!("model:    {}", r.model);
            println!("reply:    {}", r.text.trim());
            let mut detail = Vec::new();
            if r.usage.cache_read > 0 {
                detail.push(format!("{} cache hits", r.usage.cache_read));
            }
            let written = r.usage.cache_write_5m + r.usage.cache_write_1h;
            if written > 0 {
                detail.push(format!("{written} cache writes"));
            }
            if detail.is_empty() {
                println!("tokens:   {} in / {} out", r.usage.input, r.usage.output);
            } else {
                println!(
                    "tokens:   {} in ({}) / {} out",
                    r.usage.input,
                    detail.join(", "),
                    r.usage.output
                );
            }
            println!("cost:     ${:.6}", r.cost);
            println!("latency:  {} ms", r.latency_ms);
            println!("ledgered — see `rolen quota` or the TUI dashboard");
        }
        ProviderAction::Detect { register } => {
            let found = detect_all();
            if found.is_empty() {
                println!("nothing detected (looked for ollama at :11434 and claude/codex/gemini/kimi on PATH)");
                return Ok(());
            }
            let mut reg = providers::ProviderRegistry::load()?;
            for p in &found {
                println!(
                    "found: {:<16} {}",
                    p.id,
                    p.endpoint
                        .clone()
                        .or_else(|| p.cli_path.as_ref().map(|c| c.display().to_string()))
                        .unwrap_or_default()
                );
                if register {
                    let mut p = p.clone();
                    if p.ptype != ProviderType::Cli {
                        if let Ok(models) = providers::client::list_models(&p) {
                            p.models = models;
                        }
                    }
                    reg.upsert(p);
                }
            }
            if register {
                reg.save()?;
                println!("registered {} provider(s)", found.len());
            } else {
                println!("(dry run — pass --register to add them)");
            }
        }
        ProviderAction::Health { id } => {
            let reg = providers::ProviderRegistry::load()?;
            let targets: Vec<&Provider> = match &id {
                Some(id) => vec![reg
                    .get(id)
                    .with_context(|| format!("provider '{id}' not found"))?],
                None => reg.list().iter().collect(),
            };
            for p in targets {
                if p.ptype == ProviderType::Cli {
                    println!("{:<18} (cli — PTY adapter arrives in M5)", p.id);
                    continue;
                }
                let h = providers::client::health(p);
                if h.ok {
                    println!(
                        "{:<18} ● ok   {:>5} ms   {} models",
                        p.id, h.latency_ms, h.models
                    );
                } else {
                    println!("{:<18} ○ FAIL {:>5} ms   {}", p.id, h.latency_ms, h.detail);
                }
            }
        }
        ProviderAction::Budget {
            id,
            tokens,
            clear,
            cycle_start,
            renewal,
        } => {
            if cycle_start.is_some() || renewal.is_some() {
                providers::quota::set_cycle(
                    &id,
                    cycle_start.as_deref().map(parse_date).transpose()?,
                    renewal.as_deref().map(parse_date).transpose()?,
                )?;
                println!("billing cycle dates set for '{id}'");
            }
            match (clear, tokens) {
                (true, _) => {
                    if providers::quota::clear_budget(&id)? {
                        println!("budget for '{id}' cleared — quota is now unknown (no threshold alerts)");
                    } else {
                        println!("no budget was configured for '{id}'");
                    }
                }
                (false, Some(t)) => {
                    providers::quota::set_manual_budget(&id, t)?;
                    println!("manual budget for '{id}': {t} tokens/cycle");
                }
                (false, None) => {
                    if cycle_start.is_none() && renewal.is_none() {
                        anyhow::bail!(
                            "pass --tokens <N>, --cycle-start/--renewal <YYYY-MM-DD>, or --clear"
                        )
                    }
                }
            }
        }
        ProviderAction::SyncQuota { id } => {
            let reg = providers::ProviderRegistry::load()?;
            let p = reg
                .get(&id)
                .ok_or_else(|| anyhow::anyhow!("no provider '{id}'"))?
                .clone();
            let (used, limit, source) = match p.ptype {
                rolen_core::types::ProviderType::Cli => {
                    let (u, l) = rolen_cliadapters::quota::cli_quota(&p)?;
                    (u, l, rolen_core::types::QuotaSource::Parsed)
                }
                _ => {
                    let (u, l) = providers::client::fetch_quota(&p)?;
                    (u, l, rolen_core::types::QuotaSource::Api)
                }
            };
            providers::quota::record_synced(&id, used, limit, source)?;
            match limit {
                Some(l) => println!(
                    "{id}: synced — used {used} of {l} ({:.0}% remaining)",
                    100.0 * (l.saturating_sub(used) as f64 / l as f64)
                ),
                None => println!("{id}: synced — used {used} (no limit reported)"),
            }
        }
        ProviderAction::Suspend { id } => {
            let mut reg = providers::ProviderRegistry::load()?;
            if !reg.set_suspended(&id, true) {
                anyhow::bail!("no provider '{id}'");
            }
            reg.save()?;
            println!("provider '{id}' suspended — rule routing skips it (fallbacks engage)");
        }
        ProviderAction::Resume { id } => {
            let mut reg = providers::ProviderRegistry::load()?;
            if !reg.set_suspended(&id, false) {
                anyhow::bail!("no provider '{id}'");
            }
            reg.save()?;
            println!("provider '{id}' back in rotation");
        }
    }
    Ok(())
}

/// Parse a YYYY-MM-DD date into midnight UTC (FR-4.4 cycle flags).
fn parse_date(s: &str) -> Result<chrono::DateTime<chrono::Utc>> {
    let d = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|_| anyhow::anyhow!("invalid date '{s}' (want YYYY-MM-DD)"))?;
    Ok(d.and_hms_opt(0, 0, 0)
        .map(|t| chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(t, chrono::Utc))
        .unwrap())
}

/// FR-1.2 detection: ollama server + known CLI agents on PATH.
fn detect_all() -> Vec<Provider> {
    providers::detect::detect_all()
}

// --------------------------------------------------------- sessions/export

/// FR-9.1/FR-11.1: list sessions from the ledger, newest first.
fn sessions_cmd(limit: usize, json: bool) -> Result<()> {
    let ledger = rolen_core::ledger::Ledger::open_default()?;
    let mut sessions = ledger.all_sessions()?;
    if limit > 0 {
        sessions.truncate(limit);
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&sessions)?);
        return Ok(());
    }
    println!(
        "{:<22} {:<12} {:<14} {:<20} {:<12} {:>10} {:>9} STARTED",
        "ID", "ROLE", "PROVIDER", "MODEL", "STATE", "TOKENS", "COST"
    );
    for s in &sessions {
        println!(
            "{:<22} {:<12} {:<14} {:<20} {:<12} {:>10} {:>8.4}$ {}",
            s.id,
            if s.role.is_empty() { "-" } else { &s.role },
            s.provider_id,
            s.model.chars().take(20).collect::<String>(),
            format!("{:?}", s.state).to_lowercase(),
            s.tokens_in + s.tokens_out,
            s.cost,
            s.started.format("%Y-%m-%d %H:%M"),
        );
    }
    Ok(())
}

/// FR-4.6/FR-11.1: export ledger entries, sessions or tickets as CSV/JSON.
fn export_cmd(what: &str, format: &str, out: Option<String>) -> Result<()> {
    let ledger = rolen_core::ledger::Ledger::open_default()?;
    let json = match format {
        "json" => true,
        "csv" => false,
        other => anyhow::bail!("unknown format '{other}' (want csv or json)"),
    };
    let text = match (what, json) {
        ("ledger", true) => serde_json::to_string_pretty(&ledger.all_entries()?)?,
        ("sessions", true) => serde_json::to_string_pretty(&ledger.all_sessions()?)?,
        ("tickets", true) => serde_json::to_string_pretty(&ledger.all_tickets()?)?,
        ("ledger", false) => {
            let mut s = String::from(
                "id,session_id,provider_id,tokens_in,tokens_cached,tokens_cache_write_5m,tokens_cache_write_1h,tokens_out,cost,latency_ms,ts\n",
            );
            for e in ledger.all_entries()? {
                s.push_str(&format!(
                    "{},{},{},{},{},{},{},{},{:.6},{},{}\n",
                    e.id,
                    e.session_id,
                    e.provider_id,
                    e.usage.input,
                    e.usage.cache_read,
                    e.usage.cache_write_5m,
                    e.usage.cache_write_1h,
                    e.usage.output,
                    e.cost,
                    e.latency_ms.map(|v| v.to_string()).unwrap_or_default(),
                    e.ts.to_rfc3339(),
                ));
            }
            s
        }
        ("sessions", false) => {
            let mut s =
                String::from("id,task_id,provider_id,model,role,state,tokens_in,tokens_out,cost,started,transcript_path\n");
            for x in ledger.all_sessions()? {
                s.push_str(&format!(
                    "{},{},{},{},{},{},{},{},{:.6},{},{}\n",
                    x.id,
                    x.task_id.unwrap_or_default(),
                    x.provider_id,
                    x.model,
                    x.role,
                    format!("{:?}", x.state).to_lowercase(),
                    x.tokens_in,
                    x.tokens_out,
                    x.cost,
                    x.started.to_rfc3339(),
                    x.transcript_path
                        .map(|p| p.display().to_string())
                        .unwrap_or_default(),
                ));
            }
            s
        }
        ("tickets", false) => {
            let mut s = String::from("id,task_id,path,op,state,ts\n");
            for t in ledger.all_tickets()? {
                s.push_str(&format!(
                    "{},{},{},{},{},{}\n",
                    t.id,
                    t.task_id,
                    t.path.display(),
                    format!("{:?}", t.op).to_lowercase(),
                    format!("{:?}", t.state).to_lowercase(),
                    t.ts.to_rfc3339(),
                ));
            }
            s
        }
        (other, _) => anyhow::bail!("unknown export '{other}' (want ledger, sessions or tickets)"),
    };
    match out {
        Some(path) => {
            std::fs::write(&path, text)?;
            println!("exported {what} ({format}) → {path}");
        }
        None => print!("{text}"),
    }
    Ok(())
}

// -------------------------------------------------------------------- quota
fn quota_cmd(provider: Option<String>, json: bool) -> Result<()> {
    let ledger = rolen_core::ledger::Ledger::open_default()?;
    let reg = providers::ProviderRegistry::load()?;
    let subs = providers::quota::load()?;

    let ids: Vec<String> = match &provider {
        Some(p) => vec![p.clone()],
        None => {
            let mut ids: Vec<String> = reg.list().iter().map(|p| p.id.clone()).collect();
            ids.sort();
            ids.dedup();
            ids
        }
    };
    let mut rows = Vec::new();
    for id in &ids {
        let u = ledger.usage_today(Some(id))?;
        if u.requests == 0 {
            continue;
        }
        let quota_pct = subs
            .iter()
            .find(|s| s.provider_id == *id)
            .and_then(|s| s.plan_limit)
            .map(|l| u.total_tokens() as f64 * 100.0 / l as f64);
        rows.push((id.clone(), u, quota_pct));
    }
    let total = ledger.usage_today(None)?;

    if json {
        let providers_json: Vec<_> = rows
            .iter()
            .map(|(id, u, pct)| {
                serde_json::json!({
                    "provider": id,
                    "tokens_in": u.tokens_in,
                    "tokens_out": u.tokens_out,
                    "cost": u.cost,
                    "requests": u.requests,
                    "quota_used_pct": pct,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "date": "today",
                "providers": providers_json,
                "total": {
                    "tokens_in": total.tokens_in,
                    "tokens_out": total.tokens_out,
                    "cost": total.cost,
                    "requests": total.requests,
                }
            }))?
        );
        return Ok(());
    }

    println!(
        "{:<18} {:>12} {:>12} {:>10} {:>8} QUOTA",
        "PROVIDER", "TOK IN", "TOK OUT", "COST", "REQS"
    );
    for (id, u, pct) in &rows {
        let quota = pct
            .map(|p| format!("{p:.1}%"))
            .unwrap_or_else(|| "-".into());
        println!(
            "{:<18} {:>12} {:>12} {:>10.4} {:>8} {}",
            id, u.tokens_in, u.tokens_out, u.cost, u.requests, quota
        );
    }
    println!(
        "{:<18} {:>12} {:>12} {:>10.4} {:>8}",
        "TOTAL (today)", total.tokens_in, total.tokens_out, total.cost, total.requests
    );
    if rows.is_empty() {
        println!("(no usage recorded yet today — try `rolen provider test --id <provider>`)");
    }
    Ok(())
}
