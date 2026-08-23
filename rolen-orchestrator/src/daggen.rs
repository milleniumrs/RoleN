//! PRD → task DAG proposal (PRD FR-5.5): the planner role turns features
//! into TaskSpecs with dependencies and claimed paths.

use crate::scheduler::TaskSpec;
use rolen_core::project::{PrdContent, ProjectMeta};
use rolen_providers::generate;
use rolen_providers::ProviderError;

/// Generate a task DAG from the PRD. Features become tasks; the planner
/// assigns deps and non-overlapping claimed_paths.
pub fn generate_dag(meta: &ProjectMeta, prd: &PrdContent) -> Result<Vec<TaskSpec>, ProviderError> {
    let features = prd
        .features
        .iter()
        .map(|f| format!("{} ({}) {}: {}", f.id, f.priority, f.title, f.description))
        .collect::<Vec<_>>()
        .join("\n");
    let prompt = format!(
        "You are the planner for the software project \"{name}\" (stack: {stack}).\n\
         Overview: {overview}\n\nFeatures:\n{features}\n\n\
         Decompose the features into an executable task DAG for parallel coding agents.\n\
         Rules:\n\
         - 3-8 tasks. Each task has: id (short slug), role (one of: planner, coder, reviewer, doc-writer, tool-runner), \
         title, task (a precise instruction telling the agent exactly which files to create and what they contain), \
         deps (ids of tasks that must finish first), claimed_paths (files/dirs this task exclusively writes).\n\
         - claimed_paths of tasks that may run in PARALLEL must not overlap.\n\
         - First task should usually be scaffolding (project structure, build files); integration/review tasks come last.\n\
         Return ONLY a JSON array:\n\
         [{{\"id\": \"...\", \"role\": \"...\", \"title\": \"...\", \"task\": \"...\", \"deps\": [...], \"claimed_paths\": [...]}}]",
        name = meta.name,
        stack = if meta.stack.is_empty() { "unspecified".into() } else { meta.stack.join(", ") },
        overview = prd.overview,
        features = features,
    );
    let text = generate::generate_text("planner", &prompt, 4096)?;
    let v = generate::extract_json(&text, true).ok_or_else(|| {
        ProviderError::Parse(format!(
            "planner returned no JSON array: {}",
            text.chars().take(200).collect::<String>()
        ))
    })?;
    let mut tasks: Vec<TaskSpec> = serde_json::from_value(v)
        .map_err(|e| ProviderError::Parse(format!("task DAG shape: {e}")))?;
    if tasks.is_empty() {
        return Err(ProviderError::Parse("planner produced an empty DAG".into()));
    }
    // defense: force claimed_paths uniqueness across parallel-eligible tasks
    let mut seen = std::collections::HashSet::new();
    for t in &mut tasks {
        t.claimed_paths.retain(|p| seen.insert(p.clone()));
    }
    Ok(tasks)
}
