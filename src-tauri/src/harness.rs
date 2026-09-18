//! Multi-agent harness on OpenRouter, driven from beats (sessions).
//!
//! - `GlobalContext` / `SubContext`: hierarchical context. Sub-contexts are
//!   isolated threads that branch from the global context; only a condensed
//!   artifact is merged back, so long dialogues never pollute global tokens.
//! - `AgentConfig`: one agent (id, role, model, persona, sampling params,
//!   OpenRouter fallback models).
//! - Council (`run_council`): typing `@modelA @modelB … task` in a beat
//!   convenes the mentioned models in a sub-context seeded with a summary of
//!   the beat's prior context; they deliberate turn by turn and their
//!   consensus is merged back into the beat.
//!
//! New patterns: add a `pub async fn run_*` command that builds a
//! `GlobalContext`, drives a `SubContext` through `dialogue_loop`, and merges.

use crate::openrouter::{chat_completion, Usage};
use crate::{beats, db};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;

/* ---- agent runtime ---- */

#[derive(Clone, Deserialize, Serialize)]
pub struct AgentConfig {
    pub agent_id: String,
    #[serde(default)]
    pub role_name: String,
    pub model_id: String,
    pub system_prompt: String,
    #[serde(default = "default_temperature")]
    pub temperature: f64,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    /// OpenRouter fallback model ids, tried in order if the primary fails.
    #[serde(default)]
    pub fallbacks: Vec<String>,
}

fn default_temperature() -> f64 {
    0.7
}
fn default_max_tokens() -> u32 {
    1024
}

impl AgentConfig {
    fn system_message(&self, preamble: &str) -> serde_json::Value {
        json!({
            "role": "system",
            "content": format!("{preamble}\n\nYour role: {}.\n\n{}", self.role_name, self.system_prompt),
        })
    }
}

/* ---- context engine (hierarchical memory) ---- */

#[derive(Default, Serialize)]
pub struct GlobalContext {
    pub goal: String,
    pub constraints: Vec<String>,
    /// Condensed artifacts merged in from sub-contexts (never raw dialogue).
    pub history: Vec<serde_json::Value>,
    pub state: BTreeMap<String, serde_json::Value>,
}

impl GlobalContext {
    fn preamble(&self) -> String {
        format!(
            "Global goal: {}\nConstraints: {}",
            self.goal,
            if self.constraints.is_empty() {
                "none".to_string()
            } else {
                self.constraints.join("; ")
            }
        )
    }
}

/// Isolated execution thread branching from a `GlobalContext`. Local dialogue
/// lives here only; `merge_to_global` condenses it before it touches global
/// context tokens.
struct SubContext<'a> {
    name: String,
    parent: &'a mut GlobalContext,
    /// Message roles are speaker ids ("user" for the human/orchestrator).
    messages: Vec<(String, String)>,
}

impl SubContext<'_> {
    fn push(&mut self, speaker: &str, content: impl Into<String>) {
        self.messages.push((speaker.to_string(), content.into()));
    }

    /// Thread dialogue for `agent_id`: the agent's own turns are
    /// "assistant", everyone else's are "user" — so agent N's output arrives
    /// as agent N+1's user input.
    fn render_for(&self, agent_id: &str) -> Vec<serde_json::Value> {
        self.messages
            .iter()
            .map(|(speaker, content)| {
                json!({
                    "role": if speaker == agent_id { "assistant" } else { "user" },
                    "content": if speaker == "user" || speaker == agent_id {
                        content.clone()
                    } else {
                        format!("{speaker}: {content}")
                    },
                })
            })
            .collect()
    }

    /// Condense the thread (LLM summary when no artifact is given) and append
    /// only the artifact to the global history, then free the local tokens.
    async fn merge_to_global(
        &mut self,
        artifact: Option<String>,
        summarizer_model: &str,
    ) -> Result<String, String> {
        let artifact = match artifact {
            Some(a) => a, // already condensed (e.g. a judge verdict)
            None => {
                let transcript = self
                    .messages
                    .iter()
                    .map(|(s, c)| format!("{s}: {c}"))
                    .collect::<Vec<_>>()
                    .join("\n\n");
                let (summary, _) = chat_completion(
                    summarizer_model,
                    &[json!({"role": "user", "content": format!(
                        "Condense this dialogue into a short consensus summary (the final artifact). \
                         Keep decisions, rationale and open questions; drop the back-and-forth.\n\n{transcript}"
                    )})],
                    &[],
                    Some(0.2),
                    Some(512),
                    false,
                )
                .await?;
                summary
            }
        };
        self.parent
            .history
            .push(json!({"thread": self.name, "artifact": artifact}));
        self.messages.clear();
        Ok(artifact)
    }

    /// One turn: agent sees the global preamble + persona, then the whole
    /// thread role-mapped for it.
    async fn call(&self, agent: &AgentConfig) -> Result<(String, Usage), String> {
        let system = agent.system_message(&self.parent.preamble());
        let mut msgs = vec![system];
        msgs.extend(self.render_for(&agent.agent_id));
        chat_completion(
            &agent.model_id,
            &msgs,
            &agent.fallbacks,
            Some(agent.temperature),
            Some(agent.max_tokens),
            false,
        )
        .await
    }
}

