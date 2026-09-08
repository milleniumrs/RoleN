//! CLI agent sessions (Requirements FR-13.1): PTY spawn, streamed transcript, ledger,
//! overlay harvest through the write queue.
//!
//! FR-8.4: wrapped CLI sessions support graceful cancel (kill the child at a
//! poll boundary, harvest the partial overlay, mark the session interrupted and
//! keep a snapshot) and cooperative pause before spawn. A wrapped CLI's live
//! PTY state cannot be restored portably, so resume means a fresh invocation
//! whose prompt is seeded with the prior task, transcript tail and applied
//! paths.

use crate::error::AdapterError;
use crate::overlay;
use crate::pty;
use crate::spec::CliSpec;
use rolen_core::ledger::Ledger;
use rolen_core::types::{LedgerEntry, Provider, Session, SessionState};
use rolen_orchestrator::WriteQueue;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub enum CliEvent {
    Output(String),
    Paused,
    Resumed,
    Harvested {
        applied: usize,
        rejected: usize,
        paths: Vec<String>,
    },
}

pub struct CliSessionReport {
    pub session_id: String,
    pub exit_code: Option<i32>,
    /// FR-8.4: true when the shared cancel flag interrupted the session; a
    /// context snapshot was kept for resume.
    pub interrupted: bool,
    pub applied: usize,
    pub rejected: usize,
    pub paths: Vec<String>,
    pub transcript_path: PathBuf,
    pub tokens_in_est: u64,
    pub tokens_out_est: u64,
}

