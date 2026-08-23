//! CLI agent sessions (PRD FR-13.1): PTY spawn, streamed transcript, ledger,
//! overlay harvest through the write queue.

use crate::error::AdapterError;
use crate::overlay;
use crate::pty;
use crate::spec::CliSpec;
use rolen_core::ledger::Ledger;
use rolen_core::types::{LedgerEntry, Provider, Session, SessionState};
use rolen_orchestrator::WriteQueue;
use std::path::Path;
use std::sync::Arc;

pub enum CliEvent {
    Output(String),
    Harvested {
        applied: usize,
        rejected: usize,
        paths: Vec<String>,
    },
}

pub struct CliSessionReport {
    pub session_id: String,
    pub exit_code: Option<i32>,
    pub applied: usize,
    pub rejected: usize,
    pub paths: Vec<String>,
    pub transcript_path: std::path::PathBuf,
    pub tokens_in_est: u64,
    pub tokens_out_est: u64,
}

/// Run a wrapped CLI session (FR-13.1/13.2):
/// overlay copy → PTY run inside the overlay → diff → tickets → queue.
pub fn run_cli_session(
    provider: &Provider,
    task: &str,
    workdir: &Path,
    queue: Option<Arc<WriteQueue>>,
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

    let prompt = format!(
        "{task}\n\n[RoleN] Work inside the current directory. Create/modify files directly; \
         your changes are reviewed and applied by the RoleN orchestrator afterwards."
    );
    let argv = spec.argv(&prompt);

    // overlay (D3)
    let staging = overlay::create_staging(workdir)?;
    let mut transcript = String::new();
    let pty_result = pty::run_pty(&spec.program, &argv, &staging, &mut |chunk| {
        transcript.push_str(chunk);
        on_event(CliEvent::Output(chunk.to_string()));
    });

    let _ = std::fs::write(&transcript_path, &transcript);

    // harvest writes back through the queue (single-writer guarantee)
    let queue = queue.unwrap_or_else(|| WriteQueue::new(workdir.to_path_buf()));
    let harvest = overlay::harvest(workdir, &staging, &queue, &session_id)?;
    overlay::cleanup(&staging);
    on_event(CliEvent::Harvested {
        applied: harvest.applied,
        rejected: harvest.rejected,
        paths: harvest.paths.clone(),
    });

    // token estimation (FR-4.2: chars/4 until the CLI exposes real usage)
    let tokens_in_est = (prompt.len() / 4) as u64;
    let tokens_out_est = (transcript.len() / 4) as u64;
    let exit_code = pty_result.ok().and_then(|r| r.exit_code);

    session.state = if exit_code == Some(0) {
        SessionState::Done
    } else {
        SessionState::Failed
    };
    session.tokens_in = tokens_in_est;
    session.tokens_out = tokens_out_est;
    session.transcript_path = Some(transcript_path.clone());
    ledger.upsert_session(&session)?;

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
        applied: harvest.applied,
        rejected: harvest.rejected,
        paths: harvest.paths,
        transcript_path,
        tokens_in_est,
        tokens_out_est,
    })
}
