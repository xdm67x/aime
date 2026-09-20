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

use crate::providers::{chat_completion, chat_completion_stream, Usage};
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

/// Live-event sink for a running task: the UI layer (Tauri, a CLI) supplies
/// one callback that receives every tagged `TaggedEvent` as the task
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
async fn classify(classifier: &str, prompt: &str) -> Result<Tier, String> {
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
    Ok(extract_json(&r.content)
        .and_then(|v| v.get("tier").and_then(|t| t.as_str()).and_then(Tier::parse))
        .unwrap_or(Tier::Base))
}

/* ---- context + session prompt ---- */

/// The beat's prior conversation, condensed into a context brief. Uses the
/// classifier model (cheap) as the summarizer.
async fn summarize_history(beat_id: i64, model: &str) -> Result<String, String> {
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
    Ok(format!("Conversation context so far:\n{}", r.content))
}

/// Built-in session instructions, embedded from `prompts/system.md` at
/// compile time so the packaged app doesn't depend on a cwd file.
fn session_prompt() -> String {
    prompts::SYSTEM.trim().to_string()
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

const MAX_TOOL_ITERATIONS: u32 = 40;

/// Drive the model against `messages` (already seeded with system + user
/// turns). The loop only ends when the model explicitly calls `task_complete`
/// with its final answer — a plain-text reply with no tool call is treated as
/// mid-task narration (or a parse hiccup) and the loop continues after a
/// reminder, so unfinished work can't silently end the session. The hard cap
/// `MAX_TOOL_ITERATIONS` bounds runaway loops; hitting it forces one final
/// no-tools completion as a fallback. Every round's text and each tool result
/// is emitted live and appended to `entries` (the persisted transcript, in
/// order). Returns (final text, executed tool steps, total usage).
#[allow(clippy::too_many_arguments)]
async fn agentic_loop(
    model: &str,
    on_event: RawEvent<'_>,
    messages: &mut Vec<serde_json::Value>,
    entries: &mut Vec<serde_json::Value>,
    tools: &[serde_json::Value],
    cwd: Option<&str>,
) -> Result<(String, Vec<tools::ToolStep>, Usage), String> {
    let mut steps: Vec<tools::ToolStep> = vec![];
    let mut usage = Usage::default();
    let mut nudged = false;
    let mut rounds = 0u32;
    loop {
        rounds += 1;
        if cancelled() {
            return Err(STOPPED.into());
        }
        let r = {
            let mut on_delta = |t: &str| on_event(TaskEvent::Delta { text: t.into() });
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

        // Path 1: the model called `task_complete` — the only sanctioned way
        // to finish. Its `summary` argument is the final answer.
        if let Some(done) = r
            .tool_calls
            .iter()
            .find(|tc| tc.name == "task_complete")
        {
            let summary = serde_json::from_str::<serde_json::Value>(&done.arguments)
                .ok()
                .and_then(|v| v["summary"].as_str().map(str::to_string))
                .filter(|s| !s.trim().is_empty())
                // a malformed/empty summary: fall back to any narration on
                // this turn, else ask the model to restate it
                .unwrap_or_default();
            if summary.is_empty() {
                persist_round(model, &r, messages, entries, on_event);
                for tc in &r.tool_calls {
                    messages.push(
                        json!({"role": "tool", "tool_call_id": tc.id,
                               "content": "task_complete requires a non-empty 'summary' argument. Call it again with your full final answer."}),
                    );
                }
                continue;
            }
            on_event(TaskEvent::Step {
                text: summary.clone(),
            });
            entries.push(json!({
                "role": "assistant", "model": model, "content": summary,
            }));
            return Ok((summary, steps, usage));
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
                    on_event(TaskEvent::Step {
                        text: r.content.clone(),
                    });
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
                on_event(TaskEvent::Step {
                    text: r.content.clone(),
                });
                entries.push(json!({
                    "role": "assistant", "model": model, "content": r.content,
                }));
                messages.push(json!({"role": "assistant", "content": r.content}));
            } else if !nudged {
                nudged = true;
                messages.push(json!({"role": "user", "content": "Continue."}));
            }
            continue;
        }

        // the model's narration for this round is a message in its own right:
        // emit it live (the frontend seals the streaming bubble) and persist it
        if !r.content.trim().is_empty() {
            on_event(TaskEvent::Step {
                text: r.content.clone(),
            });
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
                return Err(STOPPED.into());
            }
            let (output, error) = match tools::execute(&tc.name, &tc.arguments, cwd).await {
                Ok(out) => (out, false),
                Err(e) => (e, true),
            };
            on_event(TaskEvent::Tool {
                tool: tc.name.clone(),
                arguments: tc.arguments.clone(),
                result: output.clone(),
                error,
            });
            entries.push(json!({
                "role": "tool", "model": tc.name,
                "arguments": tc.arguments, "content": output, "error": error,
            }));
            steps.push(tools::ToolStep {
                tool: tc.name.clone(),
                arguments: tc.arguments.clone(),
                result: output.clone(),
                error,
            });
            messages.push(json!({"role": "tool", "tool_call_id": tc.id, "content": output}));
        }

        // Hard cap: don't loop forever. One final call without tools forces
        // a plain-text answer out of whatever state we're in.
        if rounds >= MAX_TOOL_ITERATIONS {
            eprintln!(
                "agentic loop hit the {MAX_TOOL_ITERATIONS}-round cap; forcing a final answer"
            );
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
            return Ok((r.content, steps, usage));
        }
    }
}

