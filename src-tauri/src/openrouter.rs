use crate::{config, db};
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

/// One chat completion against OpenRouter. Retries once on failure; when
/// `json_mode` is set the retry drops `response_format` (some models reject it).
/// Returns the reply text and token usage.
pub async fn chat_completion(
    model: &str,
    messages: &[serde_json::Value],
    fallbacks: &[String],
    temperature: Option<f64>,
    max_tokens: Option<u32>,
    json_mode: bool,
) -> Result<(String, Usage), String> {
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

    let client = reqwest::Client::new();
    async fn once(
        client: &reqwest::Client,
        key: &str,
        body: &serde_json::Value,
    ) -> Result<(String, Usage), String> {
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
        let content = resp["choices"][0]["message"]["content"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| format!("Unexpected OpenRouter response: {resp}"))?;
        let usage: Usage =
            serde_json::from_value(resp.get("usage").cloned().unwrap_or_default())
                .unwrap_or_default();
        Ok((content, usage))
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

#[tauri::command]
pub async fn send_message(
    beat_id: Option<i64>,
    model: String,
    content: String,
) -> Result<String, String> {
    // persist the user message even if the call fails — it was sent
    if let Some(beat_id) = beat_id {
        db::append_messages(
            beat_id,
            vec![serde_json::json!({"role": "user", "content": content})],
        )?;
    }
    let (reply, _usage) = chat_completion(
        &model,
        &[serde_json::json!({"role": "user", "content": content})],
        &[],
        None,
        None,
        false,
    )
    .await?;
    if let Some(beat_id) = beat_id {
        db::append_messages(
            beat_id,
            vec![serde_json::json!({"role": "assistant", "model": model, "content": reply})],
        )?;
    }
    Ok(reply)
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
