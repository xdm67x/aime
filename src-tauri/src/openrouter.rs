use crate::config;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::time::{Duration, Instant};

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
pub async fn send_message(model: String, content: String) -> Result<String, String> {
    let key = config::openrouter_key()?.ok_or("No OpenRouter API key configured")?;
    let body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": content}],
    });
    let resp: serde_json::Value = reqwest::Client::new()
        .post("https://openrouter.ai/api/v1/chat/completions")
        .header("Authorization", format!("Bearer {key}"))
        .json(&body)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("OpenRouter request failed: {e}"))?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    resp["choices"][0]["message"]["content"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| format!("Unexpected OpenRouter response: {resp}"))
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