/// FR-8.4 controls for wrapped CLI sessions.
#[derive(Default, Clone)]
pub struct CliCheckpointOptions {
    /// Shared cancel flag (Ctrl+C / batch cancel). Checked before spawn and at
    /// PTY poll boundaries.
    pub cancel: Option<Arc<AtomicBool>>,
    /// Cooperative pause. Wrapped CLI sessions are single-shot, so this is
    /// honored before the child is spawned; in-flight pause is equivalent to
    /// interrupt-and-resume.
    pub pause: Option<Arc<AtomicBool>>,
    /// Resume from a snapshot written by an interrupted/paused CLI session.
    pub resume_from: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CliSnapshot {
    kind: String,
    task: String,
    prompt: String,
    transcript_tail: String,
    applied_paths: Vec<String>,
    saved: chrono::DateTime<chrono::Utc>,
}

fn cli_snapshot_path(session_id: &str) -> Result<PathBuf, AdapterError> {
    rolen_runtime::agent::snapshot_path(session_id)
        .ok_or_else(|| AdapterError::Pty("no data dir for cli snapshot".into()))
}

fn write_cli_snapshot(
    session_id: &str,
    task: &str,
    prompt: &str,
    transcript: &str,
    applied_paths: &[String],
) -> Result<PathBuf, AdapterError> {
    let path = cli_snapshot_path(session_id)?;
    let snap = CliSnapshot {
        kind: "cli".into(),
        task: task.to_string(),
        prompt: prompt.to_string(),
        transcript_tail: tail(transcript, 8_000),
        applied_paths: applied_paths.to_vec(),
        saved: chrono::Utc::now(),
    };
    let text = serde_json::to_string_pretty(&snap)
        .map_err(|e| AdapterError::Pty(format!("cli snapshot serialize: {e}")))?;
    std::fs::write(&path, text)?;
    Ok(path)
}

fn load_cli_snapshot(path: &Path) -> Result<CliSnapshot, AdapterError> {
    let text = std::fs::read_to_string(path)?;
    serde_json::from_str(&text)
        .map_err(|e| AdapterError::Pty(format!("cli snapshot {}: {e}", path.display())))
}

fn clear_cli_snapshot(session_id: &str) {
    if let Ok(path) = cli_snapshot_path(session_id) {
        if path.exists() {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn tail(text: &str, max_chars: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    chars
        .iter()
        .skip(chars.len().saturating_sub(max_chars))
        .collect()
}

fn resume_prompt(task: &str, snap: &CliSnapshot) -> String {
    format!(
        "{task}\n\n[RoleN] You previously worked on this task and were interrupted. \
         Continue without redoing work that is already applied.\n\
         Files already applied through the write queue: {}\n\
         Partial transcript tail:\n{}",
        if snap.applied_paths.is_empty() {
            "(none)".to_string()
        } else {
            snap.applied_paths.join(", ")
        },
        snap.transcript_tail
    )
}

fn flag(flag: &Option<Arc<AtomicBool>>) -> bool {
    flag.as_ref()
        .map(|f| f.load(Ordering::Relaxed))
        .unwrap_or(false)
}

/// Backwards-compatible wrapped CLI session (no checkpoint controls).
pub fn run_cli_session(
    provider: &Provider,
    task: &str,
    workdir: &Path,
    queue: Option<Arc<WriteQueue>>,
    on_event: &mut dyn FnMut(CliEvent),
) -> Result<CliSessionReport, AdapterError> {
    run_cli_session_with(provider, task, workdir, queue, None, on_event)
}

/// Run a wrapped CLI session (FR-13.1/13.2) with FR-8.4 checkpoint controls:
/// overlay copy → optional pre-spawn pause → PTY run inside the overlay →
/// diff → tickets → queue. On cancel, the partial overlay is still harvested
/// through the queue and the session is ledgered `interrupted` with a snapshot.
pub fn run_cli_session_with(
    provider: &Provider,
    task: &str,
    workdir: &Path,
    queue: Option<Arc<WriteQueue>>,
    checkpoint: Option<&CliCheckpointOptions>,
    on_event: &mut dyn FnMut(CliEvent),
) -> Result<CliSessionReport, AdapterError> {
    let spec = CliSpec::for_provider(provider)
        .ok_or_else(|| AdapterError::Pty(format!("provider '{}' has no cli_path", provider.id)))?;

    let session_id = format!(
        "cli-{}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let ledger = Ledger::open_default()?;
    let mut session = Session {
        id: session_id.clone(),
        task_id: None,
        provider_id: provider.id.clone(),
        model: "cli".into(),
        role: "cli-agent".into(),
        state: SessionState::Running,
        tokens_in: 0,
        tokens_out: 0,
        cost: 0.0,
        started: chrono::Utc::now(),
        transcript_path: None,
    };
    ledger.upsert_session(&session)?;

    // transcript
    let transcripts = rolen_core::config::data_dir()?.join("transcripts");
    std::fs::create_dir_all(&transcripts)?;
    let transcript_path = transcripts.join(format!("{session_id}.log"));

    let checkpoint = checkpoint.cloned().unwrap_or_default();
    let resumed = checkpoint
        .resume_from
        .as_ref()
        .and_then(|p| load_cli_snapshot(p).ok());
    let base_task = if task.trim().is_empty() {
        resumed.as_ref().map(|s| s.task.clone()).unwrap_or_default()
    } else {
        task.to_string()
    };
    let prompt = match &resumed {
        Some(snap) => resume_prompt(&base_task, snap),
        None => format!(
            "{base_task}\n\n[RoleN] Work inside the current directory. Create/modify files directly; \
             your changes are reviewed and applied by the RoleN orchestrator afterwards."
        ),
    };
    let argv = spec.argv(&prompt);

    // FR-8.4 pause: wrapped CLIs are single-shot, so pause is honored before
    // spawn. The session is ledgered paused and snapshotted while waiting.
    let mut was_paused = false;
    while flag(&checkpoint.pause) && !flag(&checkpoint.cancel) {
        if !was_paused {
            was_paused = true;
            session.state = SessionState::Paused;
            let _ = ledger.upsert_session(&session);
            let _ = write_cli_snapshot(&session_id, &base_task, &prompt, "", &[]);
            on_event(CliEvent::Paused);
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    if was_paused && !flag(&checkpoint.cancel) {
        session.state = SessionState::Running;
        let _ = ledger.upsert_session(&session);
        on_event(CliEvent::Resumed);
    }

    let mut transcript = String::new();
    let mut applied_paths: Vec<String> = Vec::new();
    let mut applied = 0usize;
    let mut rejected = 0usize;
    let mut exit_code: Option<i32> = None;
    let mut interrupted = false;

    if !flag(&checkpoint.cancel) {
        // overlay (D3)
        let staging = overlay::create_staging(workdir)?;
        let pty_result = pty::run_pty_cancellable(
            &spec.program,
            &argv,
            &staging,
            pty::DEFAULT_TIMEOUT,
            checkpoint.cancel.clone(),
            &mut |chunk| {
                transcript.push_str(chunk);
                on_event(CliEvent::Output(chunk.to_string()));
            },
        );
        let _ = std::fs::write(&transcript_path, &transcript);

        // harvest writes back through the queue (single-writer guarantee) even
        // when the child was cancelled — partial work is real workspace state.
        let queue = queue.unwrap_or_else(|| WriteQueue::new(workdir.to_path_buf()));
        let harvest = overlay::harvest(workdir, &staging, &queue, &session_id)?;
        overlay::cleanup(&staging);
        applied = harvest.applied;
        rejected = harvest.rejected;
        applied_paths = harvest.paths.clone();
        on_event(CliEvent::Harvested {
            applied,
            rejected,
            paths: harvest.paths,
        });

        if let Ok(result) = pty_result {
            exit_code = result.exit_code;
            interrupted = result.cancelled;
        }
    } else {
        interrupted = true;
        let _ = std::fs::write(&transcript_path, &transcript);
    }

    // token estimation (FR-4.2: chars/4 until the CLI exposes real usage)
    let tokens_in_est = (prompt.len() / 4) as u64;
    let tokens_out_est = (transcript.len() / 4) as u64;

    session.state = if interrupted {
        SessionState::Interrupted
    } else if exit_code == Some(0) {
        SessionState::Done
    } else {
        SessionState::Failed
    };
    session.tokens_in = tokens_in_est;
    session.tokens_out = tokens_out_est;
    session.transcript_path = Some(transcript_path.clone());
    ledger.upsert_session(&session)?;

    if interrupted {
        // NFR-3 parity with the built-in runtime: interrupted sessions stay
        // recoverable through a kept snapshot.
        let _ = write_cli_snapshot(
            &session_id,
            &base_task,
            &prompt,
            &transcript,
            &applied_paths,
        );
    } else if session.state == SessionState::Done {
        clear_cli_snapshot(&session_id);
    }

    // A wrapped CLI reports no usage block, so the cache buckets stay empty
    // and the token counts are length estimates.
    let usage = rolen_core::pricing::Tokens {
        input: tokens_in_est,
        output: tokens_out_est,
        ..Default::default()
    };
    // Normally 0.0: a CLI agent is a subscription with no per-token rate. It
    // is only non-zero if the user entered their own estimate for this model,
    // and it is an estimate twice over, because the tokens are guessed too.
    let cost = rolen_core::pricing::Pricing::load()
        .unwrap_or_default()
        .resolve(provider.ptype, &provider.id, &session.model)
        .cost(usage);

    ledger.record(&LedgerEntry {
        id: format!(
            "le-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ),
        session_id: session_id.clone(),
        provider_id: provider.id.clone(),
        usage,
        cost,
        latency_ms: None,
        ts: chrono::Utc::now(),
    })?;

    Ok(CliSessionReport {
        session_id,
        exit_code,
        interrupted,
        applied,
        rejected,
        paths: applied_paths,
        transcript_path,
        tokens_in_est,
        tokens_out_est,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap() -> CliSnapshot {
        CliSnapshot {
            kind: "cli".into(),
            task: "build the thing".into(),
            prompt: "build the thing\n\n[RoleN] ...".into(),
            transcript_tail: "…wrote src/main.rs…".into(),
            applied_paths: vec!["src/main.rs".into()],
            saved: chrono::Utc::now(),
        }
    }

    #[test]
    fn cli_snapshot_roundtrips_as_json() {
        let snap = snap();
        let text = serde_json::to_string_pretty(&snap).unwrap();
        let back: CliSnapshot = serde_json::from_str(&text).unwrap();
        assert_eq!(back.kind, "cli");
        assert_eq!(back.task, "build the thing");
        assert_eq!(back.applied_paths, vec!["src/main.rs".to_string()]);
    }

    #[test]
    fn resume_prompt_carries_task_paths_and_transcript_tail() {
        let prompt = resume_prompt("build the thing", &snap());
        assert!(prompt.contains("build the thing"));
        assert!(prompt.contains("src/main.rs"));
        assert!(prompt.contains("…wrote src/main.rs…"));
        assert!(prompt.contains("interrupted"));
    }

    #[test]
    fn tail_keeps_the_last_characters() {
        assert_eq!(tail("abcdef", 3), "def");
        assert_eq!(tail("abc", 10), "abc");
    }
}
