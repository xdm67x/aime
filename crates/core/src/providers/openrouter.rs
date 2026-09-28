//! OpenRouter provider (OpenAI-compatible, https://openrouter.ai).

use super::{base_body, fetch_model_list, ChatRequest, Model, Provider};
use crate::config;
use async_trait::async_trait;
use serde_json::json;

const BASE_URL: &str = "https://openrouter.ai/api/v1";

pub struct OpenRouter;

#[async_trait]
impl Provider for OpenRouter {
    fn name(&self) -> &'static str {
        "OpenRouter"
    }
    fn prefix(&self) -> &'static str {
        "OpenRouter - "
    }
    fn key_setting(&self) -> &'static str {
        "openrouter"
    }

    fn chat_setup(
        &self,
        req: &ChatRequest,
        key: &str,
    ) -> (String, Vec<(&'static str, String)>, serde_json::Value) {
        let mut body = base_body(req);
        if !req.fallbacks.is_empty() {
            // OpenRouter fallback routing: tried in order if the primary fails
            body["models"] = json!(req.fallbacks);
        }
        (
            format!("{BASE_URL}/chat/completions"),
            vec![
                ("Authorization", format!("Bearer {key}")),
                ("HTTP-Referer", "https://pulse.dev".into()),
                ("X-Title", "Pulse".into()),
            ],
            body,
        )
    }

    async fn models(&self) -> Result<Vec<Model>, String> {
        let key = config::api_key("openrouter")?.unwrap_or_default();
        fetch_model_list(
            format!("{BASE_URL}/models"),
            vec![("Authorization", format!("Bearer {key}"))],
            self.name(),
            self.prefix(),
        )
        .await
    }
}
