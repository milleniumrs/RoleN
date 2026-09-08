//! Dependency-aware parallel scheduler (Requirements FR-8).
//!
//! Runs batches of tasks as DAGs: a task starts when all its deps succeeded
//! AND its claimed_paths don't overlap any running task (FR-7.5) AND the
//! parallelism cap allows it (FR-8.1). Every agent writes through a WriteQueue
//! (FR-7.1); each completed task produces a git checkpoint (FR-7.7).
//!
//! `run_projects` runs several project DAGs in one process under one shared
//! task-slot budget (FR-8.1) with the write-ticket cap split fairly across
//! projects (FR-7.8). `run_batch` remains the single-project compatibility
//! wrapper used by `rolen batch`.

use crate::git;
use crate::queue::{QueuedWriteSink, WriteQueue};
use rolen_runtime::agent::{self, AgentEvent, AgentOptions, RunReport};
use rolen_runtime::error::RuntimeError;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSpec {
    pub id: String,
    pub role: String,
    pub title: String,
    /// The instruction given to the agent.
    pub task: String,
    #[serde(default)]
    pub deps: Vec<String>,
    /// Files this task owns (FR-7.5); overlapping claims never run together.
    #[serde(default)]
    pub claimed_paths: Vec<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct BatchSpec {
    #[serde(default)]
    pub tasks: Vec<TaskSpec>,
}

impl BatchSpec {
    pub fn load(path: &std::path::Path) -> Result<Self, SchedError> {
        let text = std::fs::read_to_string(path)?;
        serde_yaml::from_str(&text)
            .map_err(|e| SchedError::Spec(format!("{}: {e}", path.display())))
    }
}

#[derive(Debug, Error)]
pub enum SchedError {
    #[error("spec error: {0}")]
    Spec(String),
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("cycle or missing dependency involving task '{0}'")]
    Dag(String),
}

pub struct BatchOptions {
    pub workdir: PathBuf,
    /// 0 = config heuristic (D6).
    pub max_parallel: usize,
    pub shell_allow: Vec<String>,
    pub cancel: Option<Arc<AtomicBool>>,
    /// Cooperative pause for all task sessions (FR-8.4).
    pub pause: Option<Arc<AtomicBool>>,
}

/// One project's DAG plus the workspace it is allowed to write.
#[derive(Debug, Clone)]
pub struct ProjectRun {
    pub project_id: String,
    pub spec: BatchSpec,
    pub workdir: PathBuf,
}

/// Options shared by every project in one `run_projects` call.
pub struct SharedBatchOptions {
    /// 0 = config heuristic (D6). This budget is shared by all projects.
    pub max_parallel: usize,
    pub shell_allow: Vec<String>,
    pub cancel: Option<Arc<AtomicBool>>,
    /// Cooperative pause for all task sessions (FR-8.4).
    pub pause: Option<Arc<AtomicBool>>,
}

pub enum BatchEvent {
    TaskStarted {
        project: String,
        id: String,
        role: String,
    },
    Agent {
        project: String,
        id: String,
        line: String,
    },
    TaskDone {
        project: String,
        id: String,
        tokens: u64,
        steps: usize,
    },
    TaskFailed {
        project: String,
        id: String,
        error: String,
    },
    Waiting {
        project: String,
        id: String,
        reason: String,
    },
    AllDone {
        project: String,
        done: usize,
        failed: usize,
    },
}

#[derive(Debug)]
pub struct BatchReport {
    pub done: Vec<(String, RunReport)>,
    pub failed: Vec<(String, String)>,
}

enum TaskStatus {
    Pending,
    Running,
    Done,
    Failed,
}

type TaskResults = Arc<Mutex<Vec<(String, Result<RunReport, RuntimeError>)>>>;

struct ProjectState {
    run: ProjectRun,
    queue: Arc<WriteQueue>,
    git_ok: bool,
    status: HashMap<String, TaskStatus>,
    results: TaskResults,
    finished: Arc<Mutex<HashSet<String>>>,
    done_reports: Vec<(String, RunReport)>,
    failed: Vec<(String, String)>,
    project_dir: Option<PathBuf>,
    pending_questions: HashSet<String>,
    question_wait_reported: HashSet<String>,
    poll_countdown: u32,
}

