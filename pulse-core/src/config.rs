use crate::db;

pub fn api_key(provider: &str) -> Result<Option<String>, String> {
    db::get_setting(&format!("{provider}_api_key"))
}

pub fn save_api_key(provider: &str, key: &str) -> Result<(), String> {
    if !matches!(
        provider,
        "openrouter" | "opencode" | "litellm" | "mistral" | "github"
    ) {
        return Err(format!("Unknown provider: {provider}"));
    }
    db::set_setting(&format!("{provider}_api_key"), key.trim())
}

pub fn get_api_key(provider: &str) -> Result<Option<String>, String> {
    api_key(provider)
}

/// Base URL for a self-hosted provider gateway (e.g. LiteLLM proxy).
pub fn base_url(provider: &str) -> Result<Option<String>, String> {
    db::get_setting(&format!("{provider}_base_url"))
}

pub fn save_base_url(provider: &str, url: &str) -> Result<(), String> {
    if !matches!(provider, "litellm") {
        return Err(format!("Unknown provider: {provider}"));
    }
    db::set_setting(&format!("{}_base_url", provider), url.trim())
}

pub fn get_base_url(provider: &str) -> Result<Option<String>, String> {
    base_url(provider)
}

/* ---- the provider set with `pulse provider use <url> <key>` ---- */

/// The base URL of the configured OpenAI-compatible provider, e.g.
/// `https://api.openai.com/v1` or a LiteLLM proxy. Trailing slashes are
/// stripped when saved. The provider counts as configured once this is set —
/// the API key is optional (gateways may run without auth).
pub fn provider_url() -> Result<Option<String>, String> {
    Ok(db::get_setting("provider_url")?.filter(|u| !u.trim().is_empty()))
}

/// The API key of the configured provider. May be unset/empty.
pub fn provider_api_key() -> Result<Option<String>, String> {
    db::get_setting("provider_api_key")
}

/// Point Pulse at one OpenAI-compatible provider. Empty key = no auth.
pub fn save_provider(url: &str, api_key: &str) -> Result<(), String> {
    let url = url.trim().trim_end_matches('/');
    if url.is_empty() {
        return Err("Provider URL cannot be empty".into());
    }
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(format!(
            "Provider URL must start with http:// or https:// — got: {url}"
        ));
    }
    db::set_setting("provider_url", url)?;
    db::set_setting("provider_api_key", api_key.trim())
}
