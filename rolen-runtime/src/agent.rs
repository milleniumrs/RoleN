//! The agent loop (PRD FR-12.1): chat with tool calls until done.
//! Every LLM call is ledgered (FR-4.6); every file write goes through the
//! WriteSink as a ticket (FR-7.1).

use crate::error::RuntimeError;
use crate::sink::DirectWriteSink;
use crate::tools::{self, ToolContext};
use rolen_core::ledger::Ledger;
use rolen_core::rules::{self, RuleSet};
use rolen_core::types::{Session, SessionState};
use rolen_providers as providers;
use rolen_providers::chat::{HistMsg, ToolCall, ToolsChatRequest};
use std::path::PathBuf;

#[derive(Default)]
pub struct AgentOptions {
    pub workdir: PathBuf,
    pub role: String,
    pub task: String,
    /// Skip rule routing and use this provider/model directly.
    pub provider_override: Option<String>,
    pub model_override: Option<String>,
    pub max_steps: usize,
    /// Allow-listed shell programs for run_shell (empty = disabled).
    pub shell_allow: Vec<String>,
    /// Task id used on session/tickets (default: the session id).
    pub task_id: Option<String>,
    /// External write sink (orchestrator queue in M3); direct write if None.
    pub sink: Option<Box<dyn crate::sink::WriteSink>>,
    /// Cooperative cancellation: checked between agent steps.
    pub cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// Files the task is expected to produce (claimed paths). The agent is
    /// nudged until they exist (or nudges run out).
    pub expected_paths: Vec<String>,
}

pub enum AgentEvent {
    Routed {
        rule_id: String,
        provider: String,
        model: String,
        explanation: String,
    },
    Text(String),
    ToolCall {
        name: String,
        summary: String,
    },
    ToolDone {
        name: String,
        is_error: bool,
        summary: String,
    },
    Compacted {
        dropped: usize,
    },
    Retrying {
        attempt: u32,
        reason: String,
    },
    Migrated {
        from: String,
        to: String,
        model: String,
    },
    Done(String),
}

#[derive(Debug)]
pub struct RunReport {
    pub session_id: String,
    pub provider: String,
    pub model: String,
    pub final_text: String,
    pub steps: usize,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost: f64,
}

/// Retryable provider failures: rate limits, overload, transient 5xx/network.
fn is_retryable(e: &providers::ProviderError) -> bool {
    let s = e.to_string();
    s.contains("HTTP 429")
        || s.contains("HTTP 500")
        || s.contains("HTTP 502")
        || s.contains("HTTP 503")
        || s.contains("HTTP 529")
        || s.contains("timed out")
        || s.contains("overloaded")
}

/// FR-3.3 mid-session migration: re-run routing with the failing provider
/// marked unhealthy, returning the next fallback provider/model.
fn migrate(
    role: &str,
    failed_provider: &str,
    reg: &providers::ProviderRegistry,
) -> Option<(rolen_core::types::Provider, String)> {
    let rules = RuleSet::load().ok()?;
    let mut ctx = providers::routing::collect(None, None).ok()?;
    if let Some(state) = ctx.providers.get_mut(failed_provider) {
        state.healthy = false;
    }
    let decision = rules::decide(&rules, role, &ctx).ok()?;
    if decision.provider == failed_provider {
        return None;
    }
    let provider = reg.get(&decision.provider)?.clone();
    Some((provider, decision.model))
}

const DEFAULT_MAX_STEPS: usize = 30;
/// Rough chars-per-token estimate for context compaction.
const CHARS_PER_TOKEN: usize = 4;
/// Compact when estimated history exceeds this many tokens (FR-12.3 basic).
const COMPACT_AT_TOKENS: usize = 24_000;

