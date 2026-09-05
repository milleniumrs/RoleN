//! Provider registry persisted to `providers.toml` (PRD FR-1.1 / FR-14.1).

use crate::error::ProviderError;
use rolen_core::config;
use rolen_core::types::Provider;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize)]
struct ProvidersFile {
    #[serde(default)]
    providers: Vec<Provider>,
}

#[derive(Debug, Default)]
pub struct ProviderRegistry {
    providers: Vec<Provider>,
}

impl ProviderRegistry {
    pub fn load() -> Result<Self, ProviderError> {
        let file = config::providers_file()?;
        if !file.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(&file)?;
        let parsed: ProvidersFile = toml::from_str(&text)?;
        Ok(Self {
            providers: parsed.providers,
        })
    }

    pub fn save(&self) -> Result<(), ProviderError> {
        config::ensure_dirs()?;
        let text = toml::to_string_pretty(&ProvidersFile {
            providers: self.providers.clone(),
        })?;
        std::fs::write(config::providers_file()?, text)?;
        Ok(())
    }

    pub fn list(&self) -> &[Provider] {
        &self.providers
    }

    pub fn get(&self, id: &str) -> Option<&Provider> {
        self.providers.iter().find(|p| p.id == id)
    }

    /// Insert or replace by id.
    pub fn upsert(&mut self, provider: Provider) {
        if let Some(existing) = self.providers.iter_mut().find(|p| p.id == provider.id) {
            *existing = provider;
        } else {
            self.providers.push(provider);
        }
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.providers.len();
        self.providers.retain(|p| p.id != id);
        self.providers.len() != before
    }

    /// FR-4.5: take a provider out of routing rotation (or put it back).
    pub fn set_suspended(&mut self, id: &str, suspended: bool) -> bool {
        match self.providers.iter_mut().find(|p| p.id == id) {
            Some(p) => {
                p.suspended = suspended;
                true
            }
            None => false,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }

    pub fn len(&self) -> usize {
        self.providers.len()
    }
}

/// Convention for keychain/vault key references (FR-2.1: config stores only
/// the reference, never the secret). Note: uses `-` as separator — Windows
/// Credential Manager mishandles `:` in credential targets.
pub fn key_ref_for(provider_id: &str) -> String {
    format!("provider-{provider_id}")
}
