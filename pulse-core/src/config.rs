use crate::db;
use serde::{Deserialize, Serialize};

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

/// The four model slots the harness routes between. Each is an OpenRouter
/// model id the user picks in Settings; an empty string means "not set".
const MODEL_CLASSIFIER: &str = "model_classifier";
const MODEL_HIGH: &str = "model_high";
const MODEL_BASE: &str = "model_base";
const MODEL_LOW: &str = "model_low";

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct ModelConfig {
    /// Free/low-cost model that classifies each prompt into a tier.
    #[serde(default)]
    pub classifier: String,
    /// Most capable model — high-stakes work with a reflexion pass.
    #[serde(default)]
    pub high: String,
    /// Implementation workhorse — typical coding/analysis.
    #[serde(default)]
    pub base: String,
    /// Low-cost model — simple, basic tasks.
    #[serde(default)]
    pub low: String,
}

impl ModelConfig {
    pub fn load() -> Result<Self, String> {
        Ok(ModelConfig {
            classifier: db::get_setting(MODEL_CLASSIFIER)?.unwrap_or_default(),
            high: db::get_setting(MODEL_HIGH)?.unwrap_or_default(),
            base: db::get_setting(MODEL_BASE)?.unwrap_or_default(),
            low: db::get_setting(MODEL_LOW)?.unwrap_or_default(),
        })
    }
}

pub fn get_model_config() -> Result<ModelConfig, String> {
    ModelConfig::load()
}

pub fn save_model_config(config: &ModelConfig) -> Result<(), String> {
    db::set_setting(MODEL_CLASSIFIER, config.classifier.trim())?;
    db::set_setting(MODEL_HIGH, config.high.trim())?;
    db::set_setting(MODEL_BASE, config.base.trim())?;
    db::set_setting(MODEL_LOW, config.low.trim())?;
    Ok(())
}
