//! Task harness on OpenRouter / OpenCode Go, driven from beats (sessions).
//!
//! The user no longer picks a model per message. A cheap **classifier** model
//! routes each prompt to one of three tiers the user configures in Settings:
//! - `high` — most capable model; agentic work with tools, then a **reflexion**
//!   pass (critique the draft → refined answer) for hard, high-stakes tasks.
//! - `base` — implementation workhorse; agentic work with tools.
//! - `low`  — low-cost model, single completion, no tools.
//!
//! Tools (`read_file`, `write_file`, `edit_file`, `grep`, `bash`) plus one
//! `skill_*` tool per discovered skill in `~/.agents/skills/` are offered to
//! the high/base tiers; `skills.rs` holds skill discovery, `tools.rs` holds
//! the tool schemas and execution.
//!
//! New patterns: add a `pub async fn run_*` command that classifies, picks a
//! model tier, drives `agentic_loop`, and persists the result onto the beat.

use crate::providers::{self, chat_completion, chat_completion_stream, Usage};
use crate::{beats, config, db, projects, prompts, skills, tools};
use serde::Serialize;
use serde_json::json;
use std::sync::Mutex;
use tokio::task_local;

// Per-beat cancellation: stopping one beat must not touch other sessions
// running at the same time. Each run scopes its beat id into a task-local so
// providers keep calling `cancelled()` unchanged, without threading a token
// through every call. // ponytail: shared vec + task-local id — revisit only
// if cancel checks show up in profiles
task_local! {
    static BEAT: i64;
}

static CANCELLED: Mutex<Vec<i64>> = Mutex::new(Vec::new());

/// Ask the given beat's in-flight task to stop (the UI's Escape / chip ✕).
pub fn cancel_current(beat_id: i64) {
    let mut flags = CANCELLED.lock().unwrap();
    if !flags.contains(&beat_id) {
        crate::log::info(format!("cancel requested for beat {beat_id}"));
        flags.push(beat_id);
    }
}

/// Drop a stale cancel flag so a fresh run on the same beat can start.
pub fn clear_cancel(beat_id: i64) {
    CANCELLED.lock().unwrap().retain(|&b| b != beat_id);
}

fn is_cancelled(beat_id: i64) -> bool {
    CANCELLED.lock().unwrap().contains(&beat_id)
}

/// True when the current task's beat was asked to stop.
pub fn cancelled() -> bool {
    BEAT.try_with(|&id| is_cancelled(id)).unwrap_or(false)
}

pub const STOPPED: &str = "stopped";

/* ---- live events pushed to the UI while a task runs ---- */

/// One step of a running task, emitted to the webview as a `task-event` so
/// the chat renders work as it happens instead of after the whole run.
#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TaskEvent {
    /// Model + tier chosen for this task (labels the streaming bubble).
    Start { model: String, tier: String },
    /// Incremental text of the reply currently being generated.
    Delta { text: String },
    /// A tool call finished executing.
    Tool {
        tool: String,
        arguments: String,
        result: String,
        error: bool,
    },
    /// A finished intermediate step (e.g. the high-tier reflexion draft).
    Step { text: String },
}

/// Live-event sink for a running task: the UI layer (the TUI, a future CLI)
/// supplies one callback that receives every tagged `TaggedEvent` as the task
/// progresses.
pub type OnEvent<'a> = &'a mut (dyn FnMut(TaggedEvent) + Send);
type RawEvent<'a> = &'a mut (dyn FnMut(TaskEvent) + Send);

/// A task event tagged with the beat that produced it, so the UI can route
/// live updates to the right session while several run at once.
#[derive(Clone, Serialize)]
pub struct TaggedEvent {
    pub beat_id: i64,
    #[serde(flatten)]
    pub ev: TaskEvent,
}

/* ---- routing: the classifier picks a model tier ---- */

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    High,
    Base,
    Low,
}

impl Tier {
    fn as_str(self) -> &'static str {
        match self {
            Tier::High => "high",
            Tier::Base => "base",
            Tier::Low => "low",
        }
    }
    fn parse(s: &str) -> Option<Tier> {
        match s.trim().to_ascii_lowercase().as_str() {
            "high" => Some(Tier::High),
            "base" => Some(Tier::Base),
            "low" => Some(Tier::Low),
            _ => None,
        }
    }
}

/// Ask the classifier model which tier this prompt belongs to. Falls back to
/// `base` (a safe middle ground) if the reply can't be parsed.
async fn classify(classifier: &str, prompt: &str, mu: &mut ModelUsage) -> Result<Tier, String> {
    let r = chat_completion(
        classifier,
        &[
            json!({"role": "system", "content": prompts::CLASSIFIER}),
            json!({"role": "user", "content": prompt}),
        ],
        &[],
        Some(0.0),
        Some(64),
        true,
        None,
    )
    .await?;
    mu.add(&r.usage);
    let tier = extract_json(&r.content)
        .and_then(|v| v.get("tier").and_then(|t| t.as_str()).and_then(Tier::parse))
        .unwrap_or(Tier::Base);
    crate::log::debug(format!(
        "classifier replied {:?} → tier {}",
        r.content,
        tier.as_str()
    ));
    Ok(tier)
}

/* ---- context + session prompt ---- */

