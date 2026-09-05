//! Project core data side (PRD FR-5/FR-6): project metadata, PRD.md/PRD.json
//! schema + rendering, AGENTS.md rendering, and the skill registry.
//! Pure/deterministic — LLM generation lives in `rolen-providers::generate`.

use crate::config;
use crate::error::CoreError;
use crate::types::Clarification;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const PROJECT_FILE: &str = "rolen-project.yaml";
pub const PRD_JSON_SCHEMA: u32 = 1;

// ------------------------------------------------------------------- meta

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMeta {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub stack: Vec<String>,
    pub created: DateTime<Utc>,
    #[serde(default)]
    pub clarifications: Vec<Clarification>,
    #[serde(default)]
    pub skills: Vec<String>,
}

impl ProjectMeta {
    pub fn load(dir: &Path) -> Result<Self, CoreError> {
        let text = std::fs::read_to_string(dir.join(PROJECT_FILE))?;
        serde_yaml::from_str(&text)
            .map_err(|e| CoreError::Vault(format!("{} parse: {e}", PROJECT_FILE)))
    }

    pub fn save(&self, dir: &Path) -> Result<(), CoreError> {
        let text = serde_yaml::to_string(self)
            .map_err(|e| CoreError::Vault(format!("{} serialize: {e}", PROJECT_FILE)))?;
        std::fs::write(dir.join(PROJECT_FILE), text)?;
        Ok(())
    }
}

/// Scaffold a new project directory (FR-5.1). Returns (meta, dir).
pub fn scaffold(
    name: &str,
    description: &str,
    stack: Vec<String>,
    root: &Path,
) -> Result<(ProjectMeta, PathBuf), CoreError> {
    let slug: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    if slug.is_empty() {
        return Err(CoreError::Vault(
            "project name produces an empty slug".into(),
        ));
    }
    let dir = root.join(&slug);
    if dir.join(PROJECT_FILE).exists() {
        return Err(CoreError::Vault(format!(
            "project '{slug}' already exists at {}",
            dir.display()
        )));
    }
    std::fs::create_dir_all(&dir)?;
    let meta = ProjectMeta {
        id: slug.clone(),
        name: name.to_string(),
        description: description.to_string(),
        stack,
        created: Utc::now(),
        clarifications: Vec::new(),
        skills: Vec::new(),
    };
    meta.save(&dir)?;
    // git init (FR-5.1); checkpoints are the orchestrator's job (FR-7.7)
    let _ = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    Ok((meta, dir))
}

/// List all projects under a workspace root (dirs containing PROJECT_FILE).
pub fn list_projects(root: &Path) -> Vec<(PathBuf, ProjectMeta)> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root) {
        for e in entries.flatten() {
            let dir = e.path();
            if dir.is_dir() && dir.join(PROJECT_FILE).exists() {
                if let Ok(meta) = ProjectMeta::load(&dir) {
                    out.push((dir, meta));
                }
            }
        }
    }
    out.sort_by(|a, b| a.1.name.cmp(&b.1.name));
    out
}

/// Find a project dir by id/name under the workspace root.
pub fn find_project(root: &Path, name: &str) -> Option<(PathBuf, ProjectMeta)> {
    list_projects(root)
        .into_iter()
        .find(|(_, m)| m.id == name || m.name.eq_ignore_ascii_case(name))
}

/// Walk up from `start` looking for a directory holding PROJECT_FILE.
/// Lets agents/batches running inside a project workspace find their project.
pub fn find_project_dir_upwards(start: &Path) -> Option<PathBuf> {
    let mut dir = Some(start);
    while let Some(d) = dir {
        if d.join(PROJECT_FILE).exists() {
            return Some(d.to_path_buf());
        }
        dir = d.parent();
    }
    None
}