/// Single-project compatibility wrapper (used by `rolen batch`).
pub fn run_batch(
    spec: &BatchSpec,
    opts: &BatchOptions,
    on_event: &mut dyn FnMut(BatchEvent),
) -> Result<BatchReport, SchedError> {
    let shared = SharedBatchOptions {
        max_parallel: opts.max_parallel,
        shell_allow: opts.shell_allow.clone(),
        cancel: opts.cancel.clone(),
        pause: opts.pause.clone(),
    };
    let run = ProjectRun {
        project_id: "batch".into(),
        spec: spec.clone(),
        workdir: opts.workdir.clone(),
    };
    let mut reports = run_projects(vec![run], &shared, on_event)?;
    Ok(reports.remove(0))
}

/// FR-8.1: run several project DAGs concurrently in one process. All projects
/// share one task-slot budget; FR-7.8 splits the write-ticket queue cap evenly
/// across projects so a write-heavy project cannot starve the others.
pub fn run_projects(
    runs: Vec<ProjectRun>,
    opts: &SharedBatchOptions,
    on_event: &mut dyn FnMut(BatchEvent),
) -> Result<Vec<BatchReport>, SchedError> {
    if runs.is_empty() {
        return Ok(Vec::new());
    }
    let mut ids = HashSet::new();
    for run in &runs {
        if run.project_id.trim().is_empty() {
            return Err(SchedError::Spec("project id must not be empty".into()));
        }
        if !ids.insert(run.project_id.clone()) {
            return Err(SchedError::Spec(format!(
                "duplicate project id '{}'",
                run.project_id
            )));
        }
        validate_dag(&run.spec)?;
        std::fs::create_dir_all(&run.workdir)?;
    }

    let cfg = rolen_core::config::Config::load().unwrap_or_default();
    let queue_cap = per_project_queue_cap(cfg.parallelism.queue_cap, runs.len());
    let max_parallel = if opts.max_parallel > 0 {
        opts.max_parallel
    } else {
        cfg.parallelism.effective_global_cap()
    };
    let cancel = opts
        .cancel
        .clone()
        .unwrap_or_else(|| Arc::new(AtomicBool::new(false)));

    let mut states: Vec<ProjectState> = runs
        .into_iter()
        .map(|run| {
            let git_ok = git::ensure_repo(&run.workdir);
            let status = run
                .spec
                .tasks
                .iter()
                .map(|t| (t.id.clone(), TaskStatus::Pending))
                .collect();
            let project_dir = rolen_core::project::find_project_dir_upwards(&run.workdir);
            ProjectState {
                queue: WriteQueue::with_capacity(run.workdir.clone(), queue_cap),
                run,
                git_ok,
                status,
                results: Arc::new(Mutex::new(Vec::new())),
                finished: Arc::new(Mutex::new(HashSet::new())),
                done_reports: Vec::new(),
                failed: Vec::new(),
                project_dir,
                pending_questions: HashSet::new(),
                question_wait_reported: HashSet::new(),
                poll_countdown: 0,
            }
        })
        .collect();

    // Claims are process-global and scoped by workdir, so two projects sharing
    // a directory still cannot write the same path concurrently (FR-7.5).
    let mut claimed: HashMap<String, String> = HashMap::new(); // claim key -> global task key
    let mut handles: Vec<std::thread::JoinHandle<()>> = Vec::new();
    // Agent output lines travel through this channel so the event consumer
    // decides how to present them (human text vs NDJSON stream).
    let (agent_tx, agent_rx) = std::sync::mpsc::channel::<(String, String, String)>();
    let mut round_robin = 0usize;

    loop {
        // forward agent output lines
        while let Ok((project, id, line)) = agent_rx.try_recv() {
            on_event(BatchEvent::Agent { project, id, line });
        }

        // harvest finished tasks per project
        for (idx, state) in states.iter_mut().enumerate() {
            let just_finished: Vec<String> = state.finished.lock().unwrap().drain().collect();
            for id in just_finished {
                let gkey = global_task_key(idx, &id);
                claimed.retain(|_, owner| owner != &gkey);
                let res = state
                    .results
                    .lock()
                    .unwrap()
                    .drain(..)
                    .find(|(rid, _)| rid == &id);
                let task = state.run.spec.tasks.iter().find(|t| t.id == id).unwrap();
                match res {
                    Some((_, Ok(report))) => {
                        // Honesty check: a task that claims files must have
                        // produced them — a model that only narrated is a failure.
                        let missing: Vec<&String> = task
                            .claimed_paths
                            .iter()
                            .filter(|p| !state.run.workdir.join(p).exists())
                            .collect();
                        if missing.is_empty() {
                            state.status.insert(id.clone(), TaskStatus::Done);
                            if state.git_ok {
                                let _ = git::checkpoint(&state.run.workdir, &id, &task.title);
                            }
                            on_event(BatchEvent::TaskDone {
                                project: state.run.project_id.clone(),
                                id: id.clone(),
                                tokens: report.tokens_in + report.tokens_out,
                                steps: report.steps,
                            });
                            state.done_reports.push((id.clone(), report));
                        } else {
                            let err = format!(
                                "agent finished but claimed file(s) missing: {}",
                                missing
                                    .iter()
                                    .map(|s| s.as_str())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            );
                            state.status.insert(id.clone(), TaskStatus::Failed);
                            on_event(BatchEvent::TaskFailed {
                                project: state.run.project_id.clone(),
                                id: id.clone(),
                                error: err.clone(),
                            });
                            state.failed.push((id.clone(), err));
                        }
                    }
                    Some((_, Err(e))) => {
                        state.status.insert(id.clone(), TaskStatus::Failed);
                        on_event(BatchEvent::TaskFailed {
                            project: state.run.project_id.clone(),
                            id: id.clone(),
                            error: e.to_string(),
                        });
                        state.failed.push((id.clone(), e.to_string()));
                    }
                    None => {
                        state.status.insert(id.clone(), TaskStatus::Failed);
                        state.failed.push((id.clone(), "worker vanished".into()));
                    }
                }
            }
        }

        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if states.iter().all(|state| {
            state
                .status
                .values()
                .all(|s| !matches!(s, TaskStatus::Pending | TaskStatus::Running))
        }) {
            break;
        }

        let running_total: usize = states
            .iter()
            .map(|state| {
                state
                    .status
                    .values()
                    .filter(|s| matches!(s, TaskStatus::Running))
                    .count()
            })
            .sum();
        let mut slots = max_parallel.saturating_sub(running_total);
        if slots > 0 {
            // Rotate the first project inspected each tick: with scarce slots a
            // fixed order would always feed project 0 first (FR-7.8 fairness).
            for offset in 0..states.len() {
                if slots == 0 {
                    break;
                }
                let idx = (round_robin + offset) % states.len();
                let state = &mut states[idx];

                // FR-6.3: re-read this project's pending questions ~once/sec.
                if let Some(dir) = &state.project_dir {
                    if state.poll_countdown == 0 {
                        state.pending_questions =
                            rolen_core::project::pending_question_task_ids(dir);
                        state.poll_countdown = 10;
                    }
                    state.poll_countdown -= 1;
                }

                for task in &state.run.spec.tasks {
                    if slots == 0 {
                        break;
                    }
                    if !matches!(state.status.get(&task.id), Some(TaskStatus::Pending)) {
                        continue;
                    }
                    // deps must all be done; a failed dep blocks permanently
                    let dep_failed = task
                        .deps
                        .iter()
                        .any(|d| matches!(state.status.get(d), Some(TaskStatus::Failed)));
                    if dep_failed {
                        state.status.insert(task.id.clone(), TaskStatus::Failed);
                        on_event(BatchEvent::TaskFailed {
                            project: state.run.project_id.clone(),
                            id: task.id.clone(),
                            error: "dependency failed".into(),
                        });
                        state
                            .failed
                            .push((task.id.clone(), "dependency failed".into()));
                        continue;
                    }
                    let deps_done = task
                        .deps
                        .iter()
                        .all(|d| matches!(state.status.get(d), Some(TaskStatus::Done)));
                    if !deps_done {
                        continue;
                    }
                    // FR-6.3: pause while a dependency has an unanswered question
                    if blocked_by_question(&state.run.spec, &task.id, &state.pending_questions) {
                        if state.question_wait_reported.insert(task.id.clone()) {
                            on_event(BatchEvent::Waiting {
                                project: state.run.project_id.clone(),
                                id: task.id.clone(),
                                reason: "awaiting answer to a clarification question \
                                         (see the TUI Questions tab)"
                                    .into(),
                            });
                        }
                        continue;
                    }
                    state.question_wait_reported.remove(&task.id);
                    // path claims (FR-7.5)
                    if task
                        .claimed_paths
                        .iter()
                        .any(|p| claimed.contains_key(&claim_key(&state.run.workdir, p)))
                    {
                        on_event(BatchEvent::Waiting {
                            project: state.run.project_id.clone(),
                            id: task.id.clone(),
                            reason: "path claim overlap with a running task".into(),
                        });
                        continue;
                    }

                    let gkey = global_task_key(idx, &task.id);
                    for p in &task.claimed_paths {
                        claimed.insert(claim_key(&state.run.workdir, p), gkey.clone());
                    }
                    state.status.insert(task.id.clone(), TaskStatus::Running);
                    slots -= 1;
                    on_event(BatchEvent::TaskStarted {
                        project: state.run.project_id.clone(),
                        id: task.id.clone(),
                        role: task.role.clone(),
                    });

                    // spawn the agent thread
                    let project_id = state.run.project_id.clone();
                    let task = task.clone();
                    let workdir = state.run.workdir.clone();
                    let shell_allow = opts.shell_allow.clone();
                    let sink = QueuedWriteSink::new(state.queue.clone());
                    let results = state.results.clone();
                    let finished = state.finished.clone();
                    let cancel = cancel.clone();
                    let pause = opts.pause.clone();
                    let agent_tx = agent_tx.clone();
                    handles.push(std::thread::spawn(move || {
                        let prefix_project = project_id.clone();
                        let prefix_id = task.id.clone();
                        let printer = move |line: String| {
                            let _ =
                                agent_tx.send((prefix_project.clone(), prefix_id.clone(), line));
                        };
                        let mut opts = AgentOptions {
                            workdir,
                            role: task.role.clone(),
                            task: task.task.clone(),
                            provider_override: task.provider.clone(),
                            model_override: task.model.clone(),
                            task_id: Some(task.id.clone()),
                            expected_paths: task.claimed_paths.clone(),
                            sink: Some(Box::new(sink)),
                            cancel: Some(cancel),
                            pause,
                            shell_allow,
                            ..Default::default()
                        };
                        let res = agent::run(&mut opts, &mut |ev| match ev {
                            AgentEvent::Routed {
                                provider, model, ..
                            } => printer(format!("→ {provider}/{model}")),
                            AgentEvent::Text(t) => printer(format!(
                                "💬 {}",
                                t.chars().take(160).collect::<String>().trim()
                            )),
                            AgentEvent::ToolCall { name, summary } => printer(format!(
                                "🔧 {name} {}",
                                summary.chars().take(100).collect::<String>()
                            )),
                            AgentEvent::ToolDone { name, is_error, .. } => {
                                printer(format!("{} {name}", if is_error { "✗" } else { "✓" }))
                            }
                            AgentEvent::Compacted {
                                dropped,
                                summarized,
                            } => {
                                if summarized {
                                    printer(format!("… compacted ({dropped} summarized)"))
                                } else {
                                    printer(format!("… compacted ({dropped} dropped)"))
                                }
                            }
                            AgentEvent::Paused => printer("⏸ paused".into()),
                            AgentEvent::Resumed => printer("▶ resumed".into()),
                            AgentEvent::Retrying { attempt, reason } => {
                                printer(format!("⟳ retry {attempt}: {reason}"))
                            }
                            AgentEvent::Migrated { from, to, model } => {
                                printer(format!("⇄ migrated {from} → {to}/{model}"))
                            }
                            AgentEvent::Done(_) => {}
                        });
                        results.lock().unwrap().push((task.id.clone(), res));
                        finished.lock().unwrap().insert(task.id.clone());
                    }));
                }
            }
            round_robin = round_robin.wrapping_add(1);
        }

        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    for h in handles {
        let _ = h.join();
    }
    for state in &states {
        state.queue.shutdown();
    }

    // final drain of agent lines before reporting completion
    while let Ok((project, id, line)) = agent_rx.try_recv() {
        on_event(BatchEvent::Agent { project, id, line });
    }

    let mut reports = Vec::new();
    for mut state in states {
        let report = BatchReport {
            done: std::mem::take(&mut state.done_reports),
            failed: state.failed.clone(),
        };
        on_event(BatchEvent::AllDone {
            project: state.run.project_id.clone(),
            done: report.done.len(),
            failed: report.failed.len(),
        });
        reports.push(report);
    }
    Ok(reports)
}

fn global_task_key(project_idx: usize, task_id: &str) -> String {
    format!("{project_idx}:{task_id}")
}

fn claim_key(workdir: &Path, rel: &str) -> String {
    format!("{}::{rel}", workdir.to_string_lossy())
}

/// FR-7.8: split the global queue cap across concurrently running projects.
/// 0 stays unlimited; a non-zero cap always leaves at least one slot per
/// project so no project is starved to death by arithmetic rounding.
fn per_project_queue_cap(total_cap: usize, projects: usize) -> usize {
    if total_cap == 0 || projects <= 1 {
        total_cap
    } else {
        (total_cap / projects).max(1)
    }
}

/// FR-6.3: true when any transitive dependency of `id` has a pending
/// clarification question (`pending` holds task ids with unanswered
/// questions). The DAG is already validated, so recursion terminates.
fn blocked_by_question(spec: &BatchSpec, id: &str, pending: &HashSet<String>) -> bool {
    if pending.is_empty() {
        return false;
    }
    fn walk(
        spec: &BatchSpec,
        id: &str,
        pending: &HashSet<String>,
        seen: &mut HashSet<String>,
    ) -> bool {
        let Some(task) = spec.tasks.iter().find(|t| t.id == id) else {
            return false;
        };
        for dep in &task.deps {
            if pending.contains(dep) {
                return true;
            }
            if seen.insert(dep.clone()) && walk(spec, dep, pending, seen) {
                return true;
            }
        }
        false
    }
    walk(spec, id, pending, &mut HashSet::new())
}

fn validate_dag(spec: &BatchSpec) -> Result<(), SchedError> {
    let ids: HashSet<&str> = spec.tasks.iter().map(|t| t.id.as_str()).collect();
    if ids.len() != spec.tasks.len() {
        return Err(SchedError::Dag("duplicate task id".into()));
    }
    for t in &spec.tasks {
        for d in &t.deps {
            if !ids.contains(d.as_str()) {
                return Err(SchedError::Dag(format!(
                    "{} depends on unknown '{d}'",
                    t.id
                )));
            }
        }
    }
    // cycle check via DFS colors
    let mut color: HashMap<&str, u8> = HashMap::new(); // 0=unvisited 1=in-stack 2=done
    fn visit<'a>(
        id: &'a str,
        spec: &'a BatchSpec,
        color: &mut HashMap<&'a str, u8>,
    ) -> Result<(), SchedError> {
        match color.get(id).copied().unwrap_or(0) {
            1 => return Err(SchedError::Dag(format!("cycle at '{id}'"))),
            2 => return Ok(()),
            _ => {}
        }
        color.insert(id, 1);
        let task = spec.tasks.iter().find(|t| t.id == id).unwrap();
        for d in &task.deps {
            visit(d, spec, color)?;
        }
        color.insert(id, 2);
        Ok(())
    }
    for t in &spec.tasks {
        visit(&t.id, spec, &mut color)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> BatchSpec {
        let task = |id: &str, deps: &[&str]| TaskSpec {
            id: id.into(),
            role: "coder".into(),
            title: id.into(),
            task: "do it".into(),
            deps: deps.iter().map(|s| s.to_string()).collect(),
            claimed_paths: vec![],
            provider: None,
            model: None,
        };
        BatchSpec {
            tasks: vec![
                task("a", &[]),
                task("b", &["a"]),
                task("c", &["b"]),
                task("d", &[]),
            ],
        }
    }

    #[test]
    fn dag_validation_accepts_and_rejects() {
        assert!(validate_dag(&spec()).is_ok());
        let mut bad = spec();
        bad.tasks[0].deps = vec!["c".into()]; // a -> c -> b -> a cycle
        assert!(validate_dag(&bad).is_err());
    }

    #[test]
    fn question_blocks_transitive_dependents_only() {
        let pending: HashSet<String> = ["a".to_string()].into_iter().collect();
        assert!(!blocked_by_question(&spec(), "a", &pending)); // the asker itself runs
        assert!(blocked_by_question(&spec(), "b", &pending)); // direct dependent
        assert!(blocked_by_question(&spec(), "c", &pending)); // transitive dependent
        assert!(!blocked_by_question(&spec(), "d", &pending)); // unrelated task
        assert!(!blocked_by_question(&spec(), "b", &HashSet::new()));
    }

    #[test]
    fn queue_cap_is_split_fairly_across_projects() {
        assert_eq!(per_project_queue_cap(0, 3), 0); // unlimited stays unlimited
        assert_eq!(per_project_queue_cap(1000, 1), 1000); // single project: unchanged
        assert_eq!(per_project_queue_cap(1000, 4), 250);
        assert_eq!(per_project_queue_cap(3, 4), 1); // rounding never starves a project
    }

    #[test]
    fn claim_keys_are_scoped_to_their_workdir() {
        let a = PathBuf::from("a");
        let b = PathBuf::from("b");
        assert_ne!(claim_key(&a, "src/lib.rs"), claim_key(&b, "src/lib.rs"));
        assert_eq!(claim_key(&a, "x"), claim_key(&a, "x"));
    }

    fn temp_project(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "rolen-sched-{tag}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn shared_opts() -> SharedBatchOptions {
        SharedBatchOptions {
            max_parallel: 2,
            shell_allow: vec![],
            cancel: None,
            pause: None,
        }
    }

    #[test]
    fn empty_project_specs_run_together_in_one_process() {
        let a = temp_project("a");
        let b = temp_project("b");
        let runs = vec![
            ProjectRun {
                project_id: "a".into(),
                spec: BatchSpec::default(),
                workdir: a.clone(),
            },
            ProjectRun {
                project_id: "b".into(),
                spec: BatchSpec::default(),
                workdir: b.clone(),
            },
        ];
        let mut events = Vec::new();
        let reports = run_projects(runs, &shared_opts(), &mut |ev| {
            if let BatchEvent::AllDone { project, .. } = ev {
                events.push(project);
            }
        })
        .unwrap();
        assert_eq!(reports.len(), 2);
        assert!(reports.iter().all(|r| r.failed.is_empty()));
        assert_eq!(events, vec!["a".to_string(), "b".to_string()]);
        std::fs::remove_dir_all(&a).ok();
        std::fs::remove_dir_all(&b).ok();
    }

    #[test]
    fn duplicate_project_ids_are_rejected() {
        let a = temp_project("dup-a");
        let b = temp_project("dup-b");
        let runs = vec![
            ProjectRun {
                project_id: "same".into(),
                spec: BatchSpec::default(),
                workdir: a.clone(),
            },
            ProjectRun {
                project_id: "same".into(),
                spec: BatchSpec::default(),
                workdir: b.clone(),
            },
        ];
        let err = run_projects(runs, &shared_opts(), &mut |_| {}).unwrap_err();
        assert!(err.to_string().contains("duplicate project id"));
        std::fs::remove_dir_all(&a).ok();
        std::fs::remove_dir_all(&b).ok();
    }
}