/// The beat's prior conversation, condensed into a context brief. Uses the
/// classifier model (cheap) as the summarizer.
async fn summarize_history(
    beat_id: i64,
    model: &str,
    mu: &mut ModelUsage,
) -> Result<String, String> {
    let prior = beats::get_beat_messages(beat_id)?;
    let text = prior
        .iter()
        .filter_map(|m| {
            let role = m["role"].as_str()?;
            let content = m["content"].as_str()?;
            Some(format!("{role}: {content}"))
        })
        .collect::<Vec<_>>()
        .join("\n");
    if text.is_empty() {
        return Ok(String::new());
    }
    // ponytail: cap to the last ~6000 chars — enough context, bounded tokens
    let chars: Vec<char> = text.chars().collect();
    let tail: String = chars[chars.len().saturating_sub(6000)..].iter().collect();
    let r = chat_completion(
        model,
        &[json!({
            "role": "user",
            "content": prompts::fill(prompts::SUMMARIZE, &[("history", tail.as_str())])
        })],
        &[],
        Some(0.2),
        Some(512),
        false,
        None,
    )
    .await?;
    mu.add(&r.usage);
    Ok(format!("Conversation context so far:\n{}", r.content))
}

/// Built-in session instructions, embedded from `prompts/system.md` at
/// compile time so the packaged app doesn't depend on a cwd file.
fn session_prompt() -> String {
    prompts::SYSTEM.trim().to_string()
}

/// Replay the beat's prior conversation into the live message list so follow-up
/// instructions keep the session's context. Tool entries are skipped — they
/// lack the `tool_call_id` pairing the API requires, and the brief (plus the
/// assistant narration they accompanied) covers that ground.
fn prior_turns(beat_id: i64) -> Result<Vec<serde_json::Value>, String> {
    Ok(beats::get_beat_messages(beat_id)?
        .into_iter()
        .filter(|m| {
            matches!(m["role"].as_str(), Some("user" | "assistant"))
                && !m["content"].as_str().unwrap_or("").trim().is_empty()
        })
        .map(|m| replay_message(&m))
        .collect())
}

fn system_message(brief: &str, session: &str, note: &str) -> serde_json::Value {
    let mut content = String::new();
    if !brief.is_empty() {
        content.push_str(brief);
        content.push_str("\n\n");
    }
    if !session.is_empty() {
        content.push_str("Session instructions:\n");
        content.push_str(session);
        content.push_str("\n\n");
    }
    if !note.is_empty() {
        content.push_str(note);
    }
    json!({"role": "system", "content": content})
}

/* ---- agentic loop: model + tools until the model signals completion ---- */

/// Session context limit: when the live context (last round's prompt tokens)
/// fills this share of the model's context window, the loop stops instead of
/// running forever. There is no turn cap — the context IS the limit. Once a
/// session hits it, only `/compact` (a fresh session holding just a summary)
/// lets work continue.
const CONTEXT_LIMIT_PERCENT: f64 = 90.0;

