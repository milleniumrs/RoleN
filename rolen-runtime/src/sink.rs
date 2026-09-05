//! Write sink (PRD FR-7.1): the only path from agents to the filesystem.
//! M2 provides `DirectWriteSink` (single agent — atomic temp+rename writes).
//! M3 swaps in the orchestrator's queued, per-path-serialized implementation
//! behind the same trait.

use crate::error::RuntimeError;
use rolen_core::types::{TicketState, WriteOp, WriteTicket};
use std::path::{Component, Path, PathBuf};

pub trait WriteSink: Send + Sync {
    fn apply(&self, ticket: &WriteTicket) -> Result<TicketState, RuntimeError>;
}

/// Sanitise an agent-supplied path into a safe *relative* path.
///
/// Returns None when the path is absolute/drive-qualified or climbs above its
/// own root via `..`. This is a purely lexical check — it does not depend on
/// which components happen to exist, so `sub/../../evil.txt` is rejected even
/// when `sub` was never created.
fn sanitize_relative(path: &str) -> Option<PathBuf> {
    let p = Path::new(path);
    if p.is_absolute() || p.has_root() {
        return None;
    }
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            // on verbatim paths Rust yields ".." as a Normal component
            Component::Normal(s) if s == ".." => {
                if !out.pop() {
                    return None;
                }
            }
            Component::Normal(s) => out.push(s),
            Component::Prefix(_) | Component::RootDir => return None,
        }
    }
    Some(out)
}

/// Resolve `path` inside `root`, rejecting escapes (sandbox, FR-12.2).
///
/// Separators are normalised first: agents routinely emit Windows-style paths
/// (`src\main.rs`, `..\evil.txt`). Without this, a backslash is just a legal
/// filename character on Unix, so such a path would silently create a bizarre
/// file instead of a nested one — and a `..\` traversal would not be seen as
/// one. After the lexical check, existing paths are canonicalised so symlinks
/// cannot point outside the workspace either.
pub fn resolve_in(root: &Path, path: &str) -> Result<PathBuf, RuntimeError> {
    let escape = || {
        RuntimeError::Sandbox(format!(
            "path '{path}' escapes the workdir {}",
            root.display()
        ))
    };
    let normalized = path.replace('\\', "/");
    let relative = sanitize_relative(&normalized).ok_or_else(escape)?;
    let root_c = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let target = root_c.join(relative);

    if target.exists() {
        let real = target.canonicalize().map_err(RuntimeError::Io)?;
        if !real.starts_with(&root_c) {
            return Err(escape());
        }
        return Ok(real);
    }
    Ok(target)
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
                let tmp = path.with_extension("rolen-tmp");
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
                // FR-7.9: unified-diff ticket with fuzzy 3-way context match
                let old = std::fs::read_to_string(&path).unwrap_or_default();
                let Some(new) = rolen_core::patch::apply_patch(&old, &ticket.payload) else {
                    return Ok(TicketState::Rejected);
                };
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let tmp = path.with_extension("rolen-tmp");
                std::fs::write(&tmp, new)?;
                std::fs::rename(&tmp, &path)?;
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
        let dir = std::env::temp_dir().join(format!("rolen-test-write-{}", std::process::id()));
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
        let dir = std::env::temp_dir().join(format!("rolen-test-escape-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sink = DirectWriteSink::new(dir.clone());

        // traversal, both separator styles, on every platform
        assert!(sink.apply(&ticket("../evil.txt", "x")).is_err());
        assert!(sink.apply(&ticket("..\\evil.txt", "x")).is_err());
        assert!(sink.apply(&ticket("sub/../../evil.txt", "x")).is_err());

        // absolute paths must never escape the workspace either
        #[cfg(unix)]
        assert!(sink.apply(&ticket("/tmp/evil.txt", "x")).is_err());
        #[cfg(windows)]
        assert!(sink
            .apply(&ticket("C:\\Windows\\Temp\\evil.txt", "x"))
            .is_err());

        // nothing leaked outside the sandbox
        assert!(!dir.parent().unwrap().join("evil.txt").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn windows_style_separators_create_nested_paths() {
        // agents often emit `src\main.rs`; that must land in src/main.rs,
        // not in a file whose name contains a backslash
        let dir = std::env::temp_dir().join(format!("rolen-test-seps-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sink = DirectWriteSink::new(dir.clone());

        assert_eq!(
            sink.apply(&ticket("src\\main.rs", "fn main() {}")).unwrap(),
            TicketState::Applied
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("src").join("main.rs")).unwrap(),
            "fn main() {}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Pure resolver checks: no filesystem state involved, so they behave
    /// identically on every platform (the CI failure that motivated them was
    /// a Unix-only backslash issue, and `sub/../../x` only "passed" on
    /// Windows because the OS rejected a malformed verbatim path).
    #[test]
    fn sanitize_relative_rejects_escapes() {
        for bad in [
            "../evil.txt",
            "..\\evil.txt",
            "sub/../../evil.txt",
            "a/b/../../../c.txt",
            "..",
            "/etc/passwd",
            "\\\\server\\share\\x",
        ] {
            let normalized = bad.replace('\\', "/");
            assert!(
                sanitize_relative(&normalized).is_none(),
                "should have been rejected: {bad}"
            );
        }
    }

    #[test]
    fn sanitize_relative_keeps_legitimate_paths() {
        let cases = [
            ("file.txt", "file.txt"),
            ("./file.txt", "file.txt"),
            ("src/main.rs", "src/main.rs"),
            ("src\\main.rs", "src/main.rs"),
            ("a/b/../c.txt", "a/c.txt"),
            ("deep/nested/dir/x.md", "deep/nested/dir/x.md"),
        ];
        for (input, expected) in cases {
            let normalized = input.replace('\\', "/");
            let got = sanitize_relative(&normalized).expect(input);
            let expected: PathBuf = expected.split('/').collect();
            assert_eq!(got, expected, "input: {input}");
        }
    }
}
