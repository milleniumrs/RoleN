//! Sandboxed standard tools (PRD FR-12.2). Deliberately NO write_file tool —
//! agents emit write tickets via `submit_write` only.

use crate::error::RuntimeError;
use crate::sink::{resolve_in, WriteSink};
use rolen_core::types::{TicketState, WriteOp, WriteTicket};
use rolen_providers::chat::{ToolCall, ToolOutcome, ToolSpec};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const READ_CAP: usize = 100 * 1024;
const OUTPUT_CAP: usize = 20 * 1024;
const SEARCH_CAP: usize = 200;
const SHELL_TIMEOUT: Duration = Duration::from_secs(60);

pub struct ToolContext {
    pub workdir: PathBuf,
    /// Allow-listed program names for run_shell (empty = shell disabled).
    pub shell_allow: Vec<String>,
    /// The only write path: direct atomic write (M2) or orchestrator queue (M3).
    pub sink: Box<dyn WriteSink>,
    /// Task id stamped onto tickets.
    pub task_id: String,
}

pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "read_file".into(),
            description: "Read a text file inside the workspace. Args: path (relative).".into(),
            parameters: json!({
                "type": "object",
                "properties": {"path": {"type": "string", "description": "relative path"}},
                "required": ["path"]
            }),
        },
        ToolSpec {
            name: "list_dir".into(),
            description: "List files/dirs at a relative path inside the workspace.".into(),
            parameters: json!({
                "type": "object",
                "properties": {"path": {"type": "string", "description": "relative path, empty = root"}},
                "required": ["path"]
            }),
        },
        ToolSpec {
            name: "search".into(),
            description: "Substring search across workspace text files. Returns matching lines with paths.".into(),
            parameters: json!({
                "type": "object",
                "properties": {"pattern": {"type": "string"}},
                "required": ["pattern"]
            }),
        },
        ToolSpec {
            name: "run_shell".into(),
            description: "Run an allow-listed program in the workspace. Args: command (single line).".into(),
            parameters: json!({
                "type": "object",
                "properties": {"command": {"type": "string"}},
                "required": ["command"]
            }),
        },
        ToolSpec {
            name: "submit_write".into(),
            description: "The ONLY way to create/modify/delete files. Sends a write ticket to the orchestrator. Args: path, content, op (create|replace|delete).".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "content": {"type": "string"},
                    "op": {"type": "string", "enum": ["create", "replace", "delete"]}
                },
                "required": ["path", "op"]
            }),
        },
        ToolSpec {
            name: "ask_user".into(),
            description: "Ask the human a clarifying question when requirements are ambiguous.".into(),
            parameters: json!({
                "type": "object",
                "properties": {"question": {"type": "string"}},
                "required": ["question"]
            }),
        },
    ]
}

pub fn execute(ctx: &ToolContext, call: &ToolCall) -> ToolOutcome {
    match run(ctx, call) {
        Ok(content) => ToolOutcome {
            id: call.id.clone(),
            name: call.name.clone(),
            content,
            is_error: false,
        },
        Err(e) => ToolOutcome {
            id: call.id.clone(),
            name: call.name.clone(),
            content: e.to_string(),
            is_error: true,
        },
    }
}

