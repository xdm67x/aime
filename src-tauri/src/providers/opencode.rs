//! OpenCode Go provider (https://opencode.ai/docs/go/): $10/mo subscription
//! exposing open coding models behind an OpenAI-compatible API at
//! https://opencode.ai/zen/go/v1.

use super::{base_body, fetch_model_list, send_chat, ChatRequest, ChatResult, Model, Provider};
use async_trait::async_trait;

const BASE_URL: &str = "https://opencode.ai/zen/go/v1";

pub struct OpenCode;

#[async_trait]
impl Provider for OpenCode {
    fn name(&self) -> &'static str {
        "OpenCode Go"
    }
    fn prefix(&self) -> &'static str {
        "OpenCode - "
    }
    fn key_setting(&self) -> &'static str {
        "opencode"
    }

    async fn chat(&self, req: &ChatRequest, key: &str) -> Result<ChatResult, String> {
        let body = base_body(req);
        let r = reqwest::Client::new()
            .post(format!("{BASE_URL}/chat/completions"))
            .header("Authorization", format!("Bearer {key}"))
            // Go asks clients to identify themselves and send a stable session
            // id per conversation for routing/prompt caching.
            // ponytail: session id is per-app-run; per-beat ids if routing
            // quality matters
            .header("User-Agent", "Pulse/1.0")
            .header("x-opencode-session", "pulse-default");
        send_chat(r, &body, self.name()).await
    }

    async fn models(&self) -> Result<Vec<Model>, String> {
        // /models is public — no key needed to list, only to run
        fetch_model_list(
            format!("{BASE_URL}/models"),
            vec![("User-Agent", "Pulse/1.0".into())],
            self.name(),
            self.prefix(),
        )
        .await
    }
}
