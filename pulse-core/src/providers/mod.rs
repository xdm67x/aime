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

pub mod litellm;
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
        // LiteLLM-style numeric per-token costs.
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

/* ---- provider trait + registry ---- */

#[async_trait::async_trait]
pub trait Provider: Send + Sync {
    /// Display name used in error messages.
    fn name(&self) -> &'static str;
    /// Prefix that marks a model id as belonging to this provider.
    fn prefix(&self) -> &'static str;
    /// Settings key holding this provider's API key.
    fn key_setting(&self) -> &'static str;
    /// URL, headers and body extras for one chat POST. The shared `chat` and
    /// `chat_stream` paths add nothing else; streaming only adds `stream:true`
    /// on top. `req.model` is the bare (unprefixed) model id.
    fn chat_setup(
        &self,
        req: &ChatRequest,
        key: &str,
    ) -> (String, Vec<(&'static str, String)>, serde_json::Value);
    /// The provider's models, ids already prefixed.
    async fn models(&self) -> Result<Vec<Model>, String>;
}

fn providers() -> &'static [Box<dyn Provider>] {
    static P: OnceLock<Vec<Box<dyn Provider>>> = OnceLock::new();
    P.get_or_init(|| {
        vec![
            Box::new(openrouter::OpenRouter),
            Box::new(opencode::OpenCode),
            Box::new(litellm::LiteLlm),
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
/// `response_format`).
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
        match send_chat(p, &req, &key).await {
            Ok(out) => return Ok(out),
            Err(e) if attempt == 0 => {
                crate::log::warn(format!(
                    "[{}] chat completion failed, retrying without json_mode: {e}",
                    p.name()
                ));
                req.json_mode = false;
                tokio::time::sleep(Duration::from_millis(400)).await;
                let _ = e;
            }
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

async fn send_chat(p: &dyn Provider, req: &ChatRequest, key: &str) -> Result<ChatResult, String> {
    let (url, headers, body) = p.chat_setup(req, key);
    let mut r = reqwest::Client::new().post(url);
    for (k, v) in headers {
        r = r.header(k, v);
    }
    crate::log::info(format!(
        "[{}] chat request: model={} messages={} json_mode={} tools={}",
        p.name(),
        body["model"],
        body["messages"].as_array().map_or(0, Vec::len),
        body.get("response_format").is_some(),
        body.get("tools").is_some(),
    ));
    let resp = r
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("{} request failed: {e}", p.name()))?;
    let status = resp.status();
    // Read the body before checking status: an error body carries the
    // server's actual reason ("invalid model", "unknown field", …).
    let text = resp
        .text()
        .await
        .map_err(|e| format!("{}: reading response failed: {e}", p.name()))?;
    if !status.is_success() {
        let snippet = text.get(..2000).unwrap_or(&text);
        crate::log::warn(format!("[{}] HTTP {status}: {snippet}", p.name()));
        return Err(format!(
            "{} request failed: HTTP {status}: {snippet}",
            p.name()
        ));
    }
    crate::log::info(format!("[{}] HTTP {status}", p.name()));
    let resp: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("Unexpected {} response: {text} ({e})", p.name()))?;
    let message = &resp["choices"][0]["message"];
    if message.is_null() {
        return Err(format!("Unexpected {} response: {resp}", p.name()));
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
    let finish_reason = resp["choices"][0]["finish_reason"]
        .as_str()
        .map(str::to_string);
    Ok(ChatResult {
        content,
        tool_calls,
        usage,
        finish_reason,
    })
}

/// Streaming chat completion: `on_delta` receives each text chunk as it
/// arrives; the returned result carries the full content (and any tool
/// calls) once the stream ends. If the stream fails before any delta was
/// delivered, falls back to the non-streaming path (its retry included).
#[allow(clippy::too_many_arguments)]
pub async fn chat_completion_stream(
    model: &str,
    messages: &[serde_json::Value],
    fallbacks: &[String],
    temperature: Option<f64>,
    max_tokens: Option<u32>,
    json_mode: bool,
    tools: Option<&[serde_json::Value]>,
    on_delta: &mut (dyn FnMut(&str) + Send),
) -> Result<ChatResult, String> {
    let (p, model_id) = provider_for(model)?;
    let key = config::api_key(p.key_setting())?
        .ok_or_else(|| format!("No {} API key configured", p.name()))?;
    let req = ChatRequest {
        model: model_id,
        messages: messages.to_vec(),
        fallbacks: fallbacks.to_vec(),
        temperature,
        max_tokens,
        json_mode,
        tools: tools.map(|t| t.to_vec()),
    };
    let mut deltas = 0usize;
    let mut wrapped = |t: &str| {
        deltas += 1;
        on_delta(t);
    };
    let res = send_chat_stream(p, &req, &key, &mut wrapped).await;
    // a cancelled stream must not fall back to the (slower, whole-request)
    // non-streaming retry — that would undo the stop
    if res.is_err() && deltas == 0 && !crate::harness::cancelled() {
        crate::log::warn(format!(
            "[{}] stream failed before any delta; falling back to non-streaming completion",
            p.name()
        ));
        return chat_completion(
            model,
            messages,
            fallbacks,
            temperature,
            max_tokens,
            json_mode,
            tools,
        )
        .await;
    }
    res
}

/// POST an OpenAI-compatible chat request with `stream:true` and parse the
/// SSE chunk stream (text deltas + accumulated tool-call fragments + final
/// usage). Lines are complete UTF-8 spans (each ends with \n), so per-line
/// lossy decoding is safe even when chunks split mid-line.
async fn send_chat_stream(
    p: &dyn Provider,
    req: &ChatRequest,
    key: &str,
    on_delta: &mut (dyn FnMut(&str) + Send),
) -> Result<ChatResult, String> {
    let (url, headers, mut body) = p.chat_setup(req, key);
    body["stream"] = serde_json::json!(true);
    body["stream_options"] = serde_json::json!({ "include_usage": true });
    crate::log::info(format!(
        "[{}] stream request: model={} messages={} tools={}",
        p.name(),
        body["model"],
        body["messages"].as_array().map_or(0, Vec::len),
        body.get("tools").is_some(),
    ));
    let mut r = reqwest::Client::new().post(url);
    for (k, v) in headers {
        r = r.header(k, v);
    }
    let mut resp = r
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("{} request failed: {e}", p.name()))?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp
            .text()
            .await
            .map_err(|e| format!("{}: reading response failed: {e}", p.name()))?;
        let snippet = text.get(..2000).unwrap_or(&text);
        crate::log::warn(format!("[{}] HTTP {status}: {snippet}", p.name()));
        return Err(format!(
            "{} request failed: HTTP {status}: {snippet}",
            p.name()
        ));
    }
    let mut content = String::new();
    // streamed tool-call fragments arrive piecewise, keyed by index
    let mut calls: std::collections::BTreeMap<u64, ToolCall> = Default::default();
    let mut usage = Usage::default();
    let mut finish_reason: Option<String> = None;
    let mut buf: Vec<u8> = vec![];
    loop {
        if crate::harness::cancelled() {
            return Err(crate::harness::STOPPED.into());
        }
        let chunk = resp
            .chunk()
            .await
            .map_err(|e| format!("{} stream failed: {e}", p.name()))?;
        let Some(chunk) = chunk else { break };
        buf.extend_from_slice(&chunk);
        while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = buf.drain(..=pos).collect();
            let line = String::from_utf8_lossy(&line[..line.len() - 1]);
            let line = line.trim_end_matches('\r');
            let Some(rest) = line.strip_prefix("data:") else {
                continue; // comments, blank lines, unknown fields
            };
            let data = rest.trim();
            if data.trim() == "[DONE]" {
                continue;
            }
            let Ok(v) = serde_json::from_str::<serde_json::Value>(data) else {
                continue;
            };
            let (text, frags, u, fr) = parse_chunk(&v);
            if !text.is_empty() {
                content.push_str(&text);
                on_delta(&text);
            }
            for (idx, frag) in frags {
                let e = calls.entry(idx).or_insert_with(|| ToolCall {
                    id: String::new(),
                    name: String::new(),
                    arguments: String::new(),
                });
                e.id.push_str(&frag.id);
                e.name.push_str(&frag.name);
                e.arguments.push_str(&frag.arguments);
            }
            if let Some(u) = u {
                usage = u;
            }
            // last non-null stop reason wins (earlier chunks may not carry one)
            if fr.is_some() {
                finish_reason = fr;
            }
        }
    }
    let tool_calls: Vec<ToolCall> = calls.into_values().collect();
    crate::log::info(format!(
        "[{}] stream done: {} chars, {} tool calls, finish_reason={:?}",
        p.name(),
        content.len(),
        tool_calls.len(),
        finish_reason,
    ));
    Ok(ChatResult {
        content,
        tool_calls,
        usage,
        finish_reason,
    })
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

/// Parse one SSE `data:` JSON payload (OpenAI streaming shape) into a text
/// delta, streamed tool-call fragments keyed by index, optional usage, and
/// the stop reason.
type ParsedChunk = (String, Vec<(u64, ToolCall)>, Option<Usage>, Option<String>);

fn parse_chunk(v: &serde_json::Value) -> ParsedChunk {
    let mut text = String::new();
    if let Some(t) = v["choices"][0]["delta"]["content"].as_str() {
        text.push_str(t);
    }
    let frags = v["choices"][0]["delta"]["tool_calls"]
        .as_array()
        .map(|tcs| {
            tcs.iter()
                .map(|tc| {
                    (
                        tc["index"].as_u64().unwrap_or(0),
                        ToolCall {
                            id: tc["id"].as_str().unwrap_or("").into(),
                            name: tc["function"]["name"].as_str().unwrap_or("").into(),
                            arguments: tc["function"]["arguments"].as_str().unwrap_or("").into(),
                        },
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let usage = v
        .get("usage")
        .filter(|u| !u.is_null())
        .and_then(|u| serde_json::from_value(u.clone()).ok());
    let finish_reason = v["choices"][0]["finish_reason"]
        .as_str()
        .map(str::to_string);
    (text, frags, usage, finish_reason)
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
    let resp = req
        .send()
        .await
        .map_err(|e| format!("{name} request failed: {e}"))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| format!("{name}: reading response failed: {e}"))?;
    if !status.is_success() {
        let snippet = text.get(..2000).unwrap_or(&text);
        crate::log::warn(format!("[{name} models] HTTP {status}: {snippet}"));
        return Err(format!("{name} request failed: HTTP {status}: {snippet}"));
    }
    let resp: ModelsResp = serde_json::from_str(&text)
        .map_err(|e| format!("Unexpected {name} models response: {text} ({e})"))?;
    Ok(resp
        .data
        .iter()
        .map(|v| {
            let mut m = Model::from_json(v);
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
        crate::log::warn(format!("models fetch partially failed: {e}"));
    }
    crate::log::info(format!("fetched {} models from {} provider(s)", models.len(), providers().len()));
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

pub async fn list_models() -> Result<Vec<Model>, String> {
    if let Some(models) = cached_models() {
        return Ok(models);
    }
    fetch_models().await
}

/// Background loop refreshing the models cache every 15 minutes. Returns a
/// future that never completes — the host runtime decides how to drive it
/// (`tokio::spawn`, …).
pub async fn refresh_loop() {
    loop {
        tokio::time::sleep(MODELS_TTL).await;
        if let Err(e) = fetch_models().await {
            crate::log::warn(format!("models refresh failed: {e}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_chunk() {
        // text delta
        let (t, f, u, fr) = parse_chunk(&serde_json::json!(
            {"choices":[{"delta":{"content":"he"}}]}));
        assert_eq!(t, "he");
        assert!(f.is_empty() && u.is_none() && fr.is_none());
        // stop reason is surfaced
        let (_, _, _, fr) = parse_chunk(&serde_json::json!(
            {"choices":[{"delta":{},"finish_reason":"length"}]}));
        assert_eq!(fr.as_deref(), Some("length"));
        // tool-call fragment: pieces arrive split across chunks
        let (_, f, _, _) = parse_chunk(&serde_json::json!(
            {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1",
             "function":{"name":"grep","arguments":"{\"pa"}}]}}]}));
        assert_eq!(f[0].0, 0);
        assert_eq!(f[0].1.name, "grep");
        assert_eq!(f[0].1.arguments, "{\"pa");
        // final usage chunk (stream_options include_usage)
        let (_, _, u, _) = parse_chunk(&serde_json::json!(
            {"choices":[],"usage":{"prompt_tokens":3,"completion_tokens":5}}));
        assert_eq!(u.unwrap().completion_tokens, 5);
        // chunk without usage must not clobber the accumulated one
        let (_, _, u, _) = parse_chunk(&serde_json::json!({"choices":[{"delta":{}}]}));
        assert!(u.is_none());
    }

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
