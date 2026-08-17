//! Write sink (PRD FR-7.1): the only path from agents to the filesystem.
//! M2 provides `DirectWriteSink` (single agent — atomic temp+rename writes).
//! M3 swaps in the orchestrator's queued, per-path-serialized implementation
//! behind the same trait.

use crate::error::RuntimeError;
use maestro_core::types::{TicketState, WriteOp, WriteTicket};
use std::path::{Path, PathBuf};

pub trait WriteSink: Send + Sync {
    fn apply(&self, ticket: &WriteTicket) -> Result<TicketState, RuntimeError>;
}

/// Resolve `path` inside `root`, rejecting escapes (sandbox, FR-12.2).
pub fn resolve_in(root: &Path, path: &str) -> Result<PathBuf, RuntimeError> {
    let root_c = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let candidate = root_c.join(path);
    let check = if candidate.exists() {
        candidate.canonicalize().map_err(RuntimeError::Io)?
    } else {
        // New file: canonicalize the deepest existing ancestor and re-append
        // the missing tail components.
        let mut existing = candidate.clone();
        let mut tail: Vec<std::ffi::OsString> = Vec::new();
        while !existing.exists() {
            match existing.file_name() {
                Some(name) => {
                    tail.push(name.to_os_string());
                    existing.pop();
                }
                None => break,
            }
        }
        let mut base = existing.canonicalize().unwrap_or(existing);
        for comp in tail.iter().rev() {
            base.push(comp);
        }
        base
    };
    if !check.starts_with(&root_c) {
        return Err(RuntimeError::Sandbox(format!(
            "path '{path}' escapes the workdir {}",
            root.display()
        )));
    }
    Ok(check)
}

pub struct DirectWriteSink {
    pub root: PathBuf,
}

impl DirectWriteSink {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

impl WriteSink for DirectWriteSink {
    fn apply(&self, ticket: &WriteTicket) -> Result<TicketState, RuntimeError> {
        let path = resolve_in(&self.root, &ticket.path.to_string_lossy())?;
        match ticket.op {
            WriteOp::Create | WriteOp::Replace => {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                // atomic: temp file in the same dir + rename
                let tmp = path.with_extension("maestro-tmp");
                std::fs::write(&tmp, &ticket.payload)?;
                std::fs::rename(&tmp, &path)?;
            }
            WriteOp::Delete => {
                if path.exists() {
                    std::fs::remove_file(&path)?;
                }
            }
            WriteOp::Rename => {
                let target = resolve_in(&self.root, &ticket.payload)?;
                std::fs::rename(&path, &target)?;
            }
            WriteOp::Patch => {
                // full 3-way patch tickets are P1 (FR-7.9); for M2 the agent
                // is told to send full content instead
                return Err(RuntimeError::Sandbox(
                    "patch tickets not supported yet — submit full content (replace)".into(),
                ));
            }
        }
        Ok(TicketState::Applied)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn ticket(path: &str, payload: &str) -> WriteTicket {
        WriteTicket {
            id: "t1".into(),
            task_id: "test".into(),
            path: path.into(),
            op: WriteOp::Replace,
            payload: payload.into(),
            base_hash: None,
            state: TicketState::Queued,
            ts: Utc::now(),
        }
    }

    #[test]
    fn writes_inside_root() {
        let dir = std::env::temp_dir().join(format!("maestro-test-write-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sink = DirectWriteSink::new(dir.clone());
        let state = sink.apply(&ticket("sub/hello.txt", "hi")).unwrap();
        assert_eq!(state, TicketState::Applied);
        assert_eq!(
            std::fs::read_to_string(dir.join("sub/hello.txt")).unwrap(),
            "hi"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rejects_escape() {
        let dir = std::env::temp_dir().join(format!("maestro-test-escape-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sink = DirectWriteSink::new(dir.clone());
        assert!(sink.apply(&ticket("../evil.txt", "x")).is_err());
        assert!(sink.apply(&ticket("..\\evil.txt", "x")).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
