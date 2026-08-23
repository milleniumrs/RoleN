//! Dispatch layer: routes calls to the right client by provider type,
//! resolves secrets, and implements health checks (FR-1.3).

use crate::chat::{ChatRequest, ChatResponse};
use crate::error::ProviderError;
use crate::{anthropic, oauth, ollama, openai, registry};
use rolen_core::secrets;
use rolen_core::types::{AuthKind, Model, Provider, ProviderType};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct Health {
    pub ok: bool,
    pub latency_ms: u64,
    pub models: usize,
    pub detail: String,
}

fn http_client() -> Result<reqwest::blocking::Client, ProviderError> {
    http_client_with_timeout(Duration::from_secs(30))
}

/// Generations on large local models can take minutes — short timeouts only
/// for discovery/health calls.
fn http_client_long() -> Result<reqwest::blocking::Client, ProviderError> {
    http_client_with_timeout(Duration::from_secs(900))
}

fn http_client_with_timeout(timeout: Duration) -> Result<reqwest::blocking::Client, ProviderError> {
    Ok(reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(timeout)
        .build()?)
}

/// Resolve the API key for a provider via the secret store (env → keychain →
/// vault). Returns None when the provider has no key reference.
fn resolve_key(provider: &Provider) -> Result<Option<String>, ProviderError> {
    match &provider.key_ref {
        Some(kref) => Ok(Some(secrets::get_secret(kref)?)),
        None => Ok(None),
    }
}

/// Anthropic auth resolution: OAuth subscription tokens (auto-refreshed) or
/// a plain API key, depending on the provider configuration.
fn resolve_anthropic_auth(provider: &Provider) -> Result<Option<anthropic::Auth>, ProviderError> {
    match provider.auth {
        AuthKind::OAuth => Ok(Some(anthropic::Auth::OAuth(oauth::fresh_access_token(
            provider,
        )?))),
        AuthKind::Key => Ok(resolve_key(provider)?.map(anthropic::Auth::ApiKey)),
    }
}

fn endpoint_of(provider: &Provider) -> Result<String, ProviderError> {
    if let Some(ep) = &provider.endpoint {
        return Ok(ep.clone());
    }
    // SSH tunnel (if configured) takes precedence over type defaults.
    if let Some(local) = crate::tunnel::local_endpoint(provider)? {
        return Ok(local);
    }
    match provider.ptype {
        ProviderType::Api => Err(ProviderError::NoEndpoint(provider.id.clone())),
        ProviderType::Cli => Err(ProviderError::NoEndpoint(provider.id.clone())),
        ProviderType::OllamaRemote => Err(ProviderError::Api(format!(
            "provider '{}' is ollama-remote but has no tunnel spec (re-register with --tunnel user@host[:port])",
            provider.id
        ))),
        ProviderType::OllamaLocal => Ok(ollama::DEFAULT_LOCAL_BASE.to_string()),
        ProviderType::OllamaCloud => Ok(ollama::DEFAULT_CLOUD_BASE.to_string()),
    }
}

pub fn chat(provider: &Provider, req: &ChatRequest) -> Result<ChatResponse, ProviderError> {
    let http = http_client_long()?;
    let base = endpoint_of(provider)?;
    match provider.ptype {
        ProviderType::Api => {
            // Anthropic-style endpoints are recognized by host or path
            // (api.anthropic.com, api.moonshot.ai/anthropic, …); everything
            // else is treated as OpenAI-compatible.
            if base.contains("anthropic") {
                let auth = resolve_anthropic_auth(provider)?;
                anthropic::chat(&http, &base, auth, req)
            } else {
                let key = resolve_key(provider)?;
                openai::chat(&http, &base, key.as_deref(), req)
            }
        }
        ProviderType::OllamaLocal | ProviderType::OllamaCloud | ProviderType::OllamaRemote => {
            let key = resolve_key(provider)?;
            ollama::chat(&http, &base, key.as_deref(), req)
        }
        ProviderType::Cli => Err(ProviderError::Api(
            "CLI providers are wrapped via PTY adapters (M5), not HTTP chat".into(),
        )),
    }
}

pub fn list_models(provider: &Provider) -> Result<Vec<Model>, ProviderError> {
    let http = http_client()?;
    let base = endpoint_of(provider)?;
    match provider.ptype {
        ProviderType::Api => {
            if base.contains("anthropic") {
                let auth = resolve_anthropic_auth(provider)?;
                anthropic::list_models(&http, &base, auth)
            } else {
                let key = resolve_key(provider)?;
                openai::list_models(&http, &base, key.as_deref())
            }
        }
        ProviderType::OllamaLocal | ProviderType::OllamaCloud | ProviderType::OllamaRemote => {
            let key = resolve_key(provider)?;
            ollama::list_models(&http, &base, key.as_deref())
        }
        ProviderType::Cli => Ok(Vec::new()),
    }
}

