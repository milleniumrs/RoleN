//! Auto-detection of providers on this machine (FR-1.2): a running Ollama
//! server plus known CLI agents on PATH. Shared by the CLI and the TUI.

use crate::client;
use crate::ollama;
use rolen_core::types::{Provider, ProviderType};

const KNOWN_CLIS: &[&str] = &["claude", "codex", "gemini", "kimi", "ollama"];

pub fn detect_all() -> Vec<Provider> {
    let mut found = Vec::new();

    // Ollama local server
    let probe = Provider {
        id: "ollama-local".into(),
        ptype: ProviderType::OllamaLocal,
        auth: Default::default(),
        tunnel: None,
        endpoint: Some(ollama::DEFAULT_LOCAL_BASE.into()),
        cli_path: None,
        key_ref: None,
        models: Vec::new(),
    };
    if client::list_models(&probe).is_ok() {
        found.push(probe);
    }

    // CLI agents on PATH (wrapped via PTY adapters in M5)
    for cli in KNOWN_CLIS {
        if let Some(path) = which(cli) {
            found.push(Provider {
                id: format!("cli-{cli}"),
                ptype: ProviderType::Cli,
                auth: Default::default(),
                tunnel: None,
                endpoint: None,
                cli_path: Some(path.into()),
                key_ref: None,
                models: Vec::new(),
            });
        }
    }

    found
}

fn which(program: &str) -> Option<String> {
    let (cmd, arg) = if cfg!(windows) {
        ("where", program)
    } else {
        ("which", program)
    };
    let out = std::process::Command::new(cmd).arg(arg).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}