/// Persist one model round's narration + tool-call turn: emit the text live,
/// append it to `entries`, and push the assistant turn (with tool calls) onto
/// `messages` so follow-up `tool` messages stay valid.
fn persist_round(
    model: &str,
    r: &crate::providers::ChatResult,
    messages: &mut Vec<serde_json::Value>,
    entries: &mut Vec<serde_json::Value>,
    on_event: RawEvent<'_>,
) {
    if !r.content.trim().is_empty() {
        on_event(TaskEvent::Step {
            text: r.content.clone(),
        });
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
}

/// Record one model call's token usage + cost against a beat.
async fn record_usage(beat_id: i64, model: &str, u: &Usage) {
    if let Err(e) = beats::record_usage(
        beat_id,
        model,
        u.prompt_tokens as i64,
        u.completion_tokens as i64,
    )
    .await
    {
        eprintln!("usage record failed: {e}");
    }
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
    on_event: OnEvent<'_>,
) -> Result<TaskResult, String> {
    clear_cancel(beat_id);
    let mut sink = |ev: TaskEvent| {
        let _ = on_event(TaggedEvent { beat_id, ev });
    };
    BEAT.scope(beat_id, run_task_inner(beat_id, prompt, &mut sink)).await
}
async fn run_task_inner(
    beat_id: i64,
    prompt: String,
    on_event: RawEvent<'_>,
) -> Result<TaskResult, String> {
    let cfg = config::ModelConfig::load()?;
    let classifier = cfg.classifier.trim();
    if classifier.is_empty() {
        return Err("No classifier model configured — set the four models in Settings.".into());
    }
    let session = session_prompt();
    let brief = summarize_history(beat_id, classifier).await?;
    let tier = classify(classifier, &prompt).await?;
    let model = match tier {
        Tier::High => cfg.high.trim(),
        Tier::Base => cfg.base.trim(),
        Tier::Low => cfg.low.trim(),
    };
    if model.is_empty() {
        return Err(format!(
            "No {} model configured — set the four models in Settings.",
            tier.as_str()
        ));
    }
    on_event(TaskEvent::Start {
        model: model.to_string(),
        tier: tier.as_str().to_string(),
    });

    // skills only contribute their frontmatter up front; the full SKILL.md is
    // loaded on demand when the model invokes a skill tool
    let discovered = skills::discover();
    let tool_defs = tools::definitions(&discovered);

    // the persisted transcript, built chronologically as work happens
    let mut entries = vec![json!({"role": "user", "content": &prompt})];

    let (answer, steps, tool_steps, usage) = match tier {
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
            let mut msgs = vec![sys, json!({"role": "user", "content": &prompt})];
            let (draft, tool_steps, u1) = agentic_loop(
                model,
                on_event,
                &mut msgs,
                &mut entries,
                &tool_defs,
                wd.as_deref(),
            )
            .await?;
            // the agentic draft is one more assistant message (the loop's own
            // narrations and tool results are already in `entries`); for base
            // the draft IS the final answer, so don't store it twice — the
            // answer push below persists it
            on_event(TaskEvent::Step {
                text: draft.clone(),
            });
            if tier == Tier::High {
                entries.push(json!({
                    "role": "assistant", "model": model, "content": draft,
                }));
                let (final_, u2) =
                    reflexion(model, on_event, &prompt, &draft, &tool_steps, &brief, &session).await?;
                let usage = Usage {
                    prompt_tokens: u1.prompt_tokens + u2.prompt_tokens,
                    completion_tokens: u1.completion_tokens + u2.completion_tokens,
                };
                (final_, vec![draft], tool_steps, usage)
            } else {
                (draft, vec![], tool_steps, u1)
            }
        }
        Tier::Low => {
            let sys = system_message(&brief, &session, "");
            let mut on_delta = |t: &str| on_event(TaskEvent::Delta { text: t.into() });
            let r = chat_completion_stream(
                model,
                &[sys, json!({"role": "user", "content": &prompt})],
                &[],
                Some(0.7),
                None,
                false,
                None,
                &mut on_delta,
            )
            .await?;
            (r.content, vec![], vec![], r.usage)
        }
    };
    record_usage(beat_id, model, &usage).await;

    // a blank answer must not end the run silently — fall back to the last
    // real assistant message (e.g. the truncated turn the loop continued
    // from), and don't persist it twice when that's already the last entry
    let answer = if answer.trim().is_empty() {
        entries
            .iter()
            .rev()
            .find(|e| {
                e["role"] == "assistant"
                    && !e["content"].as_str().unwrap_or("").trim().is_empty()
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