/// Drive the model against `messages` (already seeded with system + user
/// turns). The loop only ends when the model explicitly calls `task_complete`
/// with its final answer — a plain-text reply with no tool call is treated as
/// mid-task narration (or a parse hiccup) and the loop continues after a
/// reminder, so unfinished work can't silently end the session. The session
/// context limit replaces any turn cap: when the prompt tokens of a round
/// reach `CONTEXT_LIMIT_PERCENT` of the model's context window, one final
/// no-tools completion is forced and the session is flagged so only `/compact`
/// can continue it. Returns (final text, executed tool steps, total usage,
/// whether the session context limit was reached).
#[allow(clippy::too_many_arguments)]
async fn agentic_loop(
    model: &str,
    on_event: RawEvent<'_>,
    messages: &mut Vec<serde_json::Value>,
    entries: &mut Vec<serde_json::Value>,
    tools: &[serde_json::Value],
    cwd: Option<&str>,
) -> Result<(String, Vec<tools::ToolStep>, Usage, bool), String> {
    let mut steps: Vec<tools::ToolStep> = vec![];
    let mut usage = Usage::default();
    let mut nudged = false;
    let mut last_prompt_tokens: Option<u64>;
    let mut round = 0usize;
    loop {
        if cancelled() {
            crate::log::info("agentic loop cancelled");
            return Err(STOPPED.into());
        }
        // Whether this round's text was already streamed to the UI live via
        // deltas. When it was, emitting a `Step` for the same text would show
        // the reply twice.
        let mut streamed = false;
        let r = {
            let mut on_delta = |t: &str| {
                streamed = true;
                on_event(TaskEvent::Delta { text: t.into() })
            };
            chat_completion_stream(
                model,
                messages,
                &[],
                Some(0.7),
                None,
                false,
                Some(tools),
                &mut on_delta,
            )
            .await?
        };
        usage.prompt_tokens += r.usage.prompt_tokens;
        usage.completion_tokens += r.usage.completion_tokens;
        // the last round's prompt tokens are the live session context size
        last_prompt_tokens = Some(r.usage.prompt_tokens);
        crate::log::debug(format!(
            "agentic round {round}: {} tool call(s), {} new chars, finish_reason={:?}",
            r.tool_calls.len(),
            r.content.len(),
            r.finish_reason
        ));
        round += 1;

        // Path 1: the model called `task_complete` — the only sanctioned way
        // to finish. Its `summary` argument is the final answer.
        if let Some(done) = r.tool_calls.iter().find(|tc| tc.name == "task_complete") {
            let summary = serde_json::from_str::<serde_json::Value>(&done.arguments)
                .ok()
                .and_then(|v| v["summary"].as_str().map(str::to_string))
                .filter(|s| !s.trim().is_empty())
                // a malformed/empty summary: fall back to any narration on
                // this turn, else ask the model to restate it
                .unwrap_or_default();
            if summary.is_empty() {
                crate::log::warn(
                    "task_complete called with an empty summary; asking the model to restate",
                );
                persist_round(model, &r, messages, entries, on_event, streamed);
                for tc in &r.tool_calls {
                    messages.push(
                        json!({"role": "tool", "tool_call_id": tc.id,
                               "content": "task_complete requires a non-empty 'summary' argument. Call it again with your full final answer."}),
                    );
                }
                continue;
            }
            // The model often restates, as the summary, the narration it
            // already produced on a previous round (or answers in plain text,
            // gets the reminder, then calls `task_complete` with the same
            // words). That identical reply is already the last transcript
            // entry — don't emit or persist a second copy of it.
            let dup = entries
                .last()
                .map(|e| e["role"] == "assistant" && e["content"] == summary)
                .unwrap_or(false);
            if !dup {
                on_event(TaskEvent::Step {
                    text: summary.clone(),
                });
                entries.push(json!({
                    "role": "assistant", "model": model, "content": summary,
                }));
            }
            return Ok((summary, steps, usage, false));
        }

        if r.tool_calls.is_empty() {
            // Plain text with no tool call: normally mid-task narration, not
            // a completion signal — keep the loop going. Only when the turn
            // was cut off (`length`), the provider claimed `tool_calls` we
            // couldn't parse (fragment loss), or the reply came back EMPTY
            // (reasoning models can burn a turn) do we treat it as a broken
            // turn: emit + persist it and continue, with a one-time nudge so
            // a blank turn can't repeat itself.
            let blank = r.content.trim().is_empty();
            if !blank && !matches!(r.finish_reason.as_deref(), Some("length" | "tool_calls")) {
                // a real narration turn: persist it, then remind the model
                // how to finish. Don't re-persist if the model repeats the
                // same text verbatim (some providers resend the turn).
                let dup = entries
                    .last()
                    .map(|e| e["role"] == "assistant" && e["content"] == r.content)
                    .unwrap_or(false);
                if !dup {
                    if !streamed {
                        on_event(TaskEvent::Step {
                            text: r.content.clone(),
                        });
                    }
                    entries.push(json!({
                        "role": "assistant", "model": model, "content": r.content,
                    }));
                    messages.push(json!({"role": "assistant", "content": r.content}));
                }
                messages.push(json!({
                    "role": "user",
                    "content": "Your reply arrived without a tool call, so the task is still open. Continue working with tools, or call `task_complete` with your final answer if everything is done.",
                }));
                continue;
            }
            if !blank {
                if !streamed {
                    on_event(TaskEvent::Step {
                        text: r.content.clone(),
                    });
                }
                entries.push(json!({
                    "role": "assistant", "model": model, "content": r.content,
                }));
                messages.push(json!({"role": "assistant", "content": r.content}));
            } else if !nudged {
                crate::log::warn("agentic round came back empty; nudging the model to continue");
                nudged = true;
                messages.push(json!({"role": "user", "content": "Continue."}));
            }
            continue;
        }

        // the model's narration for this round is a message in its own right:
        // it was already streamed live (or, on a non-streaming fallback,
        // reaches the UI as a Step) and is persisted as a transcript entry
        if !r.content.trim().is_empty() {
            if !streamed {
                on_event(TaskEvent::Step {
                    text: r.content.clone(),
                });
            }
            entries.push(json!({
                "role": "assistant", "model": model, "content": r.content,
            }));
        }
        // keep the assistant's tool-call turn in the transcript so the
        // follow-up `tool` messages stay valid
        let calls = r.tool_calls;
        let tool_calls_json: Vec<serde_json::Value> = calls
            .iter()
            .map(|tc| {
                json!({
                    "id": tc.id, "type": "function",
                    "function": {"name": tc.name, "arguments": tc.arguments}
                })
            })
            .collect();
        messages.push(json!({
            "role": "assistant",
            "content": r.content,
            "tool_calls": tool_calls_json,
        }));
        for tc in &calls {
            if cancelled() {
                crate::log::info("agentic loop cancelled between tool calls");
                return Err(STOPPED.into());
            }
            let (output, error) = match tools::execute(&tc.name, &tc.arguments, cwd).await {
                Ok(out) => (out, false),
                Err(e) => (e, true),
            };
            crate::log::info(format!(
                "tool {} {} → {} ({} chars)",
                tc.name,
                truncate_middle(&tc.arguments, 300),
                if error { "error" } else { "ok" },
                output.len()
            ));
            // The diff section is for the UI only — the model already knows
            // what it wrote, so keep it out of the transcript.
            let model_output = tools::strip_diff(&output);
            on_event(TaskEvent::Tool {
                tool: tc.name.clone(),
                arguments: tc.arguments.clone(),
                result: output.clone(),
                error,
            });
            entries.push(json!({
                "role": "tool", "model": tc.name,
                "arguments": tc.arguments, "content": model_output, "error": error,
                // full result incl. the diff section, for the UI's git-style view
                "raw_content": output,
            }));
            steps.push(tools::ToolStep {
                tool: tc.name.clone(),
                arguments: tc.arguments.clone(),
                result: output.clone(),
                error,
            });
            messages.push(json!({"role": "tool", "tool_call_id": tc.id, "content": model_output}));
        }

        // Session context limit — replaces any turn cap. When the live
        // context fills its share of the model's window, force one final
        // no-tools completion (a plain answer instead of an error, since the
        // model may legitimately be done) and flag the session as full: only
        // `/compact` can continue it afterwards.
        if context_limit_reached(model, last_prompt_tokens).await {
            crate::log::warn(format!(
                "agentic loop hit the session context limit at {} prompt tokens; forcing a final answer",
                last_prompt_tokens.unwrap_or(0)
            ));
            let r = {
                let mut on_delta = |t: &str| on_event(TaskEvent::Delta { text: t.into() });
                chat_completion_stream(
                    model,
                    messages,
                    &[],
                    Some(0.7),
                    None,
                    false,
                    None,
                    &mut on_delta,
                )
                .await?
            };
            usage.prompt_tokens += r.usage.prompt_tokens;
            usage.completion_tokens += r.usage.completion_tokens;
            return Ok((r.content, steps, usage, true));
        }
    }
}