/// Serializes read-modify-write cycles on PROJECT_FILE within this process
/// (several agent threads may record questions concurrently).
static QUESTION_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// FR-6.3: record a mid-project clarification raised by an agent (`ask_user`
/// tool) as a pending question in the project's metadata, where the TUI
/// interrogation center and the scheduler can see it.
pub fn record_question(
    dir: &Path,
    task_id: Option<&str>,
    question: &str,
) -> Result<Clarification, CoreError> {
    let _guard = QUESTION_LOCK
        .lock()
        .map_err(|_| CoreError::Vault("question lock poisoned".into()))?;
    let mut meta = ProjectMeta::load(dir)?;
    let clarification = crate::types::Clarification {
        id: format!(
            "mq-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ),
        project_id: meta.id.clone(),
        task_id: task_id.map(str::to_string),
        question: question.to_string(),
        options: Vec::new(),
        answer: None,
        status: crate::types::ClarificationStatus::Pending,
        linked_prd_path: None,
        ts: chrono::Utc::now(),
    };
    meta.clarifications.push(clarification.clone());
    // atomic write: temp file + rename, so a crash never halves the YAML
    let target = dir.join(PROJECT_FILE);
    let tmp = dir.join(format!(".{PROJECT_FILE}.tmp"));
    let text = serde_yaml::to_string(&meta)
        .map_err(|e| CoreError::Vault(format!("{PROJECT_FILE} serialize: {e}")))?;
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &target)?;
    Ok(clarification)
}

/// Task ids that currently have unanswered (pending) questions in this project.
/// The scheduler pauses tasks that depend on these (FR-6.3).
pub fn pending_question_task_ids(dir: &Path) -> std::collections::HashSet<String> {
    ProjectMeta::load(dir)
        .map(|m| {
            m.clarifications
                .iter()
                .filter(|c| c.status == crate::types::ClarificationStatus::Pending)
                .filter_map(|c| c.task_id.clone())
                .collect()
        })
        .unwrap_or_default()
}

// ------------------------------------------------------------------- PRD

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PrdFeature {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_priority")]
    pub priority: String,
}

fn default_priority() -> String {
    "should".into()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PrdContent {
    #[serde(default)]
    pub overview: String,
    #[serde(default)]
    pub goals: Vec<String>,
    #[serde(default)]
    pub non_goals: Vec<String>,
    #[serde(default)]
    pub features: Vec<PrdFeature>,
    #[serde(default)]
    pub constraints: Vec<String>,
    #[serde(default)]
    pub definition_of_done: Vec<String>,
}

/// Render PRD.md from structured content (FR-5.2).
pub fn render_prd_md(meta: &ProjectMeta, prd: &PrdContent) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "# {} — Product Requirements Document\n\n",
        meta.name
    ));
    s.push_str(&format!(
        "- Generated by RoleN on {}\n",
        meta.created.format("%Y-%m-%d")
    ));
    s.push_str(&format!(
        "- Stack: {}\n\n",
        if meta.stack.is_empty() {
            "unspecified".into()
        } else {
            meta.stack.join(", ")
        }
    ));
    s.push_str("## Overview\n\n");
    s.push_str(&format!("{}\n\n", prd.overview.trim()));
    if !prd.goals.is_empty() {
        s.push_str("## Goals\n\n");
        for g in &prd.goals {
            s.push_str(&format!("- {}\n", g.trim()));
        }
        s.push('\n');
    }
    if !prd.non_goals.is_empty() {
        s.push_str("## Non-Goals\n\n");
        for g in &prd.non_goals {
            s.push_str(&format!("- {}\n", g.trim()));
        }
        s.push('\n');
    }
    if !prd.features.is_empty() {
        s.push_str("## Features\n\n");
        for f in &prd.features {
            s.push_str(&format!(
                "### {} — {} ({})\n\n{}\n\n",
                f.id,
                f.title,
                f.priority,
                f.description.trim()
            ));
        }
    }
    if !prd.constraints.is_empty() {
        s.push_str("## Constraints\n\n");
        for c in &prd.constraints {
            s.push_str(&format!("- {}\n", c.trim()));
        }
        s.push('\n');
    }
    if !prd.definition_of_done.is_empty() {
        s.push_str("## Definition of Done\n\n");
        for d in &prd.definition_of_done {
            s.push_str(&format!("- [ ] {}\n", d.trim()));
        }
        s.push('\n');
    }
    let answered: Vec<&Clarification> = meta
        .clarifications
        .iter()
        .filter(|c| matches!(c.status, crate::types::ClarificationStatus::Answered))
        .collect();
    if !answered.is_empty() {
        s.push_str("## Clarifications (from the interview)\n\n");
        for c in answered {
            s.push_str(&format!(
                "- **Q:** {}\n  **A:** {}\n",
                c.question.trim(),
                c.answer.clone().unwrap_or_default().trim()
            ));
        }
        s.push('\n');
    }
    s
}

