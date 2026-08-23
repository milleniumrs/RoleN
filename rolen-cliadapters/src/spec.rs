//! Per-CLI adapter specs (PRD FR-13.3): how to invoke each known CLI agent
//! non-interactively with a prompt, inside a PTY.

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct CliSpec {
    pub program: PathBuf,
    /// Argument template; `{prompt}` is replaced by the task text.
    pub args: Vec<String>,
}

impl CliSpec {
    /// Build the spec for a CLI provider, using known templates keyed by the
    /// program name and falling back to a generic `-p` convention.
    pub fn for_provider(provider: &rolen_core::types::Provider) -> Option<Self> {
        let program = provider.cli_path.clone()?;
        let stem = program
            .file_stem()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        Some(Self::for_program(program, &stem))
    }

    pub fn for_program(program: PathBuf, stem: &str) -> Self {
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
        Self { program, args }
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
}
