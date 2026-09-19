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

use crate::providers::{chat_completion, Usage};
use crate::{beats, config, db, projects, prompts, skills, tools};
use serde::Serialize;
use serde_json::json;

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

/* ---- agentic loop: model + tools until a plain-text answer ---- */

const MAX_TOOL_ITERATIONS: u32 = 12;

/// Drive the model against `messages` (already seeded with system + user
/// turns): each round's tool calls are executed and fed back as `tool`
/// messages until the model replies with plain text — or after
/// `MAX_TOOL_ITERATIONS` rounds, where one final call without tools forces a
/// plain-text answer. Returns (final text, executed tool steps, total usage).
async fn agentic_loop(
    model: &str,
    messages: &mut Vec<serde_json::Value>,
    tools: &[serde_json::Value],
    cwd: Option<&str>,
) -> Result<(String, Vec<tools::ToolStep>, Usage), String> {
    let mut steps: Vec<tools::ToolStep> = vec![];
    let mut usage = Usage::default();
    for _ in 0..MAX_TOOL_ITERATIONS {
        let r = chat_completion(model, messages, &[], Some(0.7), None, false, Some(tools)).await?;
        usage.prompt_tokens += r.usage.prompt_tokens;
        usage.completion_tokens += r.usage.completion_tokens;
        if r.tool_calls.is_empty() {
            return Ok((r.content, steps, usage));
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
            let (output, error) = match tools::execute(&tc.name, &tc.arguments, cwd).await {
                Ok(out) => (out, false),
                Err(e) => (e, true),
            };
            steps.push(tools::ToolStep {
                tool: tc.name.clone(),
                arguments: tc.arguments.clone(),
                result: output.clone(),
                error,
            });
            messages.push(json!({"role": "tool", "tool_call_id": tc.id, "content": output}));
        }
    }
    let r = chat_completion(model, messages, &[], Some(0.7), None, false, None).await?;
    usage.prompt_tokens += r.usage.prompt_tokens;
    usage.completion_tokens += r.usage.completion_tokens;
    Ok((r.content, steps, usage))
}

/* ---- reflexion (high tier): critique the agentic draft → refined answer ---- */

/// One reflexion pass over the draft the agentic loop produced, with the tool
/// work summarized as evidence for the critique.
async fn reflexion(
    model: &str,
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
    let r = chat_completion(
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
#[tauri::command]
pub async fn run_task(beat_id: i64, prompt: String) -> Result<TaskResult, String> {
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

    // skills only contribute their frontmatter up front; the full SKILL.md is
    // loaded on demand when the model invokes a skill tool
    let discovered = skills::discover();
    let tool_defs = tools::definitions(&discovered);

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
            let (draft, tool_steps, u1) =
                agentic_loop(model, &mut msgs, &tool_defs, wd.as_deref()).await?;
            if tier == Tier::High {
                let (final_, u2) =
                    reflexion(model, &prompt, &draft, &tool_steps, &brief, &session).await?;
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
            let r = chat_completion(
                model,
                &[sys, json!({"role": "user", "content": &prompt})],
                &[],
                Some(0.7),
                None,
                false,
                None,
            )
            .await?;
            (r.content, vec![], vec![], r.usage)
        }
    };
    record_usage(beat_id, model, &usage).await;

    // persist onto the beat so it survives reloads
    let mut entries = vec![json!({"role": "user", "content": &prompt})];
    for s in &tool_steps {
        entries.push(json!({
            "role": "tool",
            "model": s.tool,
            "arguments": s.arguments,
            "content": s.result,
            "error": s.error,
        }));
    }
    for s in &steps {
        entries.push(json!({"role": "assistant", "model": model, "content": s}));
    }
    entries.push(json!({"role": "assistant", "model": model, "content": &answer}));
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