fn run(ctx: &ToolContext, call: &ToolCall) -> Result<String, RuntimeError> {
    match call.name.as_str() {
        "read_file" => {
            let path = resolve_in(&ctx.workdir, &arg_str(&call.args, "path")?)?;
            let bytes = std::fs::read(&path)?;
            let text = String::from_utf8_lossy(&bytes);
            Ok(cap(
                text.chars().take(READ_CAP).collect(),
                " (truncated at 100KB)",
            ))
        }
        "list_dir" => {
            let rel = arg_str(&call.args, "path").unwrap_or_default();
            let path = resolve_in(&ctx.workdir, if rel.is_empty() { "." } else { &rel })?;
            let mut entries = Vec::new();
            for e in std::fs::read_dir(&path)? {
                let e = e?;
                let suffix = if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    "/"
                } else {
                    ""
                };
                entries.push(format!("{}{}", e.file_name().to_string_lossy(), suffix));
            }
            entries.sort();
            Ok(cap(entries.join("\n"), "\n… (truncated)"))
        }
        "search" => {
            let pattern = arg_str(&call.args, "pattern")?;
            let mut hits = Vec::new();
            walk(
                &ctx.workdir,
                &ctx.workdir,
                &pattern.to_lowercase(),
                &mut hits,
                0,
            )?;
            if hits.is_empty() {
                Ok("no matches".into())
            } else {
                Ok(hits.join("\n"))
            }
        }
        "run_shell" => {
            let command = arg_str(&call.args, "command")?;
            run_shell(ctx, &command)
        }
        "submit_write" => {
            let path = arg_str(&call.args, "path")?;
            let op = match arg_str(&call.args, "op")?.as_str() {
                "create" => WriteOp::Create,
                "delete" => WriteOp::Delete,
                _ => WriteOp::Replace,
            };
            let content = arg_str(&call.args, "content").unwrap_or_default();
            let ticket = WriteTicket {
                id: format!(
                    "t-{}",
                    chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
                ),
                task_id: ctx.task_id.clone(),
                path: path.clone().into(),
                op,
                payload: content,
                base_hash: None,
                state: TicketState::Queued,
                ts: chrono::Utc::now(),
            };
            match ctx.sink.apply(&ticket)? {
                TicketState::Applied => Ok(format!("ticket {} applied: {}", ticket.id, path)),
                TicketState::Rejected => Err(RuntimeError::Sandbox(format!(
                    "ticket for {path} was rejected (stale base?)"
                ))),
                TicketState::Queued => unreachable!(),
            }
        }
        "ask_user" => {
            let q = arg_str(&call.args, "question")?;
            // Headless (M2): the interrogation queue UI arrives in M4.
            Ok(format!(
                "Question recorded for the user: \"{q}\". No interactive answer available in this run — proceed with a reasonable assumption and document it."
            ))
        }
        other => Err(RuntimeError::Sandbox(format!("unknown tool '{other}'"))),
    }
}

fn run_shell(ctx: &ToolContext, command: &str) -> Result<String, RuntimeError> {
    let mut parts = command.split_whitespace();
    let program = parts.next().unwrap_or("");
    let prog_name = PathBuf::from(program)
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let allowed = ctx
        .shell_allow
        .iter()
        .any(|a| a.trim_start_matches('*').eq_ignore_ascii_case(&prog_name));
    if !allowed {
        return Err(RuntimeError::Sandbox(format!(
            "program '{prog_name}' is not in the shell allow-list {:?}",
            ctx.shell_allow
        )));
    }
    let args: Vec<&str> = command.split_whitespace().skip(1).collect();
    let mut child = std::process::Command::new(program)
        .args(&args)
        .current_dir(&ctx.workdir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;

    let deadline = Instant::now() + SHELL_TIMEOUT;
    loop {
        if let Some(status) = child.try_wait()? {
            let out = child.wait_with_output()?;
            let mut text = String::new();
            text.push_str(&String::from_utf8_lossy(&out.stdout));
            if !out.stderr.is_empty() {
                text.push_str("\n[stderr]\n");
                text.push_str(&String::from_utf8_lossy(&out.stderr));
            }
            return Ok(format!(
                "exit: {}\n{}",
                status,
                cap(text, "\n… (truncated at 20KB)")
            ));
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            return Err(RuntimeError::Sandbox("command timed out after 60s".into()));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn walk(
    root: &PathBuf,
    dir: &PathBuf,
    pattern: &str,
    hits: &mut Vec<String>,
    depth: u32,
) -> Result<(), RuntimeError> {
    if depth > 8 || hits.len() >= SEARCH_CAP {
        return Ok(());
    }
    for e in std::fs::read_dir(dir)? {
        let e = e?;
        let path = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || name == "target" || name == "node_modules" {
            continue;
        }
        if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            walk(root, &path, pattern, hits, depth + 1)?;
        } else if let Ok(bytes) = std::fs::read(&path) {
            if bytes.contains(&0) {
                continue; // binary
            }
            let text = String::from_utf8_lossy(&bytes).to_lowercase();
            for (i, line) in text.lines().enumerate() {
                if line.contains(pattern) {
                    let rel = path.strip_prefix(root).unwrap_or(&path);
                    hits.push(format!("{}:{}: {}", rel.display(), i + 1, line.trim()));
                    if hits.len() >= SEARCH_CAP {
                        return Ok(());
                    }
                }
            }
        }
    }
    Ok(())
}

fn arg_str(args: &Value, key: &str) -> Result<String, RuntimeError> {
    Ok(args
        .get(key)
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_default())
}

fn cap(s: String, suffix: &str) -> String {
    if s.len() > OUTPUT_CAP {
        format!("{}{}", &s[..OUTPUT_CAP], suffix)
    } else {
        s
    }
}
