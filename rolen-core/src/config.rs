//! TOML configuration (PRD FR-14.1) with XDG / %APPDATA% paths.

use crate::error::CoreError;
use crate::types::{AlertAction, QuestionMode};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

const APP_DIR: &str = "rolen";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct General {
    pub workspace_root: PathBuf,
    pub theme: String,
    pub question_mode: QuestionMode,
    /// FR-9.4: also send quota/task alerts as OS toast notifications (opt-in).
    #[serde(default)]
    pub os_notifications: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Parallelism {
    /// 0 = automatic CPU heuristic `max(2, logical_cpus / 2)` (decision D6).
    pub global_cap: usize,
    /// Max concurrent sessions per provider.
    pub per_provider_cap: usize,
    /// FR-7.8 backpressure: max write tickets pending in the queue before
    /// submitters block. 0 = unlimited.
    #[serde(default = "default_queue_cap")]
    pub queue_cap: usize,
}

fn default_queue_cap() -> usize {
    1000
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Quotas {
    pub warn_pct: u8,
    pub crit_pct: u8,
    pub action: AlertAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub general: General,
    pub parallelism: Parallelism,
    pub quotas: Quotas,
}

impl Default for Config {
    fn default() -> Self {
        let workspace_root = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("rolen-workspaces");
        Self {
            general: General {
                workspace_root,
                theme: "dark".into(),
                question_mode: QuestionMode::Thorough,
                os_notifications: false,
            },
            parallelism: Parallelism {
                global_cap: 0,
                per_provider_cap: 2,
                queue_cap: default_queue_cap(),
            },
            quotas: Quotas {
                warn_pct: 80,
                crit_pct: 95,
                action: AlertAction::Notify,
            },
        }
    }
}

impl Parallelism {
    /// Effective global session cap, applying the D6 heuristic when set to 0.
    pub fn effective_global_cap(&self) -> usize {
        if self.global_cap > 0 {
            self.global_cap
        } else {
            let cpus = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4);
            (cpus / 2).max(2)
        }
    }
}

/// Append the application directory name to a platform base directory.
fn app_dir(base: PathBuf) -> PathBuf {
    base.join(APP_DIR)
}

pub fn config_dir() -> Result<PathBuf, CoreError> {
    Ok(app_dir(dirs::config_dir().ok_or(CoreError::NoConfigDir)?))
}

pub fn data_dir() -> Result<PathBuf, CoreError> {
    Ok(app_dir(dirs::data_dir().ok_or(CoreError::NoDataDir)?))
}

pub fn config_file() -> Result<PathBuf, CoreError> {
    Ok(config_dir()?.join("config.toml"))
}

pub fn providers_file() -> Result<PathBuf, CoreError> {
    Ok(config_dir()?.join("providers.toml"))
}

pub fn rules_file() -> Result<PathBuf, CoreError> {
    Ok(config_dir()?.join("rules.yaml")) // canonical YAML — decision D2
}

pub fn subscriptions_file() -> Result<PathBuf, CoreError> {
    Ok(config_dir()?.join("subscriptions.toml"))
}

/// Per-model prices. Separate from providers.toml because model discovery
/// rewrites a provider's model list wholesale (see rolen-core::pricing).
pub fn pricing_file() -> Result<PathBuf, CoreError> {
    Ok(config_dir()?.join("pricing.toml"))
}

pub fn ledger_file() -> Result<PathBuf, CoreError> {
    Ok(data_dir()?.join("ledger.sqlite3"))
}

// ------------------------------------------------- setup import / export

/// Files included in a setup export (FR-14.4). Secrets are never included —
/// config holds only keychain references, which stay valid on this machine.
pub const EXPORT_FILES: &[&str] = &[
    "config.toml",
    "providers.toml",
    "rules.yaml",
    "subscriptions.toml",
    "pricing.toml",
];

/// Export the setup as one JSON bundle: `{schema, files: {name: content}}`.
pub fn export_setup() -> Result<String, CoreError> {
    let dir = config_dir()?;
    let mut files = serde_json::Map::new();
    for name in EXPORT_FILES {
        let path = dir.join(name);
        if path.exists() {
            let text = fs::read_to_string(&path)
                .map_err(|e| CoreError::Vault(format!("export {}: {e}", path.display())))?;
            files.insert(name.to_string(), serde_json::Value::String(text));
        }
    }
    let bundle = serde_json::json!({
        "schema": 1,
        "app": "rolen",
        "exported": chrono::Utc::now().to_rfc3339(),
        "secrets": "excluded — keychain references only; re-enter keys on the target machine",
        "files": files,
    });
    serde_json::to_string_pretty(&bundle)
        .map_err(|e| CoreError::Vault(format!("export serialize: {e}")))
}

/// Import a bundle produced by [`export_setup`]. Existing files are backed up
/// next to the new ones (`<name>.bak`). Secrets are untouched.
pub fn import_setup(bundle: &str) -> Result<Vec<String>, CoreError> {
    let v: serde_json::Value =
        serde_json::from_str(bundle).map_err(|e| CoreError::Vault(format!("import parse: {e}")))?;
    let files = v["files"]
        .as_object()
        .ok_or_else(|| CoreError::Vault("import: missing 'files' object".into()))?;
    let dir = config_dir()?;
    fs::create_dir_all(&dir)?;
    let mut written = Vec::new();
    for (name, content) in files {
        if !EXPORT_FILES.contains(&name.as_str()) {
            continue; // refuse to write anything outside the known set
        }
        let Some(text) = content.as_str() else {
            continue;
        };
        let path = dir.join(name);
        if path.exists() {
            fs::copy(&path, dir.join(format!("{name}.bak")))?;
        }
        fs::write(&path, text)?;
        written.push(name.clone());
    }
    Ok(written)
}

/// Create config + data directories if missing. Returns the dirs.
pub fn ensure_dirs() -> Result<(PathBuf, PathBuf), CoreError> {
    let cfg = config_dir()?;
    let data = data_dir()?;
    fs::create_dir_all(&cfg)?;
    fs::create_dir_all(&data)?;
    Ok((cfg, data))
}

// ------------------------------------------------------------- load / save

impl Config {
    pub fn load() -> Result<Self, CoreError> {
        let text = fs::read_to_string(config_file()?)?;
        Ok(toml::from_str(&text)?)
    }

    pub fn save(&self) -> Result<(), CoreError> {
        ensure_dirs()?;
        let text = toml::to_string_pretty(self)?;
        fs::write(config_file()?, text)?;
        Ok(())
    }

    /// Load the config, writing a default one first if none exists.
    /// Returns `(config, created)`.
    pub fn ensure() -> Result<(Self, bool), CoreError> {
        ensure_dirs()?;
        if config_file()?.exists() {
            Ok((Self::load()?, false))
        } else {
            let cfg = Self::default();
            cfg.save()?;
            Ok((cfg, true))
        }
    }

    /// Ensure the workspace root exists.
    pub fn ensure_workspace_root(&self) -> Result<(), CoreError> {
        fs::create_dir_all(&self.general.workspace_root)?;
        Ok(())
    }
}
