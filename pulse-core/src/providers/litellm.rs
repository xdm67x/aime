//! LiteLLM gateway provider (https://docs.litellm.ai/docs/proxy): a
//! self-hosted OpenAI-compatible proxy. URL and API key are user settings;
//! the base URL defaults to the quick-start docker default.

use super::{base_body, fetch_model_list, ChatRequest, Model, Provider};
use crate::{config, db};
use async_trait::async_trait;

pub struct LiteLlm;

fn base_url() -> String {
    db::get_setting("litellm_base_url")
        .ok()
        .flatten()
        .filter(|u| !u.trim().is_empty())
        .map(|u| u.trim().trim_end_matches('/').to_string())
        .unwrap_or_else(|| "http://localhost:4000/v1".into())
}

#[async_trait]
impl Provider for LiteLlm {
    fn name(&self) -> &'static str {
        "LiteLLM"
    }
    fn prefix(&self) -> &'static str {
        "LiteLLM - "
    }
    fn key_setting(&self) -> &'static str {
        "litellm"
    }

    fn chat_setup(
        &self,
        req: &ChatRequest,
        key: &str,
    ) -> (String, Vec<(&'static str, String)>, serde_json::Value) {
        (
            format!("{}/chat/completions", base_url()),
            vec![("Authorization", format!("Bearer {key}"))],
            base_body(req),
        )
    }

    async fn models(&self) -> Result<Vec<Model>, String> {
        let key = config::api_key("litellm")?.unwrap_or_default();
        fetch_model_list(
            format!("{}/models", base_url()),
            vec![("Authorization", format!("Bearer {key}"))],
            self.name(),
            self.prefix(),
        )
        .await
    }
}
