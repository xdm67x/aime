//! Provider abstraction: each backend (OpenRouter, OpenCode Go, …) implements
//! the `Provider` trait — a model-id prefix, its API-key setting name, and two
//! async calls (`chat`, `models`). Adding a provider is one small file and one
//! line in `providers()`; dispatch, retry, caching and the UI contract are
//! shared here.
//!
//! Model ids handed around the app are prefixed with the provider's `prefix()`
//! (e.g. `OpenRouter - anthropic/claude…`, `OpenCode - kimi-k3`) so the picker
//! and saved config always show which provider a model routes to. Bare legacy
//! ids still route to OpenRouter.

pub mod opencode;
pub mod openrouter;

use crate::config;
use serde::{Deserialize, Serialize};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/* ---- shared types ---- */

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
/// token usage.
pub struct ChatResult {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Usage,
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

#[derive(Serialize, Deserialize)]
pub(crate) struct ModelsResp {
    pub data: Vec<Model>,
}

/* ---- provider trait + registry ---- */

#[async_trait::async_trait]
pub trait Provider: Send + Sync {
    /// Display name used in error messages.
    fn name(&self) -> &'static str;
    /// Prefix that marks a model id as belonging to this provider.
    fn prefix(&self) -> &'static str;
    /// Settings key holding this provider's API key.
    fn key_setting(&self) -> &'static str;
    /// One chat completion. `req.model` is the bare (unprefixed) model id.
    async fn chat(&self, req: &ChatRequest, key: &str) -> Result<ChatResult, String>;
    /// The provider's models, ids already prefixed.
    async fn models(&self) -> Result<Vec<Model>, String>;
}

fn providers() -> &'static [Box<dyn Provider>] {
    static P: OnceLock<Vec<Box<dyn Provider>>> = OnceLock::new();
    P.get_or_init(|| {
        vec![
            Box::new(openrouter::OpenRouter),
            Box::new(opencode::OpenCode),
        ]
    })
}

/// Resolve a prefixed model id to its provider, stripping the prefix. Bare
/// legacy ids (saved before prefixing) fall back to OpenRouter.
fn provider_for(model: &str) -> Result<(&'static dyn Provider, String), String> {
    for p in providers() {
        if let Some(id) = model.strip_prefix(p.prefix()) {
            return Ok((p.as_ref(), id.to_string()));
        }
    }
    Ok((providers()[0].as_ref(), model.to_string()))
}

/* ---- chat dispatch ---- */

/// One chat completion against whichever provider owns `model`. Retries once
/// on failure; the retry drops `json_mode` (some models reject
/// `response_format`). Providers implement the request themselves.
pub async fn chat_completion(
    model: &str,
    messages: &[serde_json::Value],
    fallbacks: &[String],
    temperature: Option<f64>,
    max_tokens: Option<u32>,
    json_mode: bool,
    tools: Option<&[serde_json::Value]>,
) -> Result<ChatResult, String> {
    let (p, model_id) = provider_for(model)?;
    let key = config::api_key(p.key_setting())?
        .ok_or_else(|| format!("No {} API key configured", p.name()))?;
    let mut req = ChatRequest {
        model: model_id,
        messages: messages.to_vec(),
        fallbacks: fallbacks.to_vec(),
        temperature,
        max_tokens,
        json_mode,
        tools: tools.map(|t| t.to_vec()),
    };
    for attempt in 0..2 {
        match p.chat(&req, &key).await {
            Ok(out) => return Ok(out),
            Err(e) if attempt == 0 => {
                req.json_mode = false;
                tokio::time::sleep(Duration::from_millis(400)).await;
                let _ = e;
            }
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

/// The OpenAI-compatible request body every provider shares; providers add
/// their own extras (fallbacks, provider headers) on top.
pub(crate) fn base_body(req: &ChatRequest) -> serde_json::Value {
    let mut body = serde_json::json!({ "model": req.model, "messages": req.messages });
    if let Some(t) = req.temperature {
        body["temperature"] = serde_json::json!(t);
    }
    if let Some(m) = req.max_tokens {
        body["max_tokens"] = serde_json::json!(m);
    }
    if req.json_mode {
        body["response_format"] = serde_json::json!({ "type": "json_object" });
    }
    if let Some(tools) = &req.tools {
        body["tools"] = serde_json::json!(tools);
        body["tool_choice"] = serde_json::json!("auto");
    }
    body
}

/// POST an OpenAI-compatible chat request and parse the standard response
/// shape (choices[0].message + usage).
pub(crate) async fn send_chat(
    req: reqwest::RequestBuilder,
    body: &serde_json::Value,
    name: &str,
) -> Result<ChatResult, String> {
    let resp: serde_json::Value = req
        .json(body)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("{name} request failed: {e}"))?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let message = &resp["choices"][0]["message"];
    if message.is_null() {
        return Err(format!("Unexpected {name} response: {resp}"));
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
        serde_json::from_value(resp.get("usage").cloned().unwrap_or_default()).unwrap_or_default();
    Ok(ChatResult {
        content,
        tool_calls,
        usage,
    })
}

/// GET an OpenAI-compatible `/models` list. Each provider passes its own
/// headers; ids get the provider prefix and name falls back to the id.
pub(crate) async fn fetch_model_list(
    url: String,
    headers: Vec<(&'static str, String)>,
    name: &str,
    prefix: &str,
) -> Result<Vec<Model>, String> {
    let mut req = reqwest::Client::new().get(url);
    for (k, v) in headers {
        req = req.header(k, v);
    }
    let resp: ModelsResp = req
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("{name} request failed: {e}"))?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    Ok(resp
        .data
        .into_iter()
        .map(|mut m| {
            if m.name.is_empty() {
                m.name = m.id.clone();
            }
            m.id = format!("{prefix}{}", m.id);
            m
        })
        .collect())
}

/* ---- merged model list + cache ---- */

const MODELS_TTL: Duration = Duration::from_secs(15 * 60);

static MODELS_CACHE: Mutex<Option<(Instant, Vec<Model>)>> = Mutex::new(None);

async fn fetch_models() -> Result<Vec<Model>, String> {
    let mut models = Vec::new();
    let mut errors = Vec::new();
    for p in providers() {
        match p.models().await {
            Ok(m) => models.extend(m),
            Err(e) => errors.push(format!("{}: {e}", p.name())),
        }
    }
    if models.is_empty() {
        return Err(errors.join("; "));
    }
    for e in &errors {
        eprintln!("models fetch failed: {e}");
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_for() {
        let (p, id) = provider_for("OpenCode - kimi-k3").unwrap();
        assert_eq!(p.name(), "OpenCode Go");
        assert_eq!(id, "kimi-k3");
        let (p, id) = provider_for("OpenRouter - anthropic/claude").unwrap();
        assert_eq!(p.name(), "OpenRouter");
        assert_eq!(id, "anthropic/claude");
        // legacy bare id falls back to OpenRouter, id unchanged
        let (p, id) = provider_for("anthropic/claude").unwrap();
        assert_eq!(p.name(), "OpenRouter");
        assert_eq!(id, "anthropic/claude");
    }
}