pub fn run(
    opts: &mut AgentOptions,
    on_event: &mut dyn FnMut(AgentEvent),
) -> Result<RunReport, RuntimeError> {
    std::fs::create_dir_all(&opts.workdir)?;

    // ---- routing (FR-3) ----
    let (provider_id, model, route_explanation, rule_id) = resolve_route(opts)?;
    on_event(AgentEvent::Routed {
        rule_id,
        provider: provider_id.clone(),
        model: model.clone(),
        explanation: route_explanation,
    });

    let reg = providers::ProviderRegistry::load()?;
    let mut provider = reg
        .get(&provider_id)
        .ok_or_else(|| providers::ProviderError::NotFound(provider_id.clone()))?
        .clone();
    let mut provider_id = provider_id;
    let mut model = model;

    // ---- session + ledger ----
    let session_id = format!(
        "s-{}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let ledger = Ledger::open_default()?;
    let mut session = Session {
        id: session_id.clone(),
        task_id: None,
        provider_id: provider_id.clone(),
        model: model.clone(),
        state: SessionState::Running,
        tokens_in: 0,
        tokens_out: 0,
        cost: 0.0,
        started: chrono::Utc::now(),
        transcript_path: None,
    };
    ledger.upsert_session(&session)?;

    let tool_ctx = ToolContext {
        workdir: opts.workdir.clone(),
        shell_allow: opts.shell_allow.clone(),
        sink: opts
            .sink
            .take()
            .unwrap_or_else(|| Box::new(DirectWriteSink::new(opts.workdir.clone()))),
        task_id: opts.task_id.clone().unwrap_or_else(|| session_id.clone()),
    };

    let mut history: Vec<HistMsg> = vec![
        HistMsg::System(system_prompt(&opts.role)),
        HistMsg::User(opts.task.clone()),
    ];

    let mut steps = 0;
    let mut nudges = 0;
    let mut writes_made = 0usize;
    let mut migrations = 0;
    let max_steps = if opts.max_steps == 0 {
        DEFAULT_MAX_STEPS
    } else {
        opts.max_steps
    };
    let result = loop {
        steps += 1;
        if steps > max_steps {
            break Err(RuntimeError::StepLimit(max_steps));
        }
        if opts
            .cancel
            .as_ref()
            .map(|c| c.load(std::sync::atomic::Ordering::Relaxed))
            .unwrap_or(false)
        {
            break Err(RuntimeError::Cancelled);
        }

        maybe_compact(&mut history, on_event);

        // ---- LLM call with retry + mid-session migration (FR-3.3) ----
        let resp: Result<_, RuntimeError> = 'call: {
            let mut attempt = 0u32;
            loop {
                match providers::client::chat_tools(
                    &provider,
                    &ToolsChatRequest {
                        model: model.clone(),
                        system: None,
                        history: history.clone(),
                        tools: tools::specs(),
                        max_tokens: Some(4096),
                    },
                ) {
                    Ok(r) => break 'call Ok(r),
                    Err(e) => {
                        if is_retryable(&e) {
                            if attempt < 3 {
                                attempt += 1;
                                on_event(AgentEvent::Retrying {
                                    attempt,
                                    reason: e.to_string().chars().take(120).collect(),
                                });
                                std::thread::sleep(std::time::Duration::from_secs(1 << attempt));
                                continue;
                            }
                            if migrations < 2 && opts.provider_override.is_none() {
                                // FR-3.3: re-evaluate routing with this provider marked
                                // unhealthy; history is provider-agnostic, so the
                                // session simply continues on the next fallback.
                                migrations += 1;
                                if let Some((new_provider, new_model)) =
                                    migrate(&opts.role, &provider_id, &reg)
                                {
                                    on_event(AgentEvent::Migrated {
                                        from: provider_id.clone(),
                                        to: new_provider.id.clone(),
                                        model: new_model.clone(),
                                    });
                                    provider_id = new_provider.id.clone();
                                    model = new_model;
                                    provider = new_provider;
                                    attempt = 0;
                                    continue;
                                }
                            }
                        }
                        break 'call Err(RuntimeError::Provider(e));
                    }
                }
            }
        };
        let resp = match resp {
            Ok(r) => r,
            Err(e) => break Err(e),
        };

        // ledger every call (FR-4.1/FR-4.6)
        let cost =
            providers::test::estimate_cost(&provider, &model, resp.tokens_in, resp.tokens_out);
        let _ = ledger.record(&rolen_core::types::LedgerEntry {
            id: format!(
                "le-{}",
                chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
            ),
            session_id: session_id.clone(),
            provider_id: provider_id.clone(),
            tokens_in: resp.tokens_in,
            tokens_out: resp.tokens_out,
            cost,
            latency_ms: Some(resp.latency_ms),
            ts: chrono::Utc::now(),
        });
        session.tokens_in += resp.tokens_in;
        session.tokens_out += resp.tokens_out;
        session.cost += cost;
        let _ = ledger.upsert_session(&session);

        if !resp.text.is_empty() {
            on_event(AgentEvent::Text(resp.text.clone()));
        }

        // Salvage: some models emit tool calls as JSON *text* instead of
        // structured tool_calls. Recover them so the loop can continue.
        let tool_calls = if resp.tool_calls.is_empty() {
            salvage_tool_calls(&resp.text)
        } else {
            resp.tool_calls.clone()
        };

        if tool_calls.is_empty() {
            // Completion guard: the task isn't done while expected files are
            // missing (or nothing was written at all for write-tasks).
            let missing: Vec<String> = opts
                .expected_paths
                .iter()
                .filter(|p| !opts.workdir.join(p).exists())
                .cloned()
                .collect();
            let needs_work = if opts.expected_paths.is_empty() {
                writes_made == 0
            } else {
                !missing.is_empty()
            };
            if needs_work && nudges < 3 {
                nudges += 1;
                history.push(HistMsg::Assistant {
                    text: resp.text.clone(),
                    tool_calls: vec![],
                });
                let detail = if missing.is_empty() {
                    "You have not submitted any file writes yet.".to_string()
                } else {
                    format!("These files are still missing: {}.", missing.join(", "))
                };
                history.push(HistMsg::User(format!(
                    "{detail} Continue working: create each remaining file via the submit_write tool \
                     (one call per file, FULL content). If your interface does not offer structured tool \
                     calls, respond with ONLY one JSON object per call, e.g.: \
                     {{\"name\": \"submit_write\", \"arguments\": {{\"path\": \"file.txt\", \"content\": \"...\", \"op\": \"create\"}}}}. \
                     Only stop when every required file exists."
                )));
                continue;
            }
            break Ok(resp.text.clone());
        }

        writes_made += tool_calls
            .iter()
            .filter(|c| c.name == "submit_write")
            .count();

        history.push(HistMsg::Assistant {
            text: resp.text.clone(),
            tool_calls: tool_calls.clone(),
        });

        let mut outcomes = Vec::new();
        for call in &tool_calls {
            on_event(AgentEvent::ToolCall {
                name: call.name.clone(),
                summary: summarize_call(call),
            });
            let outcome = tools::execute(&tool_ctx, call);
            on_event(AgentEvent::ToolDone {
                name: call.name.clone(),
                is_error: outcome.is_error,
                summary: outcome.content.chars().take(200).collect(),
            });
            outcomes.push(outcome);
        }
        history.push(HistMsg::ToolResults(outcomes));
    };

    match &result {
        Ok(text) => {
            session.state = SessionState::Done;
            let _ = ledger.upsert_session(&session);
            on_event(AgentEvent::Done(text.clone()));
        }
        Err(_) => {
            session.state = SessionState::Failed;
            let _ = ledger.upsert_session(&session);
        }
    }

    result.map(|final_text| RunReport {
        session_id,
        provider: provider_id,
        model,
        final_text,
        steps,
        tokens_in: session.tokens_in,
        tokens_out: session.tokens_out,
        cost: session.cost,
    })
}

