//! Git checkpoints (PRD FR-7.7): after each completed task, if the workspace
//! is a git repo, commit everything as a checkpoint for one-click revert.

/// Initialize a repo if the workdir isn't one yet. Returns true if git is
/// usable here.
pub fn ensure_repo(workdir: &std::path::Path) -> bool {
    if workdir.join(".git").exists() {
        return true;
    }
    std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(workdir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Commit all changes as a task checkpoint. No-op (Ok) when there's nothing
/// to commit or git is unavailable.
pub fn checkpoint(workdir: &std::path::Path, task_id: &str, title: &str) -> Result<(), String> {
    let run = |args: &[&str]| -> Result<std::process::Output, String> {
        std::process::Command::new("git")
            .args(args)
            .current_dir(workdir)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .output()
            .map_err(|e| format!("git spawn failed: {e}"))
    };
    run(&["add", "-A"])?;
    // nothing staged? skip commit
    let diff = run(&["diff", "--cached", "--quiet"])?;
    if diff.status.success() {
        return Ok(());
    }
    let msg = format!("maestro: checkpoint after task {task_id} — {title}");
    let out = run(&["commit", "-q", "-m", &msg, "--no-gpg-sign"])?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "git commit failed: {}",
            String::from_utf8_lossy(&out.stderr)
                .chars()
                .take(200)
                .collect::<String>()
        ))
    }
}
