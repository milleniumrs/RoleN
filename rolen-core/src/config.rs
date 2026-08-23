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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Parallelism {
    /// 0 = automatic CPU heuristic `max(2, logical_cpus / 2)` (decision D6).
    pub global_cap: usize,
    /// Max concurrent sessions per provider.
    pub per_provider_cap: usize,
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
            },
            parallelism: Parallelism {
                global_cap: 0,
                per_provider_cap: 2,
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

pub fn ledger_file() -> Result<PathBuf, CoreError> {
    Ok(data_dir()?.join("ledger.sqlite3"))
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