/// Variant taking the key directly (used by the Add-Provider wizard before
/// the secret is stored). OAuth providers ignore the key argument.
pub fn list_models_with_key(
    provider: &Provider,
    key: Option<&str>,
) -> Result<Vec<Model>, ProviderError> {
    let http = http_client()?;
    let base = endpoint_of(provider)?;
    match provider.ptype {
        ProviderType::Api => {
            if base.contains("anthropic") {
                let auth = match provider.auth {
                    AuthKind::OAuth => resolve_anthropic_auth(provider)?,
                    AuthKind::Key => key.map(|k| anthropic::Auth::ApiKey(k.to_string())),
                };
                anthropic::list_models(&http, &base, auth)
            } else {
                openai::list_models(&http, &base, key)
            }
        }
        ProviderType::OllamaLocal | ProviderType::OllamaCloud | ProviderType::OllamaRemote => {
            ollama::list_models(&http, &base, key)
        }
        ProviderType::Cli => Ok(Vec::new()),
    }
}

/// Tool-calling chat (FR-12.1) — the runtime's agent loop uses this.
pub fn chat_tools(
    provider: &Provider,
    req: &crate::chat::ToolsChatRequest,
) -> Result<crate::chat::ToolsChatResponse, ProviderError> {
    let http = http_client_long()?;
    let base = endpoint_of(provider)?;
    match provider.ptype {
        ProviderType::Api => {
            if base.contains("anthropic") {
                let auth = resolve_anthropic_auth(provider)?;
                anthropic::chat_tools(&http, &base, auth, req)
            } else {
                let key = resolve_key(provider)?;
                openai::chat_tools(&http, &base, key.as_deref(), req)
            }
        }
        ProviderType::OllamaLocal | ProviderType::OllamaCloud | ProviderType::OllamaRemote => {
            let key = resolve_key(provider)?;
            ollama::chat_tools(&http, &base, key.as_deref(), req)
        }
        ProviderType::Cli => Err(ProviderError::Api(
            "CLI providers are wrapped via PTY adapters (M5), not HTTP chat".into(),
        )),
    }
}

/// Health check (FR-1.3): model listing round-trip with latency.
pub fn health(provider: &Provider) -> Health {
    let started = std::time::Instant::now();
    match list_models(provider) {
        Ok(models) => Health {
            ok: true,
            latency_ms: started.elapsed().as_millis() as u64,
            models: models.len(),
            detail: "ok".into(),
        },
        Err(e) => Health {
            ok: false,
            latency_ms: started.elapsed().as_millis() as u64,
            models: 0,
            detail: e.to_string(),
        },
    }
}

/// Refresh the persisted capability matrix for a provider (FR-1.2/FR-1.4).
pub fn refresh_models(provider_id: &str) -> Result<usize, ProviderError> {
    let mut reg = registry::ProviderRegistry::load()?;
    let provider = reg
        .get(provider_id)
        .ok_or_else(|| ProviderError::NotFound(provider_id.into()))?
        .clone();
    let discovered = list_models(&provider)?;
    let mut updated = provider;
    updated.models = merge_models(&updated.models, discovered);
    let n = updated.models.len();
    reg.upsert(updated);
    reg.save()?;
    Ok(n)
}

/// Combine a fresh discovery with the models the user added by hand.
///
/// Discovery is authoritative for everything it returns. Manual entries it
/// does not mention are kept, because the endpoint not listing a model does
/// not mean the model cannot be called — that is the whole reason to add one.
/// A manual entry the API has started advertising stops being manual: it is
/// now discoverable and should track whatever the API reports.
pub fn merge_models(existing: &[Model], discovered: Vec<Model>) -> Vec<Model> {
    let mut out = discovered;
    for m in existing.iter().filter(|m| m.manual) {
        if !out.iter().any(|d| d.id == m.id) {
            out.push(m.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::merge_models;
    use rolen_core::types::Model;

    fn model(id: &str, manual: bool) -> Model {
        Model {
            id: id.into(),
            manual,
            ..Default::default()
        }
    }

    #[test]
    fn discovery_replaces_discovered_models() {
        let existing = vec![model("old", false), model("kept", false)];
        let out = merge_models(&existing, vec![model("kept", false), model("new", false)]);
        let ids: Vec<&str> = out.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["kept", "new"]);
    }

    #[test]
    fn a_manual_model_survives_a_refresh_that_omits_it() {
        // The whole point: kimi's /models does not list k3-256k, but it works.
        let existing = vec![model("kimi-k3", false), model("kimi-k3-256k", true)];
        let out = merge_models(&existing, vec![model("kimi-k3", false)]);
        let ids: Vec<&str> = out.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["kimi-k3", "kimi-k3-256k"]);
        assert!(out.iter().find(|m| m.id == "kimi-k3-256k").unwrap().manual);
    }

    #[test]
    fn a_manual_model_the_api_now_lists_stops_being_manual() {
        let existing = vec![model("kimi-k3-256k", true)];
        let out = merge_models(&existing, vec![model("kimi-k3-256k", false)]);
        assert_eq!(out.len(), 1, "must not be duplicated");
        assert!(!out[0].manual);
    }

    #[test]
    fn an_empty_discovery_still_keeps_manual_models() {
        // A provider whose /models endpoint is broken or empty must not
        // silently drop the models the user added by hand.
        let existing = vec![model("api-one", false), model("hand-added", true)];
        let out = merge_models(&existing, vec![]);
        let ids: Vec<&str> = out.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["hand-added"]);
    }
}
