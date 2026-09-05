//! Per-CLI adapter specs (PRD FR-13.3): how to invoke each known CLI agent
//! non-interactively with a prompt, inside a PTY.
//!
//! Adapters are **data-driven** (NFR-6): drop-in entries in
//! `<config dir>/cli-adapters.toml` extend or override the built-ins without
//! a recompile:
//!
//! ```toml
//! [[adapter]]
//! match = "aider"                                # substring of the program stem
//! args = ["--message", "{prompt}", "--yes"]      # {prompt} = task text
//!
//! # FR-4.2: optional quota probe — `rolen provider sync-quota` runs it and
//! # parses the output with this regex (named groups "used"/"limit", or
//! # capture groups 1/2):
//! # quota_args = ["quota"]
//! # quota_regex = "used (?P<used>[0-9_]+) of (?P<limit>[0-9_]+)"
//! ```

use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct CliSpec {
    pub program: PathBuf,
    /// Argument template; `{prompt}` is replaced by the task text.
    pub args: Vec<String>,
    /// FR-4.2: optional quota probe (subcommand args + output regex).
    pub quota_args: Option<Vec<String>>,
    pub quota_regex: Option<String>,
}

/// One user-defined adapter entry from `cli-adapters.toml`.
#[derive(Debug, Clone, Deserialize)]
struct AdapterEntry {
    /// Substring matched (case-insensitive) against the program stem.
    #[serde(rename = "match")]
    pattern: String,
    args: Vec<String>,
    #[serde(default)]
    quota_args: Option<Vec<String>>,
    #[serde(default)]
    quota_regex: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct AdaptersFile {
    #[serde(default)]
    adapter: Vec<AdapterEntry>,
}

/// User-defined adapters from `<config dir>/cli-adapters.toml`
/// (missing/invalid file = none).
fn user_adapters() -> Vec<AdapterEntry> {
    let Ok(path) = rolen_core::config::config_dir().map(|d| d.join("cli-adapters.toml")) else {
        return Vec::new();
    };
    std::fs::read_to_string(path)
        .ok()
        .map(|text| parse_adapters(&text))
        .unwrap_or_default()
}

fn parse_adapters(text: &str) -> Vec<AdapterEntry> {
    toml::from_str::<AdaptersFile>(text)
        .map(|f| f.adapter)
        .unwrap_or_default()
}

/// Pick the adapter entry for a program stem: user adapters first (first
/// match wins), then built-in templates, then the generic `-p` convention.
fn pick_entry(stem: &str, user: &[AdapterEntry]) -> AdapterEntry {
    for entry in user {
        if stem.contains(&entry.pattern.to_lowercase()) {
            return entry.clone();
        }
    }
    // Templates aim for non-interactive, permission-autonomous operation;
    // the overlay (D3) keeps that safe.
    let args: Vec<String> = match stem {
        s if s.contains("claude") => vec![
            "-p".into(),
            "{prompt}".into(),
            "--dangerously-skip-permissions".into(),
        ],
        s if s.contains("codex") => {
            vec!["exec".into(), "--full-auto".into(), "{prompt}".into()]
        }
        s if s.contains("gemini") => vec!["--yolo".into(), "-p".into(), "{prompt}".into()],
        s if s.contains("kimi") => vec!["-p".into(), "{prompt}".into()],
        // generic fallback: print-mode convention
        _ => vec!["-p".into(), "{prompt}".into()],
    };
    AdapterEntry {
        pattern: stem.to_string(),
        args,
        quota_args: None,
        quota_regex: None,
    }
}

/// Backwards-compatible wrapper used by the tests.
#[cfg(test)]
fn pick_args(stem: &str, user: &[AdapterEntry]) -> Vec<String> {
    pick_entry(stem, user).args
}

impl CliSpec {
    /// Build the spec for a CLI provider: user-defined adapters from
    /// cli-adapters.toml first, then built-in templates keyed by the program
    /// name, then a generic `-p` convention.
    pub fn for_provider(provider: &rolen_core::types::Provider) -> Option<Self> {
        let program = provider.cli_path.clone()?;
        let stem = program
            .file_stem()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        Some(Self::for_program(program, &stem))
    }

    pub fn for_program(program: PathBuf, stem: &str) -> Self {
        let entry = pick_entry(stem, &user_adapters());
        Self {
            program,
            args: entry.args,
            quota_args: entry.quota_args,
            quota_regex: entry.quota_regex,
        }
    }

    pub fn argv(&self, prompt: &str) -> Vec<String> {
        self.args
            .iter()
            .map(|a| a.replace("{prompt}", prompt))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_template() {
        let s = CliSpec::for_program(PathBuf::from("claude.exe"), "claude");
        let argv = s.argv("do stuff");
        assert_eq!(
            argv,
            vec!["-p", "do stuff", "--dangerously-skip-permissions"]
        );
    }

    #[test]
    fn codex_template() {
        let s = CliSpec::for_program(PathBuf::from("codex"), "codex");
        assert_eq!(s.argv("x"), vec!["exec", "--full-auto", "x"]);
    }

    #[test]
    fn generic_fallback() {
        let s = CliSpec::for_program(PathBuf::from("mycli"), "mycli");
        assert_eq!(s.argv("x"), vec!["-p", "x"]);
    }

    #[test]
    fn user_adapters_parse_and_override_builtins() {
        // FR-13.3/NFR-6: drop-in TOML adapters, no recompile
        let entries = parse_adapters(
            r#"
            [[adapter]]
            match = "claude"
            args = ["run", "{prompt}", "--auto"]
            "#,
        );
        assert_eq!(entries.len(), 1);
        // user adapter overrides the built-in claude template
        assert_eq!(
            pick_args("claude", &entries),
            vec!["run", "{prompt}", "--auto"]
        );
        // unrelated stems still use built-ins
        assert_eq!(
            pick_args("codex", &entries),
            vec!["exec", "--full-auto", "{prompt}"]
        );
    }

    #[test]
    fn invalid_toml_yields_no_adapters() {
        assert!(parse_adapters("not [valid toml").is_empty());
        assert!(parse_adapters("").is_empty());
    }
}
