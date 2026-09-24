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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_provider_target() {
        // known provider names, case-insensitive → key only
        for name in KNOWN_PROVIDERS {
            assert_eq!(
                parse_provider_target(name).unwrap(),
                ProviderTarget::Known(name)
            );
        }
        assert_eq!(
            parse_provider_target("OpenRouter").unwrap(),
            ProviderTarget::Known("openrouter")
        );
        // a URL → the custom provider
        assert_eq!(
            parse_provider_target("https://api.openai.com/v1").unwrap(),
            ProviderTarget::Url("https://api.openai.com/v1".into())
        );
        // anything else is an error naming the valid forms
        let err = parse_provider_target("grok").unwrap_err();
        assert!(err.contains("Unknown provider: grok"));
        assert!(err.contains("litellm, mistral, opencode, openrouter"));
        assert!(parse_provider_target("localhost:4000").is_err());
    }
}

/* ---- the provider set with `pulse provider use` ---- */

/// Providers whose host is built in, so `pulse provider use <name> <api_key>`
/// configures them with an API key alone — no base URL needed.
pub const KNOWN_PROVIDERS: &[&str] = &["litellm", "mistral", "opencode", "openrouter"];

/// What `pulse provider use <target> <api_key>` configures.
#[derive(Debug, PartialEq, Eq)]
pub enum ProviderTarget {
    /// A [`KNOWN_PROVIDERS`] entry — only the API key is stored.
    Known(&'static str),
    /// A custom OpenAI-compatible base URL — URL + API key are stored.
    Url(String),
}

/// Parse the first argument of `pulse provider use`: one of
/// [`KNOWN_PROVIDERS`] (case-insensitive) or an http(s) base URL.
pub fn parse_provider_target(target: &str) -> Result<ProviderTarget, String> {
    let t = target.trim();
    if let Some(name) = KNOWN_PROVIDERS.iter().find(|p| p.eq_ignore_ascii_case(t)) {
        return Ok(ProviderTarget::Known(name));
    }
    if t.starts_with("http://") || t.starts_with("https://") {
        return Ok(ProviderTarget::Url(t.to_string()));
    }
    Err(format!(
        "Unknown provider: {t} — pass a base URL (https://…) or one of: {}",
        KNOWN_PROVIDERS.join(", ")
    ))
}

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
/// Also records "Custom" as the configured provider (`provider_name`).
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
    db::set_setting("provider_api_key", api_key.trim())?;
    db::set_setting("provider_name", "Custom")
}

/// Display name of the provider `pulse provider use` configured (e.g.
/// "Custom", "OpenRouter") — resolved against the registry by
/// `providers::configured_provider`. Unset for providers configured before
/// this setting existed; those fall back to Custom via `provider_url`.
pub fn provider_name() -> Result<Option<String>, String> {
    Ok(db::get_setting("provider_name")?.filter(|n| !n.trim().is_empty()))
}

/// Record which provider `pulse provider use` configured.
pub fn save_provider_name(name: &str) -> Result<(), String> {
    db::set_setting("provider_name", name.trim())
}
