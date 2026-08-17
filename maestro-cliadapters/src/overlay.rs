//! Overlay + harvest (D3, FR-13.2): the CLI agent runs against a staging
//! copy of the workspace; afterwards the staging↔base diff becomes write
//! tickets applied through the orchestrator queue — the single-writer
//! guarantee (FR-7.1) holds for CLI providers too.

use crate::error::AdapterError;
use maestro_core::types::{TicketState, WriteOp, WriteTicket};
use maestro_orchestrator::WriteQueue;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const SKIP_DIRS: &[&str] = &[".git", "target", "node_modules", ".maestro-overlay"];

pub struct Harvest {
    pub applied: usize,
    pub rejected: usize,
    pub paths: Vec<String>,
}

/// Create a staging copy of `workdir` (skipping heavy/irrelevant dirs).
pub fn create_staging(workdir: &Path) -> Result<PathBuf, AdapterError> {
    let staging = std::env::temp_dir().join(format!(
        "maestro-overlay-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp()
    ));
    copy_dir(workdir, &staging)?;
    Ok(staging)
}

fn copy_dir(src: &Path, dst: &Path) -> Result<(), AdapterError> {
    std::fs::create_dir_all(dst)?;
    for e in std::fs::read_dir(src)?.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if SKIP_DIRS.contains(&name.as_str()) {
            continue;
        }
        let s = e.path();
        let d = dst.join(&name);
        if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            copy_dir(&s, &d)?;
        } else if e.file_type().map(|t| t.is_file()).unwrap_or(false) {
            std::fs::copy(&s, &d)?;
        }
    }
    Ok(())
}

enum Change {
    Write { rel: PathBuf, content: Vec<u8> },
    Delete { rel: PathBuf },
}

/// Harvest: diff staging against base and apply through the write queue.
pub fn harvest(
    base: &Path,
    staging: &Path,
    queue: &Arc<WriteQueue>,
    task_id: &str,
) -> Result<Harvest, AdapterError> {
    let mut changes: Vec<Change> = Vec::new();
    collect_writes(base, staging, staging, &mut changes)?;
    collect_deletes(base, staging, base, &mut changes)?;

    let mut applied = 0;
    let mut rejected = 0;
    let mut paths = Vec::new();
    for ch in changes {
        let (rel, op, payload, base_hash) = match &ch {
            Change::Write { rel, content } => {
                let base_file = base.join(rel);
                let base_hash = if base_file.exists() {
                    maestro_orchestrator::queue::hash_file(base, &rel.to_string_lossy())
                } else {
                    None
                };
                let op = if base_file.exists() {
                    WriteOp::Replace
                } else {
                    WriteOp::Create
                };
                (
                    rel.clone(),
                    op,
                    String::from_utf8_lossy(content).to_string(),
                    base_hash,
                )
            }
            Change::Delete { rel } => (rel.clone(), WriteOp::Delete, String::new(), None),
        };
        let ticket = WriteTicket {
            id: format!("cli-{}", uuidish()),
            task_id: task_id.to_string(),
            path: rel.clone(),
            op,
            payload,
            base_hash,
            state: TicketState::Queued,
            ts: chrono::Utc::now(),
        };
        let h = queue.submit(ticket);
        paths.push(rel.display().to_string());
        match h.wait() {
            TicketState::Applied => applied += 1,
            TicketState::Rejected => rejected += 1,
            TicketState::Queued => {}
        }
    }
    Ok(Harvest {
        applied,
        rejected,
        paths,
    })
}

pub fn cleanup(staging: &Path) {
    let _ = std::fs::remove_dir_all(staging);
}

fn collect_writes(
    base: &Path,
    root: &Path,
    dir: &Path,
    out: &mut Vec<Change>,
) -> Result<(), AdapterError> {
    for e in std::fs::read_dir(dir)?.flatten() {
        let p = e.path();
        if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            collect_writes(base, root, &p, out)?;
        } else {
            let rel = p.strip_prefix(root).unwrap().to_path_buf();
            let base_file = base.join(&rel);
            let content = std::fs::read(&p)?;
            let differs = match std::fs::read(&base_file) {
                Ok(existing) => existing != content,
                Err(_) => true,
            };
            if differs {
                out.push(Change::Write { rel, content });
            }
        }
    }
    Ok(())
}

fn collect_deletes(
    base: &Path,
    staging: &Path,
    dir: &Path,
    out: &mut Vec<Change>,
) -> Result<(), AdapterError> {
    for e in std::fs::read_dir(dir)?.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if SKIP_DIRS.contains(&name.as_str()) {
            continue;
        }
        if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            collect_deletes(base, staging, &p, out)?;
        } else {
            let rel = p.strip_prefix(base).unwrap().to_path_buf();
            if !staging.join(&rel).exists() {
                out.push(Change::Delete { rel });
            }
        }
    }
    Ok(())
}

fn uuidish() -> String {
    format!("{}", chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harvest_applies_diff_via_queue() {
        let tag = format!("{}-{}", std::process::id(), "harvest");
        let base = std::env::temp_dir().join(format!("maestro-ovl-base-{tag}"));
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("keep.txt"), "same").unwrap();
        std::fs::write(base.join("change.txt"), "old").unwrap();
        std::fs::write(base.join("delete.txt"), "gone").unwrap();

        let staging = create_staging(&base).unwrap();
        std::fs::write(staging.join("change.txt"), "new").unwrap(); // modified
        std::fs::write(staging.join("new.txt"), "fresh").unwrap(); // created
        std::fs::remove_file(staging.join("delete.txt")).unwrap(); // deleted

        let queue = WriteQueue::new(base.clone());
        let h = harvest(&base, &staging, &queue, "test-task").unwrap();
        queue.shutdown();

        assert_eq!(h.rejected, 0, "rejected: {:?}", h.paths);
        assert_eq!(h.applied, 3);
        assert_eq!(
            std::fs::read_to_string(base.join("change.txt")).unwrap(),
            "new"
        );
        assert_eq!(
            std::fs::read_to_string(base.join("new.txt")).unwrap(),
            "fresh"
        );
        assert!(!base.join("delete.txt").exists());
        assert_eq!(
            std::fs::read_to_string(base.join("keep.txt")).unwrap(),
            "same"
        );

        cleanup(&staging);
        std::fs::remove_dir_all(&base).ok();
    }
}