/// True when the live context (last round's prompt tokens) has filled
/// `CONTEXT_LIMIT_PERCENT` of the model's context window. Unknown context
/// lengths never trip the limit.
async fn context_limit_reached(model_id: &str, prompt_tokens: Option<u64>) -> bool {
    let Some(tokens) = prompt_tokens else {
        return false;
    };
    let models = providers::list_models().await.unwrap_or_default();
    let len = match models
        .iter()
        .find(|m| m.id == model_id)
        .and_then(|m| m.context_length)
    {
        Some(l) => l,
        None => return false,
    };
    if len == 0 {
        return false;
    }
    tokens as f64 / len as f64 * 100.0 >= CONTEXT_LIMIT_PERCENT
}

/// Persist one model round's narration + tool-call turn: emit the text live
/// unless it already reached the UI as streaming deltas, append it to
/// `entries`, and push the assistant turn (with tool calls) onto `messages`
/// so follow-up `tool` messages stay valid.
fn persist_round(
    model: &str,
    r: &crate::providers::ChatResult,
    messages: &mut Vec<serde_json::Value>,
    entries: &mut Vec<serde_json::Value>,
    on_event: RawEvent<'_>,
    streamed: bool,
) {
    if !r.content.trim().is_empty() {
        if !streamed {
            on_event(TaskEvent::Step {
                text: r.content.clone(),
            });
        }
        entries.push(json!({
            "role": "assistant", "model": model, "content": r.content,
        }));
    }
    let tool_calls_json: Vec<serde_json::Value> = r
        .tool_calls
        .iter()
        .map(|tc| {
            json!({
                "id": tc.id, "type": "function",
                "function": {"name": tc.name, "arguments": tc.arguments}
            })
        })
        .collect();
    messages.push(json!({
        "role": "assistant",
        "content": r.content,
        "tool_calls": tool_calls_json,
    }));
}

/* ---- reflexion (high tier): critique the agentic draft → refined answer ---- */

/// One reflexion pass over the draft the agentic loop produced, with the tool
/// work summarized as evidence for the critique.
async fn reflexion(
    model: &str,
    on_event: RawEvent<'_>,
    prompt: &str,
    draft: &str,
    tool_steps: &[tools::ToolStep],
    brief: &str,
    session: &str,
) -> Result<(String, Usage), String> {
    let evidence = tool_steps
        .iter()
        .map(|s| {
            let status = if s.error { "FAILED" } else { "ok" };
            format!("- {}({}) → {status}", s.tool, s.arguments)
        })
        .collect::<Vec<_>>()
        .join("\n");
    let evidence = if evidence.is_empty() {
        "none (no tools used)".to_string()
    } else {
        evidence
    };
    let mut on_delta = |t: &str| on_event(TaskEvent::Delta { text: t.into() });
    let r = chat_completion_stream(
        model,
        &[
            system_message(brief, session, ""),
            json!({"role": "user", "content": prompts::fill(
                prompts::REFLEXION,
                &[
                    ("prompt", prompt),
                    ("draft", draft),
                    ("evidence", evidence.as_str()),
                ],
            )}),
        ],
        &[],
        Some(0.5),
        None,
        false,
        None,
        &mut on_delta,
    )
    .await?;
    Ok((r.content, r.usage))
}

/// First `max` chars of `s` plus an ellipsis when it was longer — keeps
/// logged arguments (prompts, tool args) bounded.
fn truncate_middle(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    format!("{cut}…")
}

/// Pull the first embedded JSON object out of a reply (prose tolerated).
fn extract_json(text: &str) -> Option<serde_json::Value> {
    let start = text.find('{')?;
    let end = text.rfind('}')? + 1;
    serde_json::from_str(&text[start..end]).ok()
}

/* ---- task runner: classify → route → run → persist ---- */

#[derive(Serialize)]
pub struct TaskResult {
    pub tier: String,
    pub model: String,
    /// Intermediate reflexion steps (e.g. the draft). Empty for base/low.
    pub steps: Vec<String>,
    /// Tool calls the model made while working.
    pub tool_steps: Vec<tools::ToolStep>,
    pub answer: String,
    /// Per-model token usage for this run (classifier, summarizer, main
    /// model, reflexion — one entry per model that was called).
    pub usage: Vec<ModelUsage>,
    /// Total cost of this run in USD, summed across all model calls.
    pub cost_usd: f64,
    /// How full the main model's context window was on the last round
    /// (0.0–100.0), or `None` when the context length is unknown.
    pub context_percent: Option<f64>,
    /// True when the session hit its context limit this run. The session is
    /// paused until `/compact` opens a fresh one holding only a summary.
    pub context_full: bool,
    /// For `/compact`: the id of the fresh session holding the summary.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_beat_id: Option<i64>,
}

/// Token usage recorded against one model during a task run.
#[derive(Clone, Serialize)]
pub struct ModelUsage {
    pub model: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// Cost of these calls in USD (0.0 when pricing is unknown).
    pub cost_usd: f64,
}