/// PRD.json value (schema-versioned, FR-5.2) with clarification traceability
/// (FR-6.5).
pub fn prd_json(meta: &ProjectMeta, prd: &PrdContent) -> serde_json::Value {
    serde_json::json!({
        "schema_version": PRD_JSON_SCHEMA,
        "meta": {
            "id": meta.id,
            "name": meta.name,
            "description": meta.description,
            "stack": meta.stack,
            "created": meta.created,
        },
        "overview": prd.overview,
        "goals": prd.goals,
        "non_goals": prd.non_goals,
        "features": prd.features,
        "constraints": prd.constraints,
        "definition_of_done": prd.definition_of_done,
        "clarifications": meta.clarifications,
    })
}

/// Write PRD.md + PRD.json into the project dir.
pub fn write_prd(dir: &Path, meta: &ProjectMeta, prd: &PrdContent) -> Result<(), CoreError> {
    std::fs::write(dir.join("PRD.md"), render_prd_md(meta, prd))?;
    let json = serde_json::to_string_pretty(&prd_json(meta, prd))
        .map_err(|e| CoreError::Vault(format!("PRD.json serialize: {e}")))?;
    std::fs::write(dir.join("PRD.json"), json)?;
    Ok(())
}

/// Validate a PRD.json file (`rolen prd validate`).
pub fn validate_prd_json(path: &Path) -> Result<Vec<String>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("read: {e}"))?;
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("json: {e}"))?;
    let mut problems = Vec::new();
    if v["schema_version"].as_u64() != Some(PRD_JSON_SCHEMA as u64) {
        problems.push(format!("schema_version must be {PRD_JSON_SCHEMA}"));
    }
    if v["meta"]["name"].as_str().unwrap_or("").is_empty() {
        problems.push("meta.name missing".into());
    }
    if v["overview"].as_str().unwrap_or("").is_empty() {
        problems.push("overview missing".into());
    }
    if !v["features"].is_array() {
        problems.push("features must be an array".into());
    }
    if !v["clarifications"].is_array() {
        problems.push("clarifications must be an array".into());
    }
    Ok(problems)
}

// ------------------------------------------------------------- AGENTS.md

