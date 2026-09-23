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
