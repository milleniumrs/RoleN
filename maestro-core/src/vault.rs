//! age-encrypted fallback vault (PRD FR-2.2) for machines without a usable
//! OS keychain. Whole-file encryption: a TOML map of key_ref → secret,
//! encrypted with a passphrase from `MAESTRO_VAULT_PASSWORD` (interactive
//! unlock UI arrives with the Settings milestone).

use crate::config;
use crate::error::CoreError;
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::PathBuf;

pub const PASSWORD_ENV: &str = "MAESTRO_VAULT_PASSWORD";

fn vault_file() -> Result<PathBuf, CoreError> {
    Ok(config::config_dir()?.join("vault.age"))
}

fn password() -> Result<age::secrecy::SecretString, CoreError> {
    match std::env::var(PASSWORD_ENV) {
        Ok(pw) if !pw.is_empty() => Ok(age::secrecy::SecretString::new(pw)),
        _ => Err(CoreError::VaultLocked),
    }
}

pub fn is_available() -> bool {
    std::env::var(PASSWORD_ENV)
        .map(|v| !v.is_empty())
        .unwrap_or(false)
}

fn load_map() -> Result<BTreeMap<String, String>, CoreError> {
    let file = vault_file()?;
    if !file.exists() {
        return Ok(BTreeMap::new());
    }
    let encrypted = std::fs::read(&file)?;
    let decryptor =
        match age::Decryptor::new(&encrypted[..]).map_err(|e| CoreError::Vault(e.to_string()))? {
            age::Decryptor::Passphrase(d) => d,
            _ => {
                return Err(CoreError::Vault(
                    "unexpected vault format (not passphrase-encrypted)".into(),
                ))
            }
        };
    let pw = password()?;
    let mut reader = decryptor
        .decrypt(&pw, None)
        .map_err(|e| CoreError::Vault(format!("decrypt failed (wrong password?): {e}")))?;
    let mut text = String::new();
    reader
        .read_to_string(&mut text)
        .map_err(|e| CoreError::Vault(e.to_string()))?;
    if text.trim().is_empty() {
        return Ok(BTreeMap::new());
    }
    Ok(toml::from_str(&text)?)
}

fn save_map(map: &BTreeMap<String, String>) -> Result<(), CoreError> {
    config::ensure_dirs()?;
    let text = toml::to_string_pretty(map)?;
    let pw = password()?;
    let encryptor = age::Encryptor::with_user_passphrase(pw);
    let mut encrypted = Vec::new();
    let mut writer = encryptor
        .wrap_output(&mut encrypted)
        .map_err(|e| CoreError::Vault(e.to_string()))?;
    writer
        .write_all(text.as_bytes())
        .map_err(|e| CoreError::Vault(e.to_string()))?;
    writer
        .finish()
        .map_err(|e| CoreError::Vault(e.to_string()))?;
    // atomic-ish write: temp + rename (mirrors FR-7.6 philosophy)
    let file = vault_file()?;
    let tmp = file.with_extension("tmp");
    std::fs::write(&tmp, &encrypted)?;
    std::fs::rename(&tmp, &file)?;
    Ok(())
}

pub fn set(key_ref: &str, value: &str) -> Result<(), CoreError> {
    let mut map = load_map()?;
    map.insert(key_ref.to_string(), value.to_string());
    save_map(&map)
}

pub fn get(key_ref: &str) -> Result<String, CoreError> {
    let map = load_map()?;
    map.get(key_ref)
        .cloned()
        .ok_or_else(|| CoreError::Vault(format!("no secret for key '{key_ref}'")))
}

pub fn delete(key_ref: &str) -> Result<(), CoreError> {
    let mut map = load_map()?;
    map.remove(key_ref);
    save_map(&map)
}

/// Roundtrip probe used by `config doctor` when the keychain is unavailable.
pub fn probe() -> Result<(), CoreError> {
    set("__maestro_probe__", "ok")?;
    let ok = get("__maestro_probe__").map(|v| v == "ok").unwrap_or(false);
    let _ = delete("__maestro_probe__");
    if ok {
        Ok(())
    } else {
        Err(CoreError::Vault("roundtrip mismatch".into()))
    }
}