/// Deterministic AGENTS.md rendering (FR-5.3) — golden-testable.
pub fn render_agents_md(meta: &ProjectMeta, prd: &PrdContent) -> String {
    let mut s = String::new();
    s.push_str(&format!("# AGENTS.md — {}\n\n", meta.name));
    s.push_str("> Generated by RoleN. Keep it updated when structure, conventions or workflows change.\n\n");
    s.push_str("## Project\n\n");
    s.push_str(&format!("{}\n\n", prd.overview.trim()));
    if !meta.stack.is_empty() {
        s.push_str(&format!("**Stack:** {}\n\n", meta.stack.join(", ")));
    }
    s.push_str("## Working agreements (enforced by RoleN)\n\n");
    s.push_str("- You never write files directly — submit **write tickets** (`submit_write`) with FULL content; the orchestrator is the only writer.\n");
    s.push_str("- Paths are workspace-relative; stay inside your task's claimed paths.\n");
    s.push_str("- When requirements are ambiguous, use `ask_user` — do not guess silently.\n");
    s.push_str("- Run only allow-listed shell commands.\n\n");
    if !meta.stack.is_empty() {
        s.push_str("## Build & test\n\n");
        for cmd in stack_commands(&meta.stack) {
            s.push_str(&format!("- `{cmd}`\n"));
        }
        s.push('\n');
    }
    if !prd.features.is_empty() {
        s.push_str("## Feature map\n\n");
        for f in &prd.features {
            s.push_str(&format!("- **{}** ({}): {}\n", f.id, f.priority, f.title));
        }
        s.push('\n');
    }
    if !meta.skills.is_empty() {
        s.push_str("## Installed skills\n\n");
        for sk in &meta.skills {
            s.push_str(&format!("- `skills/{sk}/SKILL.md`\n"));
        }
        s.push('\n');
    }
    if !prd.definition_of_done.is_empty() {
        s.push_str("## Definition of done\n\n");
        for d in &prd.definition_of_done {
            s.push_str(&format!("- {}\n", d.trim()));
        }
        s.push('\n');
    }
    s
}

fn stack_commands(stack: &[String]) -> Vec<&'static str> {
    let mut cmds = Vec::new();
    for item in stack {
        match item.to_lowercase().as_str() {
            "rust" => {
                cmds.push("cargo build --workspace");
                cmds.push("cargo test --workspace");
                cmds.push("cargo clippy --workspace");
            }
            "node" | "nodejs" | "typescript" | "javascript" => {
                cmds.push("npm install");
                cmds.push("npm test");
            }
            "python" => {
                cmds.push("pytest");
                cmds.push("ruff check .");
            }
            "go" => {
                cmds.push("go build ./...");
                cmds.push("go test ./...");
            }
            _ => {}
        }
    }
    cmds
}

// ------------------------------------------------------------------ skills

#[derive(Debug, Clone)]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    /// None for built-in skills not yet materialized to disk.
    pub path: Option<PathBuf>,
}

/// Parse a SKILL.md (D4 convention): YAML front-matter with name/description/
/// tags, body follows.
pub fn parse_skill_md(text: &str) -> Option<SkillInfo> {
    let text = text.trim_start();
    let rest = text.strip_prefix("---")?;
    let end = rest.find("\n---")?;
    let front = &rest[..end];
    let v: serde_yaml::Value = serde_yaml::from_str(front).ok()?;
    let name = v["name"].as_str()?.to_string();
    let description = v["description"].as_str().unwrap_or("").to_string();
    let tags = v["tags"]
        .as_sequence()
        .map(|seq| {
            seq.iter()
                .filter_map(|t| t.as_str().map(String::from))
                .collect()
        })
        .or_else(|| {
            v["tags"]
                .as_str()
                .map(|s| s.split(',').map(|t| t.trim().to_string()).collect())
        })
        .unwrap_or_default();
    Some(SkillInfo {
        name,
        description,
        tags,
        path: None,
    })
}

