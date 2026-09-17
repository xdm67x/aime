use crate::config;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct ModelsResp {
    data: Vec<Model>,
}

#[derive(Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub context_length: Option<u64>,
    #[serde(default)]
    pub pricing: Pricing,
}

#[derive(Serialize, Deserialize, Default)]
pub struct Pricing {
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub completion: String,
}

#[tauri::command]
pub async fn list_models() -> Result<Vec<Model>, String> {
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
    Ok(models)
}
