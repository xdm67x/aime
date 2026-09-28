//! Provider abstraction: each backend (OpenRouter, OpenCode Go, Mistral, …)
//! implements the `Provider` trait — a model-id prefix, its API-key setting name, and two
//! async calls (`chat`, `models`). Adding a provider is one small file and one
//! line in `providers()`; dispatch, retry, caching and the UI contract are
//! shared here.
//!
//! Model ids handed around the app are prefixed with the provider's `prefix()`
//! (e.g. `OpenRouter - anthropic/claude…`, `OpenCode - kimi-k3`) so the picker
//! and saved config always show which provider a model routes to. Bare ids
//! route to the provider set with `aime provider use <url> <key>` when one
//! is configured, else to OpenRouter.

pub mod custom;
pub mod litellm;
pub mod mistral;
pub mod opencode;
pub mod openrouter;

mod types;

use types::ModelsResp;
pub use types::{ChatRequest, ChatResult, Model, Pricing, ToolCall, Usage};

use crate::config;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/* ---- provider trait + registry ---- */

#[async_trait::async_trait]
pub trait Provider: Send + Sync {
    /// Display name used in error messages.
    fn name(&self) -> &'static str;
    /// Prefix that marks a model id as belonging to this provider.
    fn prefix(&self) -> &'static str;
    /// Settings key holding this provider's API key.
    fn key_setting(&self) -> &'static str;
    /// True when the provider is usable: its API key is set. Unconfigured
    /// providers are skipped when listing models, so configuring a single
    /// provider (e.g. LiteLLM) is enough to run Aime on it alone.
    fn configured(&self) -> bool {
        config::api_key(self.key_setting())
            .ok()
            .flatten()
            .is_some_and(|k| !k.trim().is_empty())
    }
    /// True for providers that work without a key (e.g. an unauthenticated
    /// LiteLLM proxy); their dispatch paths accept an empty key.
    fn keyless(&self) -> bool {
        false
    }
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
            Box::new(mistral::Mistral),
            Box::new(custom::Custom),
        ]
    })
}

/// Registered providers in registry order: display name + prefix. Keys are
/// set from the TUI with `/key <name> <value>`.
pub fn provider_names() -> Vec<&'static str> {
    providers().iter().map(|p| p.name()).collect()
}

/// Resolve a provider by its display name (case-insensitive), e.g. the
/// `<name>` argument of `/models <name>`.
pub fn provider_by_name(name: &str) -> Option<&'static dyn Provider> {
    let n = name.trim().to_lowercase();
    providers()
        .iter()
        .map(|p| p.as_ref())
        .find(|p| p.name().to_lowercase() == n)
}

/// Resolve a provider by its settings key (`key_setting`) — the names
/// `aime provider use` accepts: "openrouter", "opencode", "litellm",
/// "mistral", plus "custom" for the URL-configured provider.
pub fn provider_by_key(key: &str) -> Option<&'static dyn Provider> {
    let k = key.trim().to_lowercase();
    if k == "custom" {
        return provider_by_name("Custom");
    }
    providers()
        .iter()
        .map(|p| p.as_ref())
        .find(|p| p.key_setting() == k)
}

/// The provider `aime provider use` configured — the one bare model ids
/// route to and `aime models` lists. Configs made before the provider name
/// was stored fall back to the Custom provider when a URL is set.
pub fn configured_provider() -> Option<&'static dyn Provider> {
    if let Some(name) = config::provider_name().ok().flatten() {
        if let Some(p) = provider_by_name(&name) {
            return Some(p);
        }
    }
    if config::provider_url().ok().flatten().is_some() {
        return provider_by_name("Custom");
    }
    None
}

/// The models of one provider, ids already prefixed. Unconfigured providers
/// return an error naming the key to set.
pub async fn list_models_of(name: &str) -> Result<Vec<Model>, String> {
    let p = provider_by_name(name).ok_or_else(|| {
        format!(
            "Unknown provider: {name} — known: {}",
            provider_names().join(", ")
        )
    })?;
    if !p.configured() {
        return Err(missing_key_error(p));
    }
    p.models().await
}

/// Resolve a prefixed model id to its provider, stripping the prefix. Bare
/// ids — the normal case for the workflow CLI — route to the provider
/// configured with `aime provider use`, so workflow files never need a
/// provider name before the model id; an explicit prefix still wins.
/// Without a configured provider a bare id falls back to OpenRouter —
/// unless OpenRouter is not configured and exactly one other provider is:
/// then the id belongs to that provider (a LiteLLM-only setup must not be
/// told to configure an OpenRouter key).
fn provider_for(model: &str) -> Result<(&'static dyn Provider, String), String> {
    for p in providers() {
        if let Some(id) = model.strip_prefix(p.prefix()) {
            return Ok((p.as_ref(), id.to_string()));
        }
    }
    if let Some(p) = configured_provider().filter(|p| p.configured()) {
        return Ok((p, model.to_string()));
    }
    let configured: Vec<&dyn Provider> = providers()
        .iter()
        .map(|p| p.as_ref())
        .filter(|p| p.configured())
        .collect();
    if configured.len() == 1 {
        return Ok((configured[0], model.to_string()));
    }
    Ok((providers()[0].as_ref(), model.to_string()))
}

/* ---- chat dispatch ---- */

/// The error shown when a chat is routed to a provider whose key is missing.
/// Names the exact command so the fix is one `aime provider use` away.
fn missing_key_error(p: &dyn Provider) -> String {
    let hint = if p.key_setting() == "provider" {
        "aime provider use <url> <api_key>".to_string()
    } else {
        format!("aime provider use {} <api_key>", p.key_setting())
    };
    format!("No {} API key configured — set it with: {hint}", p.name())
}