impl ModelUsage {
    fn new(model: &str) -> Self {
        Self {
            model: model.to_string(),
            prompt_tokens: 0,
            completion_tokens: 0,
            cost_usd: 0.0,
        }
    }
    fn add(&mut self, u: &Usage) {
        self.prompt_tokens += u.prompt_tokens;
        self.completion_tokens += u.completion_tokens;
    }
}

impl std::ops::AddAssign for ModelUsage {
    fn add_assign(&mut self, other: Self) {
        self.model = other.model;
        self.prompt_tokens += other.prompt_tokens;
        self.completion_tokens += other.completion_tokens;
        self.cost_usd += other.cost_usd;
    }
}

/// Price one model call from cached OpenRouter pricing; $0 when unknown.
async fn usage_cost(model_id: &str, prompt: u64, completion: u64) -> f64 {
    let models = providers::list_models().await.unwrap_or_default();
    models.iter().find(|m| m.id == model_id).map_or(0.0, |m| {
        let p: f64 = m.pricing.prompt.parse().unwrap_or(0.0);
        let c: f64 = m.pricing.completion.parse().unwrap_or(0.0);
        p * prompt as f64 + c * completion as f64
    })
}

/// Context-window fill percentage for `model` given the last round's prompt
/// token count. `None` when the model's context length is unknown.
async fn context_percent(model_id: &str, prompt_tokens: u64) -> Option<f64> {
    let models = providers::list_models().await.unwrap_or_default();
    let len = models.iter().find(|m| m.id == model_id)?.context_length?;
    if len == 0 {
        return None;
    }
    Some((prompt_tokens as f64 / len as f64 * 100.0).min(100.0))
}

/// Record one model call's token usage + cost against a beat. Returns the
/// priced `ModelUsage` so callers can aggregate and report it.
async fn record_usage(beat_id: i64, mu: &mut ModelUsage, u: &Usage) {
    mu.add(u);
    mu.cost_usd += usage_cost(&mu.model, u.prompt_tokens, u.completion_tokens).await;
    if let Err(e) = beats::record_usage(
        beat_id,
        &mu.model,
        u.prompt_tokens as i64,
        u.completion_tokens as i64,
    )
    .await
    {
        crate::log::warn(format!("usage record failed: {e}"));
    }
}

/// Persisted "session context limit reached" flag for a beat.
fn session_is_full(beat_id: i64) -> Result<bool, String> {
    beats::is_context_full(beat_id)
}

/// `/compact`: summarize the current session, archive it, and return a result
/// pointing at a brand-new session holding only that summary. The new beat
/// inherits the project (and thus the working directory) but starts with a
/// clean context.
async fn compact_session(beat_id: i64, on_event: RawEvent<'_>) -> Result<TaskResult, String> {
    let cfg = config::ModelConfig::load()?;
    let classifier = cfg.classifier.trim();
    if classifier.is_empty() {
        return Err("No classifier model configured — set the four model slots with `pulse settings set classifier|high|base|low <model>` (ids are prefixed, e.g. \"LiteLLM - gpt-4o\").".into());
    }
    let mut mu = ModelUsage::new(classifier);
    let summary = summarize_history(beat_id, classifier, &mut mu).await?;
    if summary.trim().is_empty() {
        return Err("Nothing to compact — this session has no messages yet.".into());
    }
    let new_beat = beats::create_summary_beat(beat_id, &summary)?;
    crate::log::info(format!(
        "beat {beat_id} compacted into beat {}",
        new_beat.id
    ));
    // the old session is done: archive it and clear its full flag so it can
    // still be browsed (and compacted again if ever unarchived)
    let _ = beats::set_beat_archived(beat_id, true);
    beats::set_context_full(beat_id, false)?;
    on_event(TaskEvent::Step {
        text: format!(
            "Session compacted into a new session: “{}”.\n\n{}",
            new_beat.name, summary
        ),
    });
    let cost_usd = mu.cost_usd;
    Ok(TaskResult {
        tier: "low".into(),
        model: classifier.to_string(),
        steps: vec![],
        tool_steps: vec![],
        answer: format!(
            "Compacted. New session “{}” holds the summary:\n\n{}",
            new_beat.name, summary
        ),
        usage: vec![mu],
        cost_usd,
        context_percent: None,
        context_full: false,
        new_beat_id: Some(new_beat.id),
    })
}

/// Handle one user message: the classifier picks a tier, the tier's model runs
/// the task — agentic with tools for `high`/`base` (plus a reflexion pass for
/// `high`), a single completion for `low` — and everything is persisted onto
/// the beat so it survives reloads.
/// Tag events with the beat, clear any stale cancel flag for it, and run with
/// the beat id scoped so `cancelled()` knows which session asked.
pub async fn run_task(
    beat_id: i64,
    prompt: String,
    images: Vec<String>,
    on_event: OnEvent<'_>,
) -> Result<TaskResult, String> {
    crate::log::info(format!(
        "beat {beat_id}: starting task ({} chars, {} image(s))",
        prompt.len(),
        images.len()
    ));
    let started = std::time::Instant::now();
    clear_cancel(beat_id);
    let mut sink = |ev: TaskEvent| {
        on_event(TaggedEvent { beat_id, ev });
    };
    let res = BEAT
        .scope(beat_id, run_task_inner(beat_id, prompt, images, &mut sink))
        .await;
    match &res {
        Ok(r) => crate::log::info(format!(
            "beat {beat_id}: task finished in {:.1}s — tier={} model={} cost=${:.4} tools={} context={:?}",
            started.elapsed().as_secs_f64(),
            r.tier,
            r.model,
            r.cost_usd,
            r.tool_steps.len(),
            r.context_percent
        )),
        Err(e) => crate::log::error(format!(
            "beat {beat_id}: task failed after {:.1}s: {e}",
            started.elapsed().as_secs_f64()
        )),
    }
    res
}

