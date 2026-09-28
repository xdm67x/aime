//! Mistral La Plateforme provider (https://docs.mistral.ai): an
//! OpenAI-compatible API at https://api.mistral.ai/v1.

use super::{base_body, fetch_model_list, ChatRequest, Model, Provider};
use crate::config;
use async_trait::async_trait;

const BASE_URL: &str = "https://api.mistral.ai/v1";

pub struct Mistral;

#[async_trait]
impl Provider for Mistral {
    fn name(&self) -> &'static str {
        "Mistral"
    }
    fn prefix(&self) -> &'static str {
        "Mistral - "
    }
    fn key_setting(&self) -> &'static str {
        "mistral"
    }

    fn chat_setup(
        &self,
        req: &ChatRequest,
        key: &str,
    ) -> (String, Vec<(&'static str, String)>, serde_json::Value) {
        (
            format!("{BASE_URL}/chat/completions"),
            vec![
                ("Authorization", format!("Bearer {key}")),
                ("User-Agent", "Aime/1.0".into()),
            ],
            base_body(req),
        )
    }

    async fn models(&self) -> Result<Vec<Model>, String> {
        let key = config::api_key("mistral")?.unwrap_or_default();
        fetch_model_list(
            format!("{BASE_URL}/models"),
            vec![
                ("Authorization", format!("Bearer {key}")),
                ("User-Agent", "Aime/1.0".into()),
            ],
            self.name(),
            self.prefix(),
        )
        .await
    }
}