/* ---- dialogue turn engine (shared) ---- */

/// Agents take turns in a sub-context (agent N's output is agent N+1's user
/// input); ends on `[DONE]`, JSON status "COMPLETE", or max turns. A bare
/// status-only reply is treated as pure signal and kept out of the transcript.
/// When `usage_beat` is set, each turn's token usage is recorded on that beat.
struct LoopOutput {
    termination: String,
    transcript: Vec<serde_json::Value>,
    prompt_tokens: u64,
    completion_tokens: u64,
}

async fn dialogue_loop(
    thread: &mut SubContext<'_>,
    agents: &[AgentConfig],
    max_turns: u32,
    usage_beat: Option<i64>,
) -> LoopOutput {
    let mut transcript = match thread.messages.first() {
        Some((speaker, topic)) if speaker == "user" => {
            vec![json!({"agent_id": "user", "content": topic.clone()})]
        }
        _ => vec![],
    };
    let mut termination = "max_turns".to_string();
    let mut prompt_tokens = 0u64;
    let mut completion_tokens = 0u64;
    for turn in 0..max_turns {
        let agent = &agents[turn as usize % agents.len()];
        match thread.call(agent).await {
            Ok((reply, u)) => {
                prompt_tokens += u.prompt_tokens;
                completion_tokens += u.completion_tokens;
                if let Some(beat_id) = usage_beat {
                    if let Err(e) = beats::record_usage(
                        beat_id,
                        &agent.model_id,
                        u.prompt_tokens as i64,
                        u.completion_tokens as i64,
                    )
                    .await
                    {
                        eprintln!("council usage record failed: {e}");
                    }
                }
                // a bare "done" signal (short reply that is only the flag)
                // ends the run without polluting the transcript
                if reply.trim().len() < 80 && conversation_done(&reply) {
                    termination = "agent signaled done".into();
                    break;
                }
                thread.push(&agent.agent_id, &reply);
                transcript.push(json!({
                    "agent_id": agent.agent_id,
                    "role_name": agent.role_name,
                    "model": agent.model_id,
                    "content": reply,
                }));
                if conversation_done(&reply) {
                    termination = "agent signaled done".into();
                    break;
                }
            }
            // a failed turn ends the run but is visible in the transcript
            Err(e) => {
                thread.push(&agent.agent_id, format!("[ERROR] {e}"));
                transcript.push(json!({"agent_id": agent.agent_id, "content": format!("[ERROR] {e}")}));
                termination = format!("error: {e}");
                break;
            }
        }
    }
    LoopOutput {
        termination,
        transcript,
        prompt_tokens,
        completion_tokens,
    }
}

/* ---- termination + JSON helpers (pure, unit-tested below) ---- */

/// Does this reply end the conversation? Explicit `[DONE]` marker or a
/// structured JSON status of "COMPLETE" (case-insensitive).
fn conversation_done(reply: &str) -> bool {
    let status = extract_json(reply).and_then(|v| {
        v.get("status")
            .and_then(|s| s.as_str())
            .map(str::to_string)
    });
    reply.contains("[DONE]") || status.is_some_and(|s| s.eq_ignore_ascii_case("complete"))
}

/// Pull the first embedded JSON object out of a reply (prose tolerated).
fn extract_json(text: &str) -> Option<serde_json::Value> {
    let start = text.find('{')?;
    let end = text.rfind('}')? + 1;
    serde_json::from_str(&text[start..end]).ok()
}

/* ---- council (@model @model … in the chat) ---- */

const COUNCIL_PROMPT: &str = "You are one member of a council of AI models. Discuss the user's \
question together with the other members: build on good points, challenge weak ones, \
converge on the best answer. Be concise. When the council has fully converged, reply \
with only {\"status\":\"COMPLETE\"}.";

#[derive(Serialize)]
pub struct CouncilResult {
    pub termination: String,
    pub transcript: Vec<serde_json::Value>,
    /// The council's consensus, merged into the beat's history.
    pub artifact: String,
}

/// The beat's prior conversation, condensed into a context brief for the
/// council's global context.
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
    // ponytail: cap to the last ~6000 chars — enough context, bounded tokens;
    // raise if long beats get truncated too aggressively
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