/// Run a single prompt on a beat with a specific model, bypassing the
/// classifier. Always uses the agentic loop (tools enabled), like the base
/// tier path in `run_task_inner`. Used by the workflow engine for steps that
/// specify a model.
pub async fn run_task_with_model(
    beat_id: i64,
    prompt: String,
    model: String,
    images: Vec<String>,
    on_event: OnEvent<'_>,
) -> Result<TaskResult, String> {
    crate::log::info(format!(
        "beat {beat_id}: starting task with model {model} ({} chars, {} image(s))",
        prompt.len(),
        images.len()
    ));
    let started = std::time::Instant::now();
    clear_cancel(beat_id);
    let mut sink = |ev: TaskEvent| {
        on_event(TaggedEvent { beat_id, ev });
    };
    let res = BEAT
        .scope(
            beat_id,
            run_task_with_model_inner(beat_id, prompt, model, images, &mut sink),
        )
        .await;
    match &res {
        Ok(r) => crate::log::info(format!(
            "beat {beat_id}: model task finished in {:.1}s — model={} cost=${:.4} tools={}",
            started.elapsed().as_secs_f64(),
            r.model,
            r.cost_usd,
            r.tool_steps.len()
        )),
        Err(e) => crate::log::error(format!(
            "beat {beat_id}: model task failed after {:.1}s: {e}",
            started.elapsed().as_secs_f64()
        )),
    }
    res
}

/// Inner implementation of `run_task_with_model`: runs the agentic loop with a
/// specified model, skipping classifier routing and reflexion. Reuses the same
/// setup (system message, prior turns, working dir, persistence) as the base
/// tier path in `run_task_inner`.
async fn run_task_with_model_inner(
    beat_id: i64,
    prompt: String,
    model: String,
    images: Vec<String>,
    on_event: RawEvent<'_>,
) -> Result<TaskResult, String> {
    if session_is_full(beat_id)? && !prompt.trim().eq_ignore_ascii_case("/compact") {
        return Err(
            "Session context limit reached. Run /compact to open a new session \
             holding only a summary of this one."
                .to_string(),
        );
    }
    if prompt.trim().eq_ignore_ascii_case("/compact") {
        return compact_session(beat_id, on_event).await;
    }

    let cfg = config::ModelConfig::load()?;
    let classifier = cfg.classifier.trim();
    let session = session_prompt();

    // Use the classifier for summarization when available (cheap); fall back
    // to the specified model when no classifier is configured.
    let summarizer = if classifier.is_empty() {
        &model
    } else {
        classifier
    };
    let mut summarizer_usage = ModelUsage::new(summarizer);
    let brief = summarize_history(beat_id, summarizer, &mut summarizer_usage).await?;

    let mut main_usage = ModelUsage::new(&model);
    on_event(TaskEvent::Start {
        model: model.to_string(),
        tier: "workflow".to_string(),
    });

    let discovered = skills::discover();
    let tool_defs = tools::definitions(&discovered);
    let mut entries = vec![user_entry(&prompt, &images)];

    // Agentic loop (base-tier path: tools, no reflexion)
    let wd = projects::working_dir(beat_id)?;
    let mut note = prompts::AGENT_NOTE.to_string();
    if let Some(dir) = &wd {
        if !note.is_empty() {
            note.push_str("\n\n");
        }
        note.push_str(&format!(
            "Working directory: {dir}. Relative tool paths resolve against it, \
             bash runs inside it.\n\n"
        ));
        if let Some(agents) = projects::agents_note(dir) {
            note.push_str(&agents);
        }
    }
    let sys = system_message(&brief, &session, &note);
    let mut msgs = vec![sys];
    msgs.extend(prior_turns(beat_id)?);
    msgs.push(user_message(&prompt, &images));
    let (answer, tool_steps, u1, ctx_full) = agentic_loop(
        &model,
        on_event,
        &mut msgs,
        &mut entries,
        &tool_defs,
        wd.as_deref(),
    )
    .await?;
    record_usage(beat_id, &mut main_usage, &u1).await;

    let usage = main_usage.clone();
    let cost_usd = usage.cost_usd;
    let context_percent = context_percent(&model, main_usage.prompt_tokens).await;
    let context_full = ctx_full
        || context_percent
            .map(|p| p >= CONTEXT_LIMIT_PERCENT)
            .unwrap_or(false);
    if context_full {
        beats::set_context_full(beat_id, true)?;
    }

    let answer = if answer.trim().is_empty() {
        entries
            .iter()
            .rev()
            .find(|e| {
                e["role"] == "assistant" && !e["content"].as_str().unwrap_or("").trim().is_empty()
            })
            .and_then(|e| e["content"].as_str())
            .unwrap_or("")
            .to_string()
    } else {
        answer
    };
    let dup = entries
        .last()
        .map(|e| e["role"] == "assistant" && e["content"] == answer)
        .unwrap_or(false);
    if !dup {
        entries.push(json!({"role": "assistant", "model": &model, "content": &answer}));
    }
    db::append_messages(beat_id, entries)?;

    Ok(TaskResult {
        tier: "workflow".to_string(),
        model: model.to_string(),
        steps: vec![],
        tool_steps,
        answer,
        usage: vec![usage],
        cost_usd,
        context_percent,
        context_full,
        new_beat_id: None,
    })
}

