//! Wire types shared by every provider: chat requests/results, model
//! metadata, token usage. Kept separate from the `Provider` trait so the
//! data shapes can evolve without touching dispatch.

use serde::{Deserialize, Serialize};

/// Token usage reported by the provider for one completion.
#[derive(Clone, Deserialize, Serialize, Default)]
pub struct Usage {
    #[serde(default)]
    pub prompt_tokens: u64,
    #[serde(default)]
    pub completion_tokens: u64,
}

/// One tool call requested by the model.
#[derive(Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

/// One chat completion result: reply text, any requested tool calls, and
/// token usage. `finish_reason` is the provider's stop reason (`stop`,
/// `tool_calls`, `length`, …) — the harness needs it to tell a real final
/// answer from a truncated or dropped-tool-calls turn.
pub struct ChatResult {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Usage,
    pub finish_reason: Option<String>,
}

/// One chat request, owned so the shared retry loop can mutate it.
pub struct ChatRequest {
    /// Bare model id (provider prefix already stripped).
    pub model: String,
    pub messages: Vec<serde_json::Value>,
    /// Alternate models tried in order when the primary fails.
    pub fallbacks: Vec<String>,
    pub temperature: Option<f64>,
    pub max_tokens: Option<u32>,
    /// Ask for JSON output via `response_format`.
    pub json_mode: bool,
    /// OpenAI-style tool schemas; the reply may carry tool calls.
    pub tools: Option<Vec<serde_json::Value>>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub context_length: Option<u64>,
    #[serde(default)]
    pub pricing: Pricing,
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct Pricing {
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub completion: String,
}

/// OpenAI-style `/models` responses vary: OpenRouter carries
/// `context_length` + `pricing.{prompt,completion}` (per-token strings),
/// LiteLLM proxies carry `max_input_tokens`/`max_tokens` +
/// `input_cost_per_token`/`output_cost_per_token` (per-token floats), and
/// others (OpenCode Go) carry none of it. `Model::from_json` normalizes all
/// of these so usage pricing and context fill work for every provider.
impl Model {
    pub fn from_json(v: &serde_json::Value) -> Self {
        let mut m = Self {
            id: v["id"].as_str().unwrap_or_default().to_string(),
            name: v["name"].as_str().unwrap_or_default().to_string(),
            context_length: v["context_length"].as_u64().or_else(|| {
                v["max_input_tokens"]
                    .as_u64()
                    .or_else(|| v["max_tokens"].as_u64())
                    .or_else(|| v["max_input_tokens"].as_str().and_then(|s| s.parse().ok()))
            }),
            pricing: Pricing {
                prompt: v["pricing"]["prompt"]
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_default(),
                completion: v["pricing"]["completion"]
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_default(),
            },
        };
        if m.pricing.prompt.is_empty() {
            if let Some(p) = v["input_cost_per_token"].as_f64() {
                m.pricing.prompt = p.to_string();
            }
        }
        if m.pricing.completion.is_empty() {
            if let Some(c) = v["output_cost_per_token"].as_f64() {
                m.pricing.completion = c.to_string();
            }
        }
        if m.name.is_empty() {
            m.name = m.id.clone();
        }
        m
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) struct ModelsResp {
    pub data: Vec<serde_json::Value>,
}
