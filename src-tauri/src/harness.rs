//! Task harness on OpenRouter, driven from beats (sessions).
//!
//! The user no longer picks a model per message. A cheap **classifier** model
//! routes each prompt to one of three tiers the user configures in Settings:
//! - `high` — most capable model; the task runs a **reflexion** pass
//!   (draft → critique → refined answer) for hard, high-stakes work.
//! - `base` — implementation workhorse for typical coding/analysis.
//! - `low`  — low-cost model for simple, basic tasks.
//!
//! New patterns: add a `pub async fn run_*` command that classifies, picks a
//! model tier, runs the work, and persists the result onto the beat.

use crate::openrouter::{chat_completion, Usage};
use crate::{beats, config, db};
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

const CLASSIFIER_PROMPT: &str = "You route a user task to one of three model tiers by difficulty.\n\
- \"high\": deep reasoning, multi-step planning, architecture, hard debugging, or high-stakes \
problems that benefit from reflection.\n\
- \"base\": typical implementation work — writing or modifying code, explaining, straightforward \
but non-trivial tasks.\n\
- \"low\": simple, basic tasks — quick facts, greetings, formatting, trivial questions.\n\
Reply with only JSON: {\"tier\":\"high\"|\"base\"|\"low\"}.";

/// Ask the classifier model which tier this prompt belongs to. Falls back to
/// `base` (a safe middle ground) if the reply can't be parsed.
async fn classify(classifier: &str, prompt: &str) -> Result<Tier, String> {
    let (reply, _) = chat_completion(
        classifier,
        &[
            json!({"role": "system", "content": CLASSIFIER_PROMPT}),
            json!({"role": "user", "content": prompt}),
        ],
        &[],
        Some(0.0),
        Some(64),
        true,
    )
    .await?;
    Ok(extract_json(&reply)
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
    let (summary, _) = chat_completion(
        model,
        &[json!({"role": "user", "content": format!(
            "Summarize this conversation so far into a compact context brief. \
             Keep facts, decisions and open questions.\n\n{tail}")})],
        &[],
        Some(0.2),
        Some(512),
        false,
    )
    .await?;
    Ok(format!("Conversation context so far:\n{summary}"))
}

/// Optional session-level system prompt. Read from SYSTEM_PROMPT.md in the
/// working directory each time a run starts, so edits apply on the next run.
/// ponytail: cwd-relative — fine for `pnpm tauri dev`; if the packaged app
/// can't find it, switch to an explicit path from settings.
fn session_prompt() -> String {
    std::fs::read_to_string("SYSTEM_PROMPT.md")
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn system_message(brief: &str, session: &str) -> serde_json::Value {
    let mut content = String::new();
    if !brief.is_empty() {
        content.push_str(brief);
        content.push_str("\n\n");
    }
    if !session.is_empty() {
        content.push_str("Session instructions (SYSTEM_PROMPT.md):\n");
        content.push_str(session);
    }
    json!({"role": "system", "content": content})
}

/* ---- reflexion (high tier): draft → critique → refined answer ---- */

/// Two-pass reflexion: a draft answer, then a self-critique that produces the
/// refined final answer. Returns (final_answer, intermediate_steps, usage).
async fn reflexion(
    model: &str,
    prompt: &str,
    brief: &str,
    session: &str,
) -> Result<(String, Vec<String>, Usage), String> {
    let sys = system_message(brief, session);
    // 1) draft
    let (draft, u1) = chat_completion(
        model,
        &[
            sys.clone(),
            json!({"role": "user", "content": format!(
                "Answer the following task. This is a draft — be thorough.\n\nTask:\n{prompt}")}),
        ],
        &[],
        Some(0.7),
        None,
        false,
    )
    .await?;
    // 2) refine: critique the draft and produce the final answer
    let (final_, u2) = chat_completion(
        model,
        &[
            sys,
            json!({"role": "user", "content": format!(
                "Here is your draft answer:\n\n{draft}\n\n\
                 Critique it for correctness, gaps and clarity, then give the final, improved answer.\n\nFinal answer:")}),
        ],
        &[],
        Some(0.5),
        None,
        false,
    )
    .await?;
    Ok((
        final_,
        vec![draft],
        Usage {
            prompt_tokens: u1.prompt_tokens + u2.prompt_tokens,
            completion_tokens: u1.completion_tokens + u2.completion_tokens,
        },
    ))
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
/// the task (with a reflexion pass for `high`), and the result is persisted
/// onto the beat so it survives reloads.
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

    let (answer, steps, usage) = match tier {
        Tier::High => reflexion(model, &prompt, &brief, &session).await?,
        Tier::Base | Tier::Low => {
            let sys = system_message(&brief, &session);
            let (reply, u) = chat_completion(
                model,
                &[sys, json!({"role": "user", "content": &prompt})],
                &[],
                Some(0.7),
                None,
                false,
            )
            .await?;
            (reply, vec![], u)
        }
    };
    record_usage(beat_id, model, &usage).await;

    // persist onto the beat so it survives reloads
    let mut entries = vec![json!({"role": "user", "content": &prompt})];
    for s in &steps {
        entries.push(json!({"role": "assistant", "model": model, "content": s}));
    }
    entries.push(json!({"role": "assistant", "model": model, "content": &answer}));
    db::append_messages(beat_id, entries)?;

    Ok(TaskResult {
        tier: tier.as_str().to_string(),
        model: model.to_string(),
        steps,
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
        assert_eq!(extract_json("x {\"a\": {\"b\": 1}} y").unwrap()["a"]["b"], 1);
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