/// The user message sent to the model: plain text normally, but with images
/// attached it becomes an OpenAI-style multimodal content-parts array
/// (image_url parts first, then the text). Data URLs (data:image/...;base64)
/// are what the frontend pastes from the clipboard.
fn user_message(prompt: &str, images: &[String]) -> serde_json::Value {
    if images.is_empty() {
        return json!({ "role": "user", "content": prompt });
    }
    let mut parts = vec![];
    for url in images {
        parts.push(json!({
            "type": "image_url",
            "image_url": { "url": url },
        }));
    }
    parts.push(json!({ "type": "text", "text": prompt }));
    json!({ "role": "user", "content": parts })
}

/// The persisted transcript entry for the user turn — text plus the raw image
/// data URLs under `images`, so prior turns can be replayed multimodally.
fn user_entry(prompt: &str, images: &[String]) -> serde_json::Value {
    if images.is_empty() {
        return json!({ "role": "user", "content": prompt });
    }
    json!({ "role": "user", "content": prompt, "images": images })
}

/// Rebuild a message for the live API call from a persisted transcript entry.
/// Entries that carry images become multimodal content-parts again; plain
/// entries stay as plain strings.
fn replay_message(m: &serde_json::Value) -> serde_json::Value {
    let role = m["role"].clone();
    let text = m["content"].as_str().unwrap_or("").to_string();
    if let Some(imgs) = m["images"].as_array() {
        if !imgs.is_empty() {
            let urls: Vec<String> = imgs
                .iter()
                .filter_map(|i| i.as_str().map(str::to_string))
                .collect();
            let mut parts: Vec<serde_json::Value> = urls
                .iter()
                .map(|u| json!({"type": "image_url", "image_url": {"url": u}}))
                .collect();
            if !text.trim().is_empty() {
                parts.push(json!({ "type": "text", "text": text }));
            }
            return json!({ "role": role, "content": parts });
        }
    }
    json!({ "role": role, "content": text })
}