/// Built-in skill library, materialized to `<config>/skills/` on first use so
/// users can edit/extend it (FR-5.4).
pub const BUILTIN_SKILLS: &[(&str, &str)] = &[
    ("prd-refinement", "---\nname: prd-refinement\ndescription: Turn vague ideas into complete PRDs by hunting corner cases\ntags: [planning, prd, requirements, clarification]\n---\n\n# PRD Refinement\n\nWhen refining a PRD: enumerate actors, data entities, error paths, empty states, permission boundaries, scale limits, and offline behavior. Convert every 'should probably' into either a requirement or an explicit non-goal.\n"),
    ("agents-md", "---\nname: agents-md\ndescription: Author and maintain AGENTS.md files that keep coding agents effective\ntags: [agents, documentation, conventions]\n---\n\n# AGENTS.md Authoring\n\nKeep AGENTS.md factual: build/test commands that actually work, directory map, conventions with examples, forbidden actions. Update it in the same commit as any structural change.\n"),
    ("git-workflow", "---\nname: git-workflow\ndescription: Branch, checkpoint and review discipline for agent-driven changes\ntags: [git, workflow, review, checkpoints]\n---\n\n# Git Workflow\n\nOne task = one checkpoint commit. Review diffs between checkpoints, not final states. Revert by resetting to the previous checkpoint, never by hand-editing backwards.\n"),
    ("rust-workspace", "---\nname: rust-workspace\ndescription: Cargo workspace layout and crate boundaries for Rust projects\ntags: [rust, cargo, workspace, architecture]\n---\n\n# Rust Workspace\n\nOne crate per responsibility; UI-free core crates; workspace-level dependency versions; `cargo test --workspace` must stay green.\n"),
    ("appcui-tui", "---\nname: appcui-tui\ndescription: Building terminal UIs with the AppCUI-rs library\ntags: [rust, tui, appcui, ui]\n---\n\n# AppCUI TUI\n\nUse the proc-macros (`#[Window]`, `window!`, control macros). Handle events via the *Events traits. Keep windows small and compose with tabs/splitters. Check examples/ in the AppCUI-rs repo for idioms.\n"),
    ("api-integration", "---\nname: api-integration\ndescription: Robust HTTP API clients: retries, pagination, rate limits, secret handling\ntags: [api, http, integration, providers]\n---\n\n# API Integration\n\nNever log secrets. Parse responses defensively (optional fields, unit-test the parsers against recorded payloads). Treat 429 as a routing signal, not an exception.\n"),
];

/// Ensure the skill library exists on disk; returns its directory.
pub fn skill_library_dir() -> Result<PathBuf, CoreError> {
    let dir = config::config_dir()?.join("skills");
    for (name, content) in BUILTIN_SKILLS {
        let f = dir.join(name).join("SKILL.md");
        if !f.exists() {
            std::fs::create_dir_all(f.parent().unwrap())?;
            std::fs::write(f, content)?;
        }
    }
    Ok(dir)
}

/// Load all skills from the library directory.
pub fn load_skills() -> Result<Vec<SkillInfo>, CoreError> {
    let dir = skill_library_dir()?;
    let mut out = Vec::new();
    for e in std::fs::read_dir(&dir)?.flatten() {
        let f = e.path().join("SKILL.md");
        if f.exists() {
            if let Ok(text) = std::fs::read_to_string(&f) {
                if let Some(mut info) = parse_skill_md(&text) {
                    info.path = Some(e.path());
                    out.push(info);
                }
            }
        }
    }
    Ok(out)
}

/// Suggest skills by keyword overlap with the project text (FR-5.4).
pub fn suggest_skills(meta: &ProjectMeta, prd: &PrdContent, limit: usize) -> Vec<SkillInfo> {
    let mut haystack = format!(
        "{} {} {} {} {}",
        meta.name,
        meta.description,
        meta.stack.join(" "),
        prd.overview,
        prd.features
            .iter()
            .map(|f| format!("{} {}", f.title, f.description))
            .collect::<Vec<_>>()
            .join(" ")
    )
    .to_lowercase();
    haystack.push(' ');
    let mut scored: Vec<(usize, SkillInfo)> = load_skills()
        .unwrap_or_default()
        .into_iter()
        .map(|s| {
            let score = s
                .tags
                .iter()
                .filter(|t| haystack.contains(&t.to_lowercase()))
                .count();
            (score, s)
        })
        .filter(|(score, _)| *score > 0)
        .collect();
    scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    scored.into_iter().take(limit).map(|(_, s)| s).collect()
}

