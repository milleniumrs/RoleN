//! age-encrypted fallback vault (PRD FR-2.2) for machines without a usable
//! OS keychain. Whole-file encryption: a TOML map of key_ref → secret,
//! encrypted with a passphrase from `ROLEN_VAULT_PASSWORD` (interactive
//! unlock UI arrives with the Settings milestone).

use crate::config;
use crate::error::CoreError;
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::PathBuf;

pub const PASSWORD_ENV: &str = "ROLEN_VAULT_PASSWORD";

fn vault_file() -> Result<PathBuf, CoreError> {
    Ok(config::config_dir()?.join("vault.age"))
}

fn raw_password() -> Option<String> {
    std::env::var(PASSWORD_ENV).ok().filter(|pw| !pw.is_empty())
}

fn password() -> Result<age::secrecy::SecretString, CoreError> {
    match raw_password() {
        Some(pw) => Ok(age::secrecy::SecretString::new(pw)),
        None => Err(CoreError::VaultLocked),
    }
}

pub fn is_available() -> bool {
    raw_password().is_some()
}

/// Decrypt a vault blob with an explicit passphrase (pure — unit-tested).
fn decrypt_map(
    encrypted: &[u8],
    pw: &age::secrecy::SecretString,
) -> Result<BTreeMap<String, String>, CoreError> {
    let decryptor =
        match age::Decryptor::new(encrypted).map_err(|e| CoreError::Vault(e.to_string()))? {
            age::Decryptor::Passphrase(d) => d,
            _ => {
                return Err(CoreError::Vault(
                    "unexpected vault format (not passphrase-encrypted)".into(),
                ))
            }
        };
    let mut reader = decryptor
        .decrypt(pw, None)
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

/// Encrypt a secret map with an explicit passphrase (pure — unit-tested).
fn encrypt_map(
    map: &BTreeMap<String, String>,
    pw: &age::secrecy::SecretString,
) -> Result<Vec<u8>, CoreError> {
    let text = toml::to_string_pretty(map)?;
    let encryptor = age::Encryptor::with_user_passphrase(pw.clone());
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
    Ok(encrypted)
}

fn load_map() -> Result<BTreeMap<String, String>, CoreError> {
    let file = vault_file()?;
    if !file.exists() {
        return Ok(BTreeMap::new());
    }
    let encrypted = std::fs::read(&file)?;
    let pw = password()?;
    decrypt_map(&encrypted, &pw)
}

fn save_map(map: &BTreeMap<String, String>) -> Result<(), CoreError> {
    config::ensure_dirs()?;
    let pw = password()?;
    let encrypted = encrypt_map(map, &pw)?;
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
    set("__rolen_probe__", "ok")?;
    let ok = get("__rolen_probe__").map(|v| v == "ok").unwrap_or(false);
    let _ = delete("__rolen_probe__");
    if ok {
        Ok(())
    } else {
        Err(CoreError::Vault("roundtrip mismatch".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pw(s: &str) -> age::secrecy::SecretString {
        age::secrecy::SecretString::new(s.to_string())
    }

    /// The age-vault is the fallback secret backend wherever the OS keychain
    /// is unusable (headless Linux, CI). Pure crypto roundtrip: no env vars,
    /// no shared files, so it is race-free under parallel tests.
    #[test]
    fn encrypt_decrypt_roundtrip() {
        let mut map = BTreeMap::new();
        map.insert("provider-kimi".to_string(), "sk-secret".to_string());
        map.insert("provider-x".to_string(), "another".to_string());

        let blob = encrypt_map(&map, &pw("correct horse")).expect("encrypt");
        assert!(!blob.is_empty());
        assert!(
            !String::from_utf8_lossy(&blob).contains("sk-secret"),
            "secrets must not appear in the encrypted blob"
        );

        let back = decrypt_map(&blob, &pw("correct horse")).expect("decrypt");
        assert_eq!(back, map);
    }

    #[test]
    fn wrong_password_fails_to_decrypt() {
        let mut map = BTreeMap::new();
        map.insert("k".to_string(), "v".to_string());
        let blob = encrypt_map(&map, &pw("right")).expect("encrypt");
        assert!(decrypt_map(&blob, &pw("wrong")).is_err());
    }

    #[test]
    fn empty_vault_decrypts_to_empty_map() {
        let blob = encrypt_map(&BTreeMap::new(), &pw("p")).expect("encrypt");
        assert!(decrypt_map(&blob, &pw("p")).expect("decrypt").is_empty());
    }
}
