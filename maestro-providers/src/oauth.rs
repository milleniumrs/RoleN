//! OAuth subscription support (Anthropic Claude Pro/Max via opencode's
//! auth.json). Tokens are stored as a JSON blob in the secret store under the
//! provider's key_ref; access tokens are refreshed automatically when close
//! to expiry.

use crate::error::ProviderError;
use maestro_core::secrets;
use maestro_core::types::Provider;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Claude Code / opencode-claude-auth public OAuth client id.
const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const TOKEN_URL: &str = "https://console.anthropic.com/v1/oauth/token";
/// Refresh when less than this many seconds remain.
const REFRESH_SKEW_SECS: i64 = 120;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthTokens {
    pub access: String,
    pub refresh: String,
    /// Expiry as epoch seconds or milliseconds (opencode uses millis).
    pub expires: i64,
}

impl OAuthTokens {
    fn expires_secs(&self) -> i64 {
        if self.expires > 1_000_000_000_000 {
            self.expires / 1000
        } else {
            self.expires
        }
    }

    pub fn is_expired(&self) -> bool {
        chrono::Utc::now().timestamp() + REFRESH_SKEW_SECS >= self.expires_secs()
    }
}

/// Default opencode auth location (~/.local/share/opencode/auth.json).
pub fn default_opencode_auth() -> Option<std::path::PathBuf> {
    dirs_path().filter(|p| p.exists())
}

fn dirs_path() -> Option<std::path::PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(|h| Path::new(&h).join(".local/share/opencode/auth.json"))
}

/// Import the `anthropic` OAuth entry from an opencode auth.json.
pub fn import_from_opencode(path: &Path) -> Result<OAuthTokens, ProviderError> {
    let text = std::fs::read_to_string(path)?;
    let v: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| ProviderError::Parse(format!("invalid auth.json: {e}")))?;
    let a = &v["anthropic"];
    if a["type"].as_str() != Some("oauth") {
        return Err(ProviderError::Parse(format!(
            "no anthropic oauth entry in {}",
            path.display()
        )));
    }
    Ok(OAuthTokens {
        access: a["access"]
            .as_str()
            .ok_or_else(|| ProviderError::Parse("missing access token".into()))?
            .to_string(),
        refresh: a["refresh"]
            .as_str()
            .ok_or_else(|| ProviderError::Parse("missing refresh token".into()))?
            .to_string(),
        expires: a["expires"].as_i64().unwrap_or(0),
    })
}

/// Store tokens for a provider (as its key_ref secret, JSON-encoded).
pub fn store_tokens(key_ref: &str, tokens: &OAuthTokens) -> Result<(), ProviderError> {
    let json = serde_json::to_string(tokens)
        .map_err(|e| ProviderError::Parse(format!("token serialize: {e}")))?;
    secrets::set_secret(key_ref, &json)?;
    Ok(())
}

/// Returns a valid access token for the provider, refreshing first if needed.
pub fn fresh_access_token(provider: &Provider) -> Result<String, ProviderError> {
    let kref = provider.key_ref.as_ref().ok_or_else(|| {
        ProviderError::Api(format!("provider '{}' has no token reference", provider.id))
    })?;
    let raw = secrets::get_secret(kref)?;
    let mut tokens: OAuthTokens = serde_json::from_str(&raw)
        .map_err(|e| ProviderError::Parse(format!("stored tokens are not OAuth JSON: {e}")))?;
    if tokens.is_expired() {
        tokens = refresh_tokens(&tokens)?;
        store_tokens(kref, &tokens)?;
    }
    Ok(tokens.access)
}

fn refresh_tokens(tokens: &OAuthTokens) -> Result<OAuthTokens, ProviderError> {
    let http = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;
    let resp = http
        .post(TOKEN_URL)
        .json(&serde_json::json!({
            "grant_type": "refresh_token",
            "refresh_token": tokens.refresh,
            "client_id": CLIENT_ID,
        }))
        .send()?;
    let status = resp.status();
    let text = resp.text()?;
    if !status.is_success() {
        return Err(ProviderError::Api(format!(
            "oauth refresh failed HTTP {status}: {}",
            text.chars().take(300).collect::<String>()
        )));
    }
    let v: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| ProviderError::Parse(format!("invalid refresh response: {e}")))?;
    let access = v["access_token"]
        .as_str()
        .ok_or_else(|| ProviderError::Parse("refresh response missing access_token".into()))?
        .to_string();
    let refresh = v["refresh_token"]
        .as_str()
        .map(|s| s.to_string())
        .unwrap_or_else(|| tokens.refresh.clone());
    let expires_in = v["expires_in"].as_i64().unwrap_or(3600);
    Ok(OAuthTokens {
        access,
        refresh,
        expires: chrono::Utc::now().timestamp() + expires_in,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expiry_handles_epoch_secs_and_millis() {
        let future_secs = chrono::Utc::now().timestamp() + 3600;
        assert!(!OAuthTokens {
            access: "a".into(),
            refresh: "r".into(),
            expires: future_secs
        }
        .is_expired());
        assert!(!OAuthTokens {
            access: "a".into(),
            refresh: "r".into(),
            expires: future_secs * 1000
        }
        .is_expired());
        assert!(OAuthTokens {
            access: "a".into(),
            refresh: "r".into(),
            expires: 1
        }
        .is_expired());
        assert!(OAuthTokens {
            access: "a".into(),
            refresh: "r".into(),
            expires: 1_000_000
        }
        .is_expired()); // millis in the past
    }
}