fn resolve_route(opts: &AgentOptions) -> Result<(String, String, String, String), RuntimeError> {
    if let Some(p) = &opts.provider_override {
        let model = opts
            .model_override
            .clone()
            .ok_or_else(|| RuntimeError::Sandbox("--model is required with --provider".into()))?;
        return Ok((
            p.clone(),
            model.clone(),
            "explicit --provider/--model override".into(),
            "override".into(),
        ));
    }
    let rules = RuleSet::load()?;
    let ctx = providers::routing::collect(None, None)?;
    let decision = rules::decide(&rules, &opts.role, &ctx)?;
    let model = match &opts.model_override {
        Some(m) => m.clone(),
        None => decision.model.clone(),
    };
    let skipped = if decision.skipped.is_empty() {
        String::new()
    } else {
        format!(
            "; skipped: {}",
            decision
                .skipped
                .iter()
                .map(|(e, r)| format!("{e} ({r})"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    Ok((
        decision.provider.clone(),
        model,
        format!("{}{}", decision.explanation, skipped),
        decision.rule_id.clone(),
    ))
}

fn system_prompt(role: &str) -> String {
    format!(
        "You are RoleN's {role} agent working inside a sandboxed workspace.\n\
         Rules you MUST follow:\n\
         - You can read files, list directories, search, and run allow-listed shell commands via tools.\n\
         - To create, modify or delete files you MUST use the submit_write tool with the FULL new content. \
           You have no other way to write files.\n\
         - Paths are always relative to the workspace root.\n\
         - If requirements are ambiguous, use ask_user. If no answer is available, state your assumption.\n\
         - Work step by step. When the task is fully done, reply with a concise summary and no tool calls."
    )
}

fn summarize_call(call: &ToolCall) -> String {
    let args = call.args.to_string();
    args.chars().take(120).collect()
}

/// Recover tool calls emitted as JSON text by models that don't populate the
/// structured tool_calls field (common with smaller local models). Accepts a
/// bare object, an array of objects, or JSON objects embedded in prose.
/// Only objects whose `name` matches a known tool are considered.
fn salvage_tool_calls(text: &str) -> Vec<ToolCall> {
    let known = [
        "read_file",
        "list_dir",
        "search",
        "run_shell",
        "submit_write",
        "ask_user",
    ];
    let mut out = Vec::new();
    let try_value = |v: &serde_json::Value, out: &mut Vec<ToolCall>| {
        if let (Some(name), Some(args)) = (
            v["name"].as_str(),
            v.get("arguments").or_else(|| v.get("args")),
        ) {
            if known.contains(&name) {
                out.push(ToolCall {
                    id: format!("salvaged_{}", out.len()),
                    name: name.to_string(),
                    args: args.clone(),
                });
            }
        }
    };

    // whole-text JSON (object or array)
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(text.trim()) {
        match &v {
            serde_json::Value::Array(arr) => arr.iter().for_each(|v| try_value(v, &mut out)),
            other => try_value(other, &mut out),
        }
        if !out.is_empty() {
            return out;
        }
    }
    // embedded JSON objects in prose: balanced-brace scan
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '{' {
            let mut depth = 0;
            let mut in_str = false;
            let mut esc = false;
            let mut j = i;
            while j < chars.len() {
                let c = chars[j];
                if in_str {
                    if esc {
                        esc = false;
                    } else if c == '\\' {
                        esc = true;
                    } else if c == '"' {
                        in_str = false;
                    }
                } else if c == '"' {
                    in_str = true;
                } else if c == '{' {
                    depth += 1;
                } else if c == '}' {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                j += 1;
            }
            if depth == 0 {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(
                    &chars[i..=j].iter().collect::<String>(),
                ) {
                    try_value(&v, &mut out);
                }
                i = j + 1;
            } else {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    out
}

/// Deterministic context compaction (basic FR-12.3; LLM-summarized hand-off
/// arrives with the session-migration work). Keeps system + task + the most
/// recent exchanges.
fn maybe_compact(history: &mut Vec<HistMsg>, on_event: &mut dyn FnMut(AgentEvent)) {
    let estimate = |h: &[HistMsg]| -> usize {
        h.iter()
            .map(|m| match m {
                HistMsg::System(s) | HistMsg::User(s) => s.len(),
                HistMsg::Assistant { text, tool_calls } => {
                    text.len()
                        + tool_calls
                            .iter()
                            .map(|c| c.args.to_string().len() + 32)
                            .sum::<usize>()
                }
                HistMsg::ToolResults(rs) => rs.iter().map(|r| r.content.len() + 16).sum(),
            })
            .sum::<usize>()
            / CHARS_PER_TOKEN
    };
    if estimate(history) < COMPACT_AT_TOKENS || history.len() <= 4 {
        return;
    }
    let keep_head = 2; // system + original task
    let keep_tail = 6;
    let drop_count = history.len() - keep_head - keep_tail;
    if drop_count == 0 {
        return;
    }
    let tail: Vec<HistMsg> = history.split_off(history.len() - keep_tail);
    history.truncate(keep_head);
    history.push(HistMsg::System(format!(
        "[compaction] {drop_count} earlier messages (tool calls/results) were dropped to fit the context window. \
         Re-read files with tools if you need their content again."
    )));
    history.extend(tail);
    on_event(AgentEvent::Compacted {
        dropped: drop_count,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn salvages_bare_object() {
        let calls = salvage_tool_calls(
            r#"{"name": "submit_write", "arguments": {"path": "a.txt", "content": "x", "op": "create"}}"#,
        );
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "submit_write");
        assert_eq!(calls[0].args["path"], "a.txt");
    }

    #[test]
    fn salvages_multiple_embedded_in_prose() {
        let text = r##"I'll do it now.
{"name": "submit_write", "arguments": {"path": "h.md", "content": "# Hi", "op": "create"}}
then
{"name": "list_dir", "arguments": {"path": ""}}"##;
        let calls = salvage_tool_calls(text);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[1].name, "list_dir");
    }

    #[test]
    fn ignores_unknown_names_and_plain_text() {
        assert!(salvage_tool_calls("just a normal answer").is_empty());
        assert!(salvage_tool_calls(r#"{"name": "write_file", "arguments": {}}"#).is_empty());
    }
}
