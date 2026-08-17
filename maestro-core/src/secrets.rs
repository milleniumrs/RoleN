//! Secret storage routing (PRD FR-2): env var → OS keychain → encrypted vault.
//!
//! - FR-2.1: OS keychain via `keyring` (Windows Credential Manager, macOS
//!   Keychain, Secret Service).
//! - FR-2.2: age-encrypted vault fallback (see `vault` module) when the
//!   keychain is unavailable.
//! - FR-2.3: `MAESTRO_KEY_<KEY_REF>` env vars take precedence (CI/headless).

use crate::error::CoreError;
use crate::vault;

const SERVICE: &str = "maestro";
const PROBE_KEY: &str = "__maestro_probe__";

fn entry(key_ref: &str) -> Result<keyring::Entry, CoreError> {
    Ok(keyring::Entry::new(SERVICE, key_ref)?)
}

/// Environment variable checked before any backend (FR-2.3):
/// `MAESTRO_KEY_<KEY_REF>` with non-alphanumerics replaced by `_`, uppercased.
pub fn env_var_name(key_ref: &str) -> String {
    let sanitized: String = key_ref
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect::<String>()
        .to_uppercase();
    format!("MAESTRO_KEY_{sanitized}")
}

/// Store a secret: keychain first, encrypted vault as fallback.
pub fn set_secret(key_ref: &str, value: &str) -> Result<(), CoreError> {
    match entry(key_ref).and_then(|e| e.set_password(value).map_err(CoreError::from)) {
        Ok(()) => Ok(()),
        Err(keychain_err) => {
            if vault::is_available() {
                vault::set(key_ref, value)
            } else {
                Err(keychain_err)
            }
        }
    }
}

/// Read a secret: env var, then keychain, then vault.
pub fn get_secret(key_ref: &str) -> Result<String, CoreError> {
    if let Ok(v) = std::env::var(env_var_name(key_ref)) {
        return Ok(v);
    }
    match entry(key_ref).and_then(|e| e.get_password().map_err(CoreError::from)) {
        Ok(v) => Ok(v),
        Err(keychain_err) => {
            if vault::is_available() {
                vault::get(key_ref)
            } else {
                Err(keychain_err)
            }
        }
    }
}

pub fn delete_secret(key_ref: &str) -> Result<(), CoreError> {
    // best effort on both backends
    let _ = entry(key_ref).and_then(|e| e.delete_credential().map_err(CoreError::from));
    if vault::is_available() {
        let _ = vault::delete(key_ref);
    }
    Ok(())
}

/// Which storage backend is usable, for diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Keychain,
    Vault,
    None,
}

pub fn active_backend() -> Backend {
    if keychain_probe().is_ok() {
        Backend::Keychain
    } else if vault::is_available() && vault::probe().is_ok() {
        Backend::Vault
    } else {
        Backend::None
    }
}

/// Keychain roundtrip probe (write, read back, delete).
pub fn keychain_probe() -> Result<(), CoreError> {
    let e = entry(PROBE_KEY)?;
    e.set_password("ok")?;
    let roundtrip = e.get_password().map(|v| v == "ok").unwrap_or(false);
    let _ = e.delete_credential();
    if roundtrip {
        Ok(())
    } else {
        Err(CoreError::Keyring(keyring::Error::Invalid(
            "roundtrip".into(),
            "value mismatch".into(),
        )))
    }
}