/// Convene a council: the mentioned models deliberate in a sub-context seeded
/// with a summary of the beat's prior context, then their consensus is merged
/// back and persisted to the beat (transcript + final artifact).
#[tauri::command]
pub async fn run_council(
    beat_id: i64,
    prompt: String,
    models: Vec<String>,
    max_turns: Option<u32>,
) -> Result<CouncilResult, String> {
    if models.len() < 2 {
        return Err("Council needs at least two @models".into());
    }
    let agents: Vec<AgentConfig> = models
        .iter()
        .map(|m| AgentConfig {
            agent_id: m.clone(),
            role_name: m.clone(),
            model_id: m.clone(),
            system_prompt: COUNCIL_PROMPT.into(),
            temperature: 0.7,
            max_tokens: 1024,
            fallbacks: vec![],
        })
        .collect();
    let mut global = GlobalContext {
        goal: summarize_history(beat_id, &models[0]).await?,
        ..Default::default()
    };
    let max_turns = max_turns.unwrap_or_else(|| (agents.len() as u32) * 2);

    let (termination, transcript, artifact) = {
        let mut thread = SubContext {
            name: "council".into(),
            parent: &mut global,
            messages: vec![],
        };
        thread.push("user", &prompt);
        let out = dialogue_loop(&mut thread, &agents, max_turns, Some(beat_id)).await;
        let artifact = thread.merge_to_global(None, &models[0]).await?;

        // persist the whole run onto the beat so it survives reloads
        let mut entries = vec![json!({"role": "user", "content": prompt})];
        for t in &out.transcript {
            if t["agent_id"] != "user" {
                entries.push(json!({
                    "role": "assistant",
                    "model": t["agent_id"],
                    "content": t["content"],
                }));
            }
        }
        entries.push(json!({"role": "assistant", "model": "council", "content": artifact}));
        db::append_messages(beat_id, entries)?;
        (out.termination, out.transcript, artifact)
    };

    Ok(CouncilResult {
        termination,
        transcript,
        artifact,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_json() {
        assert_eq!(extract_json("{\"status\":\"COMPLETE\"}").unwrap()["status"], "COMPLETE");
        // JSON embedded in prose survives
        let v = extract_json("Here is my decision:\n{\"status\": \"COMPLETE\", \"n\": 2}\nThanks!").unwrap();
        assert_eq!(v["n"], 2);
        assert!(extract_json("no json here").is_none());
        assert!(extract_json("broken {json").is_none());
        // picks the outer object of nested ones
        assert_eq!(extract_json("x {\"a\": {\"b\": 1}} y").unwrap()["a"]["b"], 1);
    }

    #[test]
    fn test_conversation_done() {
        assert!(conversation_done("We agree. [DONE]"));
        assert!(conversation_done("Done. {\"status\": \"COMPLETE\"}"));
        assert!(conversation_done("{\"status\": \"complete\"}"));
        assert!(!conversation_done("Still thinking {\"status\": \"WORKING\"}"));
        assert!(!conversation_done("plain reply"));
        assert!(!conversation_done("I am DONE with this")); // marker needs brackets
    }

    #[test]
    fn test_render_for_role_mapping() {
        let mut global = GlobalContext {
            goal: "g".into(),
            constraints: vec!["c1".into()],
            ..Default::default()
        };
        let mut thread = SubContext {
            name: "t".into(),
            parent: &mut global,
            messages: vec![],
        };
        thread.push("user", "topic");
        thread.push("agent_a", "hello from A");
        thread.push("agent_b", "hello from B");
        let msgs = thread.render_for("agent_b");
        assert_eq!(msgs[0]["role"], "user"); // human topic
        assert_eq!(msgs[1]["role"], "user"); // agent A's turn is user input for B
        assert_eq!(msgs[1]["content"], "agent_a: hello from A");
        assert_eq!(msgs[2]["role"], "assistant"); // B's own turn
        assert_eq!(msgs[2]["content"], "hello from B");
    }

    #[test]
    fn test_merge_to_global_direct_artifact() {
        let mut global = GlobalContext::default();
        {
            let mut thread = SubContext {
                name: "t".into(),
                parent: &mut global,
                messages: vec![],
            };
            thread.push("user", "long stuff");
            thread.push("agent_a", "more long stuff");
            // explicit artifact skips the summarizer LLM call
            let a = tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(thread.merge_to_global(Some("consensus".into()), ""));
            assert_eq!(a.unwrap(), "consensus");
            assert!(thread.messages.is_empty()); // local tokens freed
        }
        assert_eq!(global.history.len(), 1);
        assert_eq!(global.history[0]["thread"], "t");
        assert_eq!(global.history[0]["artifact"], "consensus");
    }
}