/// Install a skill into the project (copy `skills/<name>/`).
pub fn install_skill(project_dir: &Path, skill_name: &str) -> Result<PathBuf, CoreError> {
    let library = skill_library_dir()?;
    let src = library.join(skill_name);
    if !src.join("SKILL.md").exists() {
        return Err(CoreError::Vault(format!(
            "skill '{skill_name}' not found in {}",
            library.display()
        )));
    }
    let dst = project_dir.join("skills").join(skill_name);
    std::fs::create_dir_all(&dst)?;
    for e in std::fs::read_dir(&src)?.flatten() {
        if e.file_type().map(|t| t.is_file()).unwrap_or(false) {
            std::fs::copy(e.path(), dst.join(e.file_name()))?;
        }
    }
    Ok(dst)
}

// ------------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> ProjectMeta {
        ProjectMeta {
            id: "demo".into(),
            name: "Demo".into(),
            description: "a demo".into(),
            stack: vec!["rust".into()],
            created: Utc::now(),
            clarifications: vec![],
            skills: vec![],
        }
    }

    fn prd() -> PrdContent {
        PrdContent {
            overview: "A demo project.".into(),
            goals: vec!["Do things".into()],
            non_goals: vec![],
            features: vec![PrdFeature {
                id: "F1".into(),
                title: "Core".into(),
                description: "The core feature".into(),
                priority: "must".into(),
            }],
            constraints: vec![],
            definition_of_done: vec!["tests pass".into()],
        }
    }

    #[test]
    fn prd_md_contains_key_sections() {
        let md = render_prd_md(&meta(), &prd());
        assert!(md.contains("# Demo — Product Requirements Document"));
        assert!(md.contains("## Goals"));
        assert!(md.contains("### F1 — Core (must)"));
        assert!(md.contains("- [ ] tests pass"));
    }

    #[test]
    fn prd_json_roundtrip_validates() {
        let dir = std::env::temp_dir().join(format!("rolen-prd-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        write_prd(&dir, &meta(), &prd()).unwrap();
        let problems = validate_prd_json(&dir.join("PRD.json")).unwrap();
        assert!(problems.is_empty(), "problems: {problems:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn agents_md_has_working_agreements_and_commands() {
        let md = render_agents_md(&meta(), &prd());
        assert!(md.contains("write tickets"));
        assert!(md.contains("cargo test --workspace"));
        assert!(md.contains("**F1**"));
    }

    #[test]
    fn parses_skill_frontmatter() {
        let info = parse_skill_md(BUILTIN_SKILLS[0].1).unwrap();
        assert_eq!(info.name, "prd-refinement");
        assert!(info.tags.contains(&"planning".to_string()));
    }

    #[test]
    fn skill_suggestion_matches_stack() {
        let skills = suggest_skills(&meta(), &prd(), 3);
        assert!(skills.iter().any(|s| s.name == "rust-workspace"));
    }

    #[test]
    fn record_question_appends_pending_and_is_visible_to_scheduler() {
        let dir = std::env::temp_dir().join(format!("rolen-recq-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        meta().save(&dir).unwrap();

        let c = record_question(&dir, Some("task-a"), "Which DB engine?").unwrap();
        assert_eq!(c.status, crate::types::ClarificationStatus::Pending);
        assert_eq!(c.task_id.as_deref(), Some("task-a"));

        let pending = pending_question_task_ids(&dir);
        assert!(pending.contains("task-a"));

        // answering the question unblocks the dependent tasks
        let mut m = ProjectMeta::load(&dir).unwrap();
        m.clarifications[0].status = crate::types::ClarificationStatus::Answered;
        m.save(&dir).unwrap();
        assert!(pending_question_task_ids(&dir).is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn finds_project_dir_walking_upwards() {
        let dir = std::env::temp_dir().join(format!("rolen-upwards-{}", std::process::id()));
        let nested = dir.join("sub").join("dir");
        std::fs::create_dir_all(&nested).unwrap();
        meta().save(&dir).unwrap();
        assert_eq!(find_project_dir_upwards(&nested), Some(dir.clone()));
        assert!(find_project_dir_upwards(std::path::Path::new("C:\\")).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }
}
