use crate::config;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Token usage reported by OpenRouter for one completion.
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
/// token usage.
pub struct ChatResult {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Usage,
}

/// One chat completion against OpenRouter. Retries once on failure; when
/// `json_mode` is set the retry drops `response_format` (some models reject it).
/// When `tools` is set the reply may carry tool calls instead of text.
pub async fn chat_completion(
    model: &str,
    messages: &[serde_json::Value],
    fallbacks: &[String],
    temperature: Option<f64>,
    max_tokens: Option<u32>,
    json_mode: bool,
    tools: Option<&[serde_json::Value]>,
) -> Result<ChatResult, String> {
    let key = config::openrouter_key()?.ok_or("No OpenRouter API key configured")?;
    let mut body = serde_json::json!({ "model": model, "messages": messages });
    if let Some(t) = temperature {
        body["temperature"] = serde_json::json!(t);
    }
    if let Some(m) = max_tokens {
        body["max_tokens"] = serde_json::json!(m);
    }
    if !fallbacks.is_empty() {
        // OpenRouter fallback routing: tried in order if the primary fails
        body["models"] = serde_json::json!(fallbacks);
    }
    if json_mode {
        body["response_format"] = serde_json::json!({ "type": "json_object" });
    }
    if let Some(tools) = tools {
        body["tools"] = serde_json::json!(tools);
        body["tool_choice"] = serde_json::json!("auto");
    }

    let client = reqwest::Client::new();
    async fn once(
        client: &reqwest::Client,
        key: &str,
        body: &serde_json::Value,
    ) -> Result<ChatResult, String> {
        let resp: serde_json::Value = client
            .post("https://openrouter.ai/api/v1/chat/completions")
            .header("Authorization", format!("Bearer {key}"))
            .header("HTTP-Referer", "https://pulse.dev")
            .header("X-Title", "Pulse")
            .json(body)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| format!("OpenRouter request failed: {e}"))?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        let message = &resp["choices"][0]["message"];
        if message.is_null() {
            return Err(format!("Unexpected OpenRouter response: {resp}"));
        }
        let content = message["content"].as_str().unwrap_or("").to_string();
        let tool_calls = message
            .get("tool_calls")
            .and_then(|t| t.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|tc| {
                        Some(ToolCall {
                            id: tc["id"].as_str()?.to_string(),
                            name: tc["function"]["name"].as_str()?.to_string(),
                            arguments: tc["function"]["arguments"]
                                .as_str()
                                .unwrap_or("{}")
                                .to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let usage: Usage =
            serde_json::from_value(resp.get("usage").cloned().unwrap_or_default())
                .unwrap_or_default();
        Ok(ChatResult {
            content,
            tool_calls,
            usage,
        })
    }
    for attempt in 0..2 {
        match once(&client, &key, &body).await {
            Ok(out) => return Ok(out),
            Err(e) if attempt == 0 => {
                if json_mode {
                    body.as_object_mut().unwrap().remove("response_format");
                }
                tokio::time::sleep(Duration::from_millis(400)).await;
                let _ = e;
            }
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

const MODELS_TTL: Duration = Duration::from_secs(15 * 60);

static MODELS_CACHE: Mutex<Option<(Instant, Vec<Model>)>> = Mutex::new(None);

#[derive(Serialize, Deserialize)]
struct ModelsResp {
    data: Vec<Model>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
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

async fn fetch_models() -> Result<Vec<Model>, String> {
    let key = config::openrouter_key()?.unwrap_or_default();
    let resp: ModelsResp = reqwest::Client::new()
        .get("https://openrouter.ai/api/v1/models")
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("OpenRouter request failed: {e}"))?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let mut models = resp.data;
    models.sort_by(|a, b| a.id.cmp(&b.id));
    *MODELS_CACHE.lock().unwrap() = Some((Instant::now(), models.clone()));
    Ok(models)
}

fn cached_models() -> Option<Vec<Model>> {
    MODELS_CACHE
        .lock()
        .unwrap()
        .as_ref()
        .filter(|(t, _)| t.elapsed() < MODELS_TTL)
        .map(|(_, m)| m.clone())
}

#[tauri::command]
pub async fn list_models() -> Result<Vec<Model>, String> {
    if let Some(models) = cached_models() {
        return Ok(models);
    }
    fetch_models().await
}

/// Refresh the models cache in the background every 15 minutes.
pub fn spawn_refresh_loop() {
    tauri::async_runtime::spawn(async {
        loop {
            tokio::time::sleep(MODELS_TTL).await;
            if let Err(e) = fetch_models().await {
                eprintln!("models refresh failed: {e}");
            }
        }
    });
}