async fn run_task_inner(
    beat_id: i64,
    prompt: String,
    images: Vec<String>,
    on_event: RawEvent<'_>,
) -> Result<TaskResult, String> {
    // A full session only accepts `/compact` — everything else is refused
    // until the context is carried over into a fresh summarized session.
    if session_is_full(beat_id)? && !prompt.trim().eq_ignore_ascii_case("/compact") {
        return Err(
            "Session context limit reached. Run /compact to open a new session \
             holding only a summary of this one."
                .to_string(),
        );
    }
    if prompt.trim().eq_ignore_ascii_case("/compact") {
        return compact_session(beat_id, on_event).await;
    }
    if let Some(wf_name) = prompt.trim().strip_prefix("/workflow ") {
        crate::log::info(format!("beat {beat_id}: running workflow {wf_name}"));
        let wf = crate::workflows::load(wf_name.trim())?;
        let mut tagged = |te: TaggedEvent| {
            on_event(te.ev);
        };
        let result = crate::workflows::run(beat_id, &wf, &mut tagged).await?;
        let answer = result
            .steps
            .iter()
            .map(|s| format!("## {}\n\n{}", s.name, s.answer))
            .collect::<Vec<_>>()
            .join("\n\n---\n\n");
        return Ok(TaskResult {
            tier: "workflow".into(),
            model: wf.model.unwrap_or_else(|| "classifier".into()),
            steps: result.steps.iter().map(|s| s.name.clone()).collect(),
            tool_steps: vec![],
            answer,
            usage: vec![],
            cost_usd: result.total_cost_usd,
            context_percent: None,
            context_full: false,
            new_beat_id: None,
        });
    }
    let cfg = config::ModelConfig::load()?;
    let classifier = cfg.classifier.trim();
    if classifier.is_empty() {
        return Err("No classifier model configured — set the four model slots with `pulse settings set classifier|high|base|low <model>` (ids are prefixed, e.g. \"LiteLLM - gpt-4o\").".into());
    }
    let session = session_prompt();
    let mut classifier_usage = ModelUsage::new(classifier);
    let brief = summarize_history(beat_id, classifier, &mut classifier_usage).await?;
    let tier = classify(classifier, &prompt, &mut classifier_usage).await?;
    let model = match tier {
        Tier::High => cfg.high.trim(),
        Tier::Base => cfg.base.trim(),
        Tier::Low => cfg.low.trim(),
    };
    crate::log::info(format!(
        "beat {beat_id}: classifier routed to tier {} → model {model}",
        tier.as_str()
    ));
    if model.is_empty() {
        return Err(format!(
            "No {} model configured — set the four models in Settings.",
            tier.as_str()
        ));
    }
    let mut main_usage = ModelUsage::new(model);
    on_event(TaskEvent::Start {
        model: model.to_string(),
        tier: tier.as_str().to_string(),
    });

    // skills only contribute their frontmatter up front; the full SKILL.md is
    // loaded on demand when the model invokes a skill tool
    let discovered = skills::discover();
    let tool_defs = tools::definitions(&discovered);

    // the persisted transcript, built chronologically as work happens
    let mut entries = vec![user_entry(&prompt, &images)];

    let (answer, steps, tool_steps, _usage, ctx_full) = match tier {
        Tier::High | Tier::Base => {
            // a beat attached to a project runs its tools inside the project
            // directory and gets its AGENTS.md injected as instructions
            let wd = projects::working_dir(beat_id)?;
            let mut note = prompts::AGENT_NOTE.to_string();
            if let Some(dir) = &wd {
                if !note.is_empty() {
                    note.push_str("\n\n");
                }
                note.push_str(&format!(
                    "Working directory: {dir}. Relative tool paths resolve against it, \
                     bash runs inside it.\n\n"
                ));
                if let Some(agents) = projects::agents_note(dir) {
                    note.push_str(&agents);
                }
            }
            let sys = system_message(&brief, &session, &note);
            // system first (some providers require it), then the replayed
            // prior turns, then this instruction
            let mut msgs = vec![sys];
            msgs.extend(prior_turns(beat_id)?);
            msgs.push(user_message(&prompt, &images));
            let (draft, tool_steps, u1, ctx_full) = agentic_loop(
                model,
                on_event,
                &mut msgs,
                &mut entries,
                &tool_defs,
                wd.as_deref(),
            )
            .await?;
            // for base the draft IS the final answer, already emitted by
            // `agentic_loop` when `task_complete` fired — don't show it twice.
            // For high tier the draft is a distinct intermediate message: it
            // was already emitted and persisted by the loop's `task_complete`
            // handling, so don't duplicate it — reflexion refines it next.
            if tier == Tier::High {
                crate::log::debug("high tier: running reflexion pass over the agentic draft");
                let (final_, u2) = reflexion(
                    model,
                    on_event,
                    &prompt,
                    &draft,
                    &tool_steps,
                    &brief,
                    &session,
                )
                .await?;
                let usage = Usage {
                    prompt_tokens: u1.prompt_tokens + u2.prompt_tokens,
                    completion_tokens: u1.completion_tokens + u2.completion_tokens,
                };
                record_usage(beat_id, &mut main_usage, &usage).await;
                (final_, vec![draft], tool_steps, usage, ctx_full)
            } else {
                record_usage(beat_id, &mut main_usage, &u1).await;
                (draft, vec![], tool_steps, u1, ctx_full)
            }
        }
        Tier::Low => {
            let sys = system_message(&brief, &session, "");
            let mut msgs = vec![sys];
            msgs.extend(prior_turns(beat_id)?);
            msgs.push(user_message(&prompt, &images));
            let mut on_delta = |t: &str| on_event(TaskEvent::Delta { text: t.into() });
            let r = chat_completion_stream(
                model,
                &msgs,
                &[],
                Some(0.7),
                None,
                false,
                None,
                &mut on_delta,
            )
            .await?;
            record_usage(beat_id, &mut main_usage, &r.usage).await;
            (r.content, vec![], vec![], r.usage, false)
        }
    };

    // aggregate: only the routed (post-classification) model's calls; the
    // classifier/summarizer calls are internal plumbing, not session usage
    let usage = main_usage.clone();
    let cost_usd = usage.cost_usd;
    let context_percent = context_percent(model, main_usage.prompt_tokens).await;
    // the session is full when the loop said so, or the reported fill already
    // crossed the limit line
    let context_full = ctx_full
        || context_percent
            .map(|p| p >= CONTEXT_LIMIT_PERCENT)
            .unwrap_or(false);
    if context_full {
        beats::set_context_full(beat_id, true)?;
    }

    // a blank answer must not end the run silently — fall back to the last
    // real assistant message (e.g. the truncated turn the loop continued
    // from), and don't persist it twice when that's already the last entry
    let answer = if answer.trim().is_empty() {
        entries
            .iter()
            .rev()
            .find(|e| {
                e["role"] == "assistant" && !e["content"].as_str().unwrap_or("").trim().is_empty()
            })
            .and_then(|e| e["content"].as_str())
            .unwrap_or("")
            .to_string()
    } else {
        answer
    };
    let dup = entries
        .last()
        .map(|e| e["role"] == "assistant" && e["content"] == answer)
        .unwrap_or(false);

    // everything already landed in `entries` in order; just close with the answer
    if !dup {
        entries.push(json!({"role": "assistant", "model": model, "content": &answer}));
    }
    db::append_messages(beat_id, entries)?;

    Ok(TaskResult {
        tier: tier.as_str().to_string(),
        model: model.to_string(),
        steps,
        tool_steps,
        answer,
        usage: vec![usage],
        cost_usd,
        context_percent,
        context_full,
        new_beat_id: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cancel_flag() {
        clear_cancel(1);
        assert!(!is_cancelled(1));
        cancel_current(1);
        assert!(is_cancelled(1));
        // other beats are untouched — that's the whole point
        assert!(!is_cancelled(2));
        clear_cancel(1);
        assert!(!is_cancelled(1));
    }

    #[test]
    fn test_truncate_middle() {
        assert_eq!(truncate_middle("short", 10), "short");
        let long = "x".repeat(20);
        assert_eq!(truncate_middle(&long, 5), "xxxxx…");
        // multibyte chars count as one, never split a codepoint
        assert_eq!(truncate_middle(&"é".repeat(3), 2), "éé…");
    }

    #[test]
    fn test_extract_json() {
        assert_eq!(extract_json("{\"tier\":\"high\"}").unwrap()["tier"], "high");
        // JSON embedded in prose survives
        let v = extract_json("Decision:\n{\"tier\": \"low\", \"n\": 2}\nThanks!").unwrap();
        assert_eq!(v["n"], 2);
        assert_eq!(v["tier"], "low");
        assert!(extract_json("no json here").is_none());
        assert!(extract_json("broken {json").is_none());
        // picks the outer object of nested ones
        assert_eq!(
            extract_json("x {\"a\": {\"b\": 1}} y").unwrap()["a"]["b"],
            1
        );
    }

    #[test]
    fn test_tier_parse() {
        assert_eq!(Tier::parse("high"), Some(Tier::High));
        assert_eq!(Tier::parse("BASE"), Some(Tier::Base));
        assert_eq!(Tier::parse(" low "), Some(Tier::Low));
        assert_eq!(Tier::parse("unknown"), None);
        assert_eq!(Tier::parse(""), None);
        assert_eq!(Tier::High.as_str(), "high");
    }
}
