//! The provider configured by `aime provider use <url> <key>`: any
//! OpenAI-compatible endpoint (OpenAI, LiteLLM, Ollama's `/v1`, vLLM, …).
//!
//! Unlike the built-in providers there is nothing hardcoded — both the base
//! URL and the API key are user settings. The URL alone marks the provider as
//! configured; gateways without auth accept an empty key.

use super::{base_body, fetch_model_list, ChatRequest, Model, Provider};
use crate::{config, db};
use async_trait::async_trait;

pub struct Custom;

/// The configured base URL, trailing slashes stripped.
fn base_url() -> String {
    config::provider_url()
        .ok()
        .flatten()
        .map(|u| u.trim().trim_end_matches('/').to_string())
        .unwrap_or_default()
}

/// Headers for one request; `Authorization` only when a key is set — a
/// gateway without auth rejects an empty `Bearer` header.
fn headers(key: &str) -> Vec<(&'static str, String)> {
    match key.trim().is_empty() {
        true => vec![],
        false => vec![("Authorization", format!("Bearer {key}"))],
    }
}

#[async_trait]
impl Provider for Custom {
    fn name(&self) -> &'static str {
        "Custom"
    }
    fn prefix(&self) -> &'static str {
        "Custom - "
    }
    /// `config::api_key("provider")` reads the `provider_api_key` setting.
    fn key_setting(&self) -> &'static str {
        "provider"
    }
    /// Configured once a URL is set — the key is optional (auth-less gateways).
    fn configured(&self) -> bool {
        config::provider_url().ok().flatten().is_some()
    }
    /// A gateway may run without auth; a missing key is not an error.
    fn keyless(&self) -> bool {
        true
    }

    fn chat_setup(
        &self,
        req: &ChatRequest,
        key: &str,
    ) -> (String, Vec<(&'static str, String)>, serde_json::Value) {
        (
            format!("{}/chat/completions", base_url()),
            headers(key),
            base_body(req),
        )
    }

    async fn models(&self) -> Result<Vec<Model>, String> {
        let key = db::get_setting("provider_api_key")?.unwrap_or_default();
        fetch_model_list(
            format!("{}/models", base_url()),
            headers(&key),
            self.name(),
            self.prefix(),
        )
        .await
    }
}
