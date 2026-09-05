//! Dependency-aware parallel scheduler (PRD FR-8).
//!
//! Runs a batch of tasks as a DAG: a task starts when all its deps succeeded
//! AND its claimed_paths don't overlap any running task (FR-7.5) AND the
//! parallelism cap allows it (FR-8.1). Every agent writes through the shared
//! WriteQueue (FR-7.1); each completed task produces a git checkpoint (FR-7.7).

use crate::git;
use crate::queue::{QueuedWriteSink, WriteQueue};
use rolen_runtime::agent::{self, AgentEvent, AgentOptions, RunReport};
use rolen_runtime::error::RuntimeError;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
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

#[derive(Debug, Default, Serialize, Deserialize)]
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

pub enum BatchEvent {
    TaskStarted {
        id: String,
        role: String,
    },
    Agent {
        id: String,
        line: String,
    },
    TaskDone {
        id: String,
        tokens: u64,
        steps: usize,
    },
    TaskFailed {
        id: String,
        error: String,
    },
    Waiting {
        id: String,
        reason: String,
    },
    AllDone {
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

pub fn run_batch(
    spec: &BatchSpec,
    opts: &BatchOptions,
    on_event: &mut dyn FnMut(BatchEvent),
) -> Result<BatchReport, SchedError> {
    validate_dag(spec)?;
    std::fs::create_dir_all(&opts.workdir)?;
    let git_ok = git::ensure_repo(&opts.workdir);

    let queue = WriteQueue::with_capacity(
        opts.workdir.clone(),
        rolen_core::config::Config::load()
            .map(|c| c.parallelism.queue_cap)
            .unwrap_or(0),
    );
    let max_parallel = if opts.max_parallel > 0 {
        opts.max_parallel
    } else {
        rolen_core::config::Config::load()
            .map(|c| c.parallelism.effective_global_cap())
            .unwrap_or(4)
    };

    let mut status: HashMap<String, TaskStatus> = spec
        .tasks
        .iter()
        .map(|t| (t.id.clone(), TaskStatus::Pending))
        .collect();
    let mut claimed: HashMap<String, String> = HashMap::new(); // path -> task id
    let results: TaskResults = Arc::new(Mutex::new(Vec::new()));
    let finished: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
    let cancel = opts
        .cancel
        .clone()
        .unwrap_or_else(|| Arc::new(AtomicBool::new(false)));

    let mut handles: Vec<std::thread::JoinHandle<()>> = Vec::new();
    let mut done_reports: Vec<(String, RunReport)> = Vec::new();
    let mut failed: Vec<(String, String)> = Vec::new();
    // FR-6.3: tasks depending on a task with unanswered questions are paused
    // until the user answers (interrogation center / TUI Questions tab).
    let project_dir = rolen_core::project::find_project_dir_upwards(&opts.workdir);
    let mut pending_questions: HashSet<String> = HashSet::new();
    let mut question_wait_reported: HashSet<String> = HashSet::new();
    let mut poll_countdown = 0u32; // re-read the project file ~once per second
                                   // agent output lines travel through this channel so the event consumer
                                   // decides how to present them (human text vs NDJSON stream)
    let (agent_tx, agent_rx) = std::sync::mpsc::channel::<(String, String)>();

    loop {
        // forward agent output lines
        while let Ok((id, line)) = agent_rx.try_recv() {
            on_event(BatchEvent::Agent { id, line });
        }
        // harvest finished tasks
        let just_finished: Vec<String> = finished.lock().unwrap().drain().collect();
        for id in just_finished {
            // release claims
            claimed.retain(|_, owner| owner != &id);
            let res = results
                .lock()
                .unwrap()
                .drain(..)
                .find(|(rid, _)| rid == &id);
            let task = spec.tasks.iter().find(|t| t.id == id).unwrap();
            match res {
                Some((_, Ok(report))) => {
                    // Honesty check: a task that claims files must have
                    // produced them — a model that only narrated is a failure.
                    let missing: Vec<&String> = task
                        .claimed_paths
                        .iter()
                        .filter(|p| !opts.workdir.join(p).exists())
                        .collect();
                    if missing.is_empty() {
                        status.insert(id.clone(), TaskStatus::Done);
                        if git_ok {
                            let _ = git::checkpoint(&opts.workdir, &id, &task.title);
                        }
                        on_event(BatchEvent::TaskDone {
                            id: id.clone(),
                            tokens: report.tokens_in + report.tokens_out,
                            steps: report.steps,
                        });
                        done_reports.push((id.clone(), report));
                    } else {
                        let err = format!(
                            "agent finished but claimed file(s) missing: {}",
                            missing
                                .iter()
                                .map(|s| s.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        );
                        status.insert(id.clone(), TaskStatus::Failed);
                        on_event(BatchEvent::TaskFailed {
                            id: id.clone(),
                            error: err.clone(),
                        });
                        failed.push((id.clone(), err));
                    }
                }
                Some((_, Err(e))) => {
                    status.insert(id.clone(), TaskStatus::Failed);
                    on_event(BatchEvent::TaskFailed {
                        id: id.clone(),
                        error: e.to_string(),
                    });
                    failed.push((id.clone(), e.to_string()));
                }
                None => {
                    status.insert(id.clone(), TaskStatus::Failed);
                    failed.push((id.clone(), "worker vanished".into()));
                }
            }
        }

        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if status
            .values()
            .all(|s| !matches!(s, TaskStatus::Pending | TaskStatus::Running))
        {
            break;
        }

        // find launchable tasks
        if let Some(dir) = &project_dir {
            if poll_countdown == 0 {
                pending_questions = rolen_core::project::pending_question_task_ids(dir);
                poll_countdown = 10;
            }
            poll_countdown -= 1;
        }
        let running_count = status
            .values()
            .filter(|s| matches!(s, TaskStatus::Running))
            .count();
        let mut slots = max_parallel.saturating_sub(running_count);
        if slots > 0 {
            for task in &spec.tasks {
                if slots == 0 {
                    break;
                }
                if !matches!(status[&task.id], TaskStatus::Pending) {
                    continue;
                }
                // deps must all be done; a failed dep blocks permanently
                let deps: Vec<&TaskStatus> = task.deps.iter().map(|d| &status[d]).collect();
                if deps.iter().any(|s| matches!(s, TaskStatus::Failed)) {
                    status.insert(task.id.clone(), TaskStatus::Failed);
                    on_event(BatchEvent::TaskFailed {
                        id: task.id.clone(),
                        error: "dependency failed".into(),
                    });
                    failed.push((task.id.clone(), "dependency failed".into()));
                    continue;
                }
                if !deps.iter().all(|s| matches!(s, TaskStatus::Done)) {
                    continue;
                }
                // FR-6.3: pause while a dependency has an unanswered question
                if blocked_by_question(spec, &task.id, &pending_questions) {
                    if question_wait_reported.insert(task.id.clone()) {
                        on_event(BatchEvent::Waiting {
                            id: task.id.clone(),
                            reason: "awaiting answer to a clarification question \
                                     (see the TUI Questions tab)"
                                .into(),
                        });
                    }
                    continue;
                }
                question_wait_reported.remove(&task.id);
                // path claims (FR-7.5)
                if task.claimed_paths.iter().any(|p| claimed.contains_key(p)) {
                    on_event(BatchEvent::Waiting {
                        id: task.id.clone(),
                        reason: "path claim overlap with a running task".into(),
                    });
                    continue;
                }

                for p in &task.claimed_paths {
                    claimed.insert(p.clone(), task.id.clone());
                }
                status.insert(task.id.clone(), TaskStatus::Running);
                slots -= 1;
                on_event(BatchEvent::TaskStarted {
                    id: task.id.clone(),
                    role: task.role.clone(),
                });

                // spawn the agent thread
                let task = task.clone();
                let workdir = opts.workdir.clone();
                let shell_allow = opts.shell_allow.clone();
                let sink = QueuedWriteSink::new(queue.clone());
                let results = results.clone();
                let finished = finished.clone();
                let cancel = cancel.clone();
                let pause = opts.pause.clone();
                let agent_tx = agent_tx.clone();
                handles.push(std::thread::spawn(move || {
                    let prefix_id = task.id.clone();
                    let printer = move |line: String| {
                        let _ = agent_tx.send((prefix_id.clone(), line));
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

        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    for h in handles {
        let _ = h.join();
    }
    queue.shutdown();

    // final drain of agent lines before reporting completion
    while let Ok((id, line)) = agent_rx.try_recv() {
        on_event(BatchEvent::Agent { id, line });
    }

    let report = BatchReport {
        done: done_reports,
        failed: failed.clone(),
    };
    on_event(BatchEvent::AllDone {
        done: report.done.len(),
        failed: failed.len(),
    });
    Ok(report)
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
