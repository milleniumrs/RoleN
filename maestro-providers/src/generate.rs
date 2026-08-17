//! LLM-driven generation for the project core (PRD FR-5.2/FR-6.1): interview
//! questions and PRD content, produced by the `interrogator` / `doc-writer`
//! roles through the rule engine. Robust JSON extraction included.

use crate::chat::ChatRequest;
use crate::client;
use crate::error::ProviderError;
use crate::registry::ProviderRegistry;
use crate::routing;
use maestro_core::project::{PrdContent, ProjectMeta};
use maestro_core::rules::{self, RuleSet};
use maestro_core::types::{Provider, QuestionMode};
use serde_json::Value;

/// One generated interview question.
#[derive(Debug, Clone)]
pub struct GenQuestion {
    pub question: String,
    pub options: Vec<String>,
}

/// Route a role through the rule engine and return the provider + model.
fn route(role: &str) -> Result<(Provider, String), ProviderError> {
    let rules = RuleSet::load()?;
    let ctx = routing::collect(None, None)?;
    let decision = rules::decide(&rules, role, &ctx)
        .map_err(|e| ProviderError::Api(format!("routing role '{role}': {e}")))?;
    let reg = ProviderRegistry::load()?;
    let provider = reg
        .get(&decision.provider)
        .ok_or_else(|| ProviderError::NotFound(decision.provider.clone()))?
        .clone();
    Ok((provider, decision.model))
}

fn ask(role: &str, prompt: &str, max_tokens: u32) -> Result<String, ProviderError> {
    let (provider, model) = route(role)?;
    let mut req = ChatRequest::single(model, prompt);
    req.max_tokens = Some(max_tokens);
    let resp = client::chat(&provider, &req)?;
    Ok(resp.text)
}

/// Extract the first balanced JSON array (or object) from LLM output.
pub fn extract_json(text: &str, want_array: bool) -> Option<Value> {
    let chars: Vec<char> = text.chars().collect();
    let (open, close) = if want_array { ('[', ']') } else { ('{', '}') };
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == open {
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
                } else if c == open {
                    depth += 1;
                } else if c == close {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                j += 1;
            }
            if depth == 0 {
                if let Ok(v) =
                    serde_json::from_str::<Value>(&chars[i..=j].iter().collect::<String>())
                {
                    return Some(v);
                }
            }
            i += 1;
        } else {
            i += 1;
        }
    }
    None
}

fn question_count(mode: QuestionMode) -> usize {
    match mode {
        QuestionMode::Thorough => 14,
        QuestionMode::Balanced => 8,
        QuestionMode::Minimal => 4,
    }
}

/// FR-6.1: generate targeted clarifying questions for a project.
pub fn generate_questions(
    meta: &ProjectMeta,
    mode: QuestionMode,
) -> Result<Vec<GenQuestion>, ProviderError> {
    let n = question_count(mode);
    let prompt = format!(
        "You are the interrogator for a new software project. Your job: eliminate ambiguity BEFORE any code is written.\n\
         Project name: {name}\nDescription: {desc}\nStack: {stack}\n\n\
         Generate exactly {n} clarifying questions covering corner cases developers forget: \
         scope edges, data model, error handling, auth/permissions, performance targets, target platforms, \
         i18n, accessibility, testing strategy, deployment, licensing, and definition-of-done.\n\
         Return ONLY a JSON array, no prose:\n\
         [{{\"question\": \"...\", \"options\": [\"option A\", \"option B\"]}}, ...]\n\
         Options are suggested answers (2-4 per question, may be empty for free-text questions).",
        name = meta.name,
        desc = meta.description,
        stack = if meta.stack.is_empty() { "unspecified".into() } else { meta.stack.join(", ") },
        n = n,
    );
    let text = ask("interrogator", &prompt, 2048)?;
    let v = extract_json(&text, true).ok_or_else(|| {
        ProviderError::Parse(format!(
            "interrogator returned no JSON array: {}",
            text.chars().take(200).collect::<String>()
        ))
    })?;
    let arr = v
        .as_array()
        .ok_or_else(|| ProviderError::Parse("questions JSON is not an array".into()))?;
    let mut out = Vec::new();
    for q in arr {
        if let Some(question) = q["question"].as_str() {
            let options = q["options"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|o| o.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            out.push(GenQuestion {
                question: question.to_string(),
                options,
            });
        }
    }
    if out.is_empty() {
        return Err(ProviderError::Parse(
            "no usable questions in interrogator output".into(),
        ));
    }
    Ok(out)
}

/// FR-5.2: generate structured PRD content from meta + answered clarifications.
pub fn generate_prd(meta: &ProjectMeta) -> Result<PrdContent, ProviderError> {
    let mut clar = String::new();
    for c in &meta.clarifications {
        if let Some(a) = &c.answer {
            clar.push_str(&format!("- Q: {}\n  A: {}\n", c.question, a));
        }
    }
    let prompt = format!(
        "You are the doc-writer for a new software project. Produce the structured PRD content.\n\
         Project name: {name}\nDescription: {desc}\nStack: {stack}\n\
         Clarified with the user:\n{clar}\n\n\
         Return ONLY a JSON object (no prose, no markdown fences) with exactly these keys:\n\
         {{\"overview\": \"2-4 sentences\", \"goals\": [\"...\"], \"non_goals\": [\"...\"], \
         \"features\": [{{\"id\": \"F1\", \"title\": \"...\", \"description\": \"...\", \"priority\": \"must|should|could\"}}], \
         \"constraints\": [\"...\"], \"definition_of_done\": [\"...\"]}}\n\
         5-10 features. Every feature description must mention acceptance-relevant details \
         surfaced by the clarifications.",
        name = meta.name,
        desc = meta.description,
        stack = if meta.stack.is_empty() { "unspecified".into() } else { meta.stack.join(", ") },
        clar = if clar.is_empty() { "(none — make conservative assumptions and list them as constraints)".into() } else { clar },
    );
    let text = ask("doc-writer", &prompt, 4096)?;
    let v = extract_json(&text, false).ok_or_else(|| {
        ProviderError::Parse(format!(
            "doc-writer returned no JSON object: {}",
            text.chars().take(200).collect::<String>()
        ))
    })?;
    serde_json::from_value(v).map_err(|e| ProviderError::Parse(format!("PRD JSON shape: {e}")))
}

/// Raw LLM call for DAG generation (used by maestro-orchestrator::daggen).
pub fn generate_text(role: &str, prompt: &str, max_tokens: u32) -> Result<String, ProviderError> {
    ask(role, prompt, max_tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_array_from_prose() {
        let text = "Sure! Here you go:\n```json\n[{\"question\": \"q1\", \"options\": []}]\n```\nHope that helps";
        let v = extract_json(text, true).unwrap();
        assert_eq!(v[0]["question"], "q1");
    }

    #[test]
    fn extracts_object_with_nested_braces() {
        let text = r#"{"overview": "a {nested} thing", "goals": ["x"]}"#;
        let v = extract_json(text, false).unwrap();
        assert_eq!(v["overview"], "a {nested} thing");
    }

    #[test]
    fn returns_none_on_garbage() {
        assert!(extract_json("no json here", true).is_none());
    }
}