/// The key for one chat against a provider: the configured key, or an empty
/// string when the provider is keyless and no key is set.
fn dispatch_key(p: &dyn Provider) -> Result<String, String> {
    match config::api_key(p.key_setting())? {
        Some(k) if !k.trim().is_empty() => Ok(k),
        _ if p.keyless() => Ok(String::new()),
        _ => Err(missing_key_error(p)),
    }
}

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
    let key = dispatch_key(p)?;
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
    let key = dispatch_key(p)?;
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
    let mut configured = 0usize;
    for p in providers() {
        if !p.configured() {
            continue;
        }
        configured += 1;
        match p.models().await {
            Ok(m) => models.extend(m),
            Err(e) => errors.push(format!("{}: {e}", p.name())),
        }
    }
    if configured == 0 {
        return Err("No provider configured — set one with: aime provider use \
             <url|litellm|mistral|opencode|openrouter> <api_key>"
            .into());
    }
    if models.is_empty() {
        return Err(errors.join("; "));
    }
    for e in &errors {
        crate::log::warn(format!("models fetch partially failed: {e}"));
    }
    crate::log::info(format!(
        "fetched {} models from {} provider(s)",
        models.len(),
        providers().len()
    ));
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

    // Redirect HOME to a fresh temp dir (shares log::HOME_LOCK with the db
    // and log tests) so configured()/provider_for() read an isolated DB.
    fn with_temp_home(f: impl FnOnce()) {
        let _g = crate::log::HOME_LOCK.lock().unwrap();
        let tmp = std::env::temp_dir().join(format!("aime-providers-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::env::set_var("HOME", &tmp);
        f();
        std::env::set_var("HOME", std::env::temp_dir());
    }

    #[test]
    fn test_provider_for() {
        with_temp_home(|| {
            let (p, id) = provider_for("OpenCode - kimi-k3").unwrap();
            assert_eq!(p.name(), "OpenCode Go");
            assert_eq!(id, "kimi-k3");
            let (p, id) = provider_for("OpenRouter - anthropic/claude").unwrap();
            assert_eq!(p.name(), "OpenRouter");
            assert_eq!(id, "anthropic/claude");
            // with no provider configured the legacy bare id falls back to
            // OpenRouter, id unchanged
            let (p, id) = provider_for("anthropic/claude").unwrap();
            assert_eq!(p.name(), "OpenRouter");
            assert_eq!(id, "anthropic/claude");
        });
    }

    #[test]
    fn test_bare_id_routes_to_sole_configured_provider() {
        with_temp_home(|| {
            // LiteLLM-only setup: a bare id must route to LiteLLM, not
            // demand an OpenRouter key
            config::save_api_key("litellm", "sk-test").unwrap();
            for p in providers() {
                assert_eq!(p.configured(), p.name() == "LiteLLM");
            }
            let (p, id) = provider_for("anthropic/claude").unwrap();
            assert_eq!(p.name(), "LiteLLM");
            assert_eq!(id, "anthropic/claude");
            let (p, id) = provider_for("gpt-4o").unwrap();
            assert_eq!(p.name(), "LiteLLM");
            assert_eq!(id, "gpt-4o");
            // an explicit prefix still wins over the sole-provider fallback
            let (p, id) = provider_for("OpenRouter - anthropic/claude").unwrap();
            assert_eq!(p.name(), "OpenRouter");
            assert_eq!(id, "anthropic/claude");
        });
    }

    #[test]
    fn test_bare_id_routes_to_configured_provider() {
        with_temp_home(|| {
            // `aime provider use mistral <key>`: bare ids belong to Mistral
            // even though another provider is configured too
            config::save_api_key("mistral", "sk-test").unwrap();
            config::save_provider_name("Mistral").unwrap();
            assert_eq!(configured_provider().unwrap().name(), "Mistral");
            let (p, id) = provider_for("mistral-large-latest").unwrap();
            assert_eq!(p.name(), "Mistral");
            assert_eq!(id, "mistral-large-latest");
            // an explicit prefix still wins over the configured provider
            let (p, _) = provider_for("LiteLLM - gpt-4o").unwrap();
            assert_eq!(p.name(), "LiteLLM");
        });
    }

    #[test]
    fn test_configured_provider_legacy_fallback() {
        with_temp_home(|| {
            // a URL saved before provider_name existed still configures Custom
            assert!(configured_provider().is_none());
            config::save_provider("https://api.openai.com/v1", "sk-test").unwrap();
            let p = configured_provider().unwrap();
            assert_eq!(p.name(), "Custom");
            let (p, id) = provider_for("gpt-4o").unwrap();
            assert_eq!(p.name(), "Custom");
            assert_eq!(id, "gpt-4o");
        });
    }

    #[test]
    fn test_missing_key_error_names_setting() {
        let msg = missing_key_error(providers()[0].as_ref());
        assert_eq!(
            msg,
            "No OpenRouter API key configured — set it with: aime provider use openrouter <api_key>"
        );
    }

    #[test]
    fn test_provider_lookup() {
        let mistral = provider_by_name("mistral").unwrap();
        assert_eq!(mistral.name(), "Mistral");
        assert_eq!(mistral.prefix(), "Mistral - ");
        assert_eq!(mistral.key_setting(), "mistral");
        assert!(provider_by_name("nope").is_none());
        let names = provider_names();
        assert!(names.contains(&"Mistral"));
        assert!(names.contains(&"LiteLLM"));
        // settings-key lookup: the names `aime provider use` accepts
        assert_eq!(provider_by_key("openrouter").unwrap().name(), "OpenRouter");
        assert_eq!(provider_by_key("OpenCode").unwrap().name(), "OpenCode Go");
        assert_eq!(provider_by_key("custom").unwrap().name(), "Custom");
        assert!(provider_by_key("nope").is_none());
    }
}
