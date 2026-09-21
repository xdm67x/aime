//! Native Node addon exposing `pulse-core` (the agent harness) to the Pulse
//! VS Code extension. The Rust engine is shared with the Tauri app; only the
//! host shell changes.
//!
//! `#[napi]` functions are callable from the extension host (Node.js).
//! Async functions run on napi's tokio runtime; live task events stream to
//! JS through a `ThreadsafeFunction` callback.

use napi::bindgen_prelude::*;
use napi::{
    threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode},
    Error,
};
use napi_derive::napi;
use pulse_core::{beats, config, diff, harness, projects, providers, skills, tools, workflows};
use serde_json::json;
use std::sync::Arc;

fn to_err(e: String) -> Error {
    Error::new(Status::GenericFailure, e)
}

/* ---- config: API keys, base URLs, model slots ---- */

#[napi(object)]
pub struct ModelConfig {
    pub classifier: String,
    pub high: String,
    pub base: String,
    pub low: String,
}

#[napi]
pub fn get_api_key(provider: String) -> Result<Option<String>> {
    config::api_key(&provider).map_err(to_err)
}

#[napi]
pub fn save_api_key(provider: String, key: String) -> Result<()> {
    config::save_api_key(&provider, &key).map_err(to_err)
}

#[napi]
pub fn get_base_url(provider: String) -> Result<Option<String>> {
    config::base_url(&provider).map_err(to_err)
}

#[napi]
pub fn save_base_url(provider: String, url: String) -> Result<()> {
    config::save_base_url(&provider, &url).map_err(to_err)
}

#[napi]
pub fn get_model_config() -> Result<ModelConfig> {
    let c = config::ModelConfig::load().map_err(to_err)?;
    Ok(ModelConfig {
        classifier: c.classifier,
        high: c.high,
        base: c.base,
        low: c.low,
    })
}

#[napi]
pub fn save_model_config(cfg: ModelConfig) -> Result<()> {
    config::save_model_config(&config::ModelConfig {
        classifier: cfg.classifier,
        high: cfg.high,
        base: cfg.base,
        low: cfg.low,
    })
    .map_err(to_err)
}

/* ---- beats ---- */

#[napi(object)]
pub struct Beat {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub archived: bool,
    pub created_at: String,
    pub cost_usd: f64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub project_id: Option<i64>,
    pub project_name: Option<String>,
}

fn to_beat(b: beats::Beat) -> Beat {
    Beat {
        id: b.id,
        name: b.name,
        description: b.description,
        archived: b.archived,
        created_at: b.created_at,
        cost_usd: b.cost_usd,
        prompt_tokens: b.prompt_tokens,
        completion_tokens: b.completion_tokens,
        project_id: b.project_id,
        project_name: b.project_name,
    }
}

#[napi]
pub fn list_beats() -> Result<Vec<Beat>> {
    beats::list_beats()
        .map_err(to_err)
        .map(|v| v.into_iter().map(to_beat).collect())
}

#[napi]
pub fn create_beat(name: String, description: String, project_id: Option<i64>) -> Result<Beat> {
    beats::create_beat(&name, &description, project_id)
        .map(to_beat)
        .map_err(to_err)
}

#[napi]
pub fn set_beat_archived(id: i64, archived: bool) -> Result<()> {
    beats::set_beat_archived(id, archived).map_err(to_err)
}

#[napi]
pub fn delete_beat(id: i64) -> Result<String> {
    beats::delete_beat(id).map_err(to_err)
}

#[napi(ts_return_type = "Array<Record<string, unknown>>")]
pub fn get_beat_messages(id: i64) -> Result<Vec<serde_json::Value>> {
    beats::get_beat_messages(id).map_err(to_err)
}

#[napi(object)]
pub struct UsageTotal {
    pub model: String,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cost_usd: f64,
}

#[napi]
pub fn usage_totals(beat_id: i64) -> Result<Vec<UsageTotal>> {
    beats::usage_totals(beat_id).map_err(to_err).map(|v| {
        v.into_iter()
            .map(|u| UsageTotal {
                model: u.model,
                prompt_tokens: u.prompt_tokens,
                completion_tokens: u.completion_tokens,
                cost_usd: u.cost_usd,
            })
            .collect()
    })
}

/* ---- workflows ---- */

#[napi(object)]
pub struct WorkflowStep {
    /// "agent" | "ask" | "reflexion"
    pub kind: String,
    pub model: String,
    /// Custom prompt for this step. Empty → built-in default.
    pub prompt: String,
    /// Short label shown in the UI (e.g. "Plan").
    pub label: String,
}

#[napi(object)]
pub struct Workflow {
    pub id: String,
    pub name: String,
    pub description: String,
    pub steps: Vec<WorkflowStep>,
}

fn to_step(s: workflows::WorkflowStep) -> WorkflowStep {
    WorkflowStep {
        kind: s.kind.as_str().into(),
        model: s.model,
        prompt: s.prompt,
        label: s.label,
    }
}

fn from_step(s: WorkflowStep) -> std::result::Result<workflows::WorkflowStep, String> {
    let kind = workflows::StepKind::parse(&s.kind)
        .ok_or_else(|| format!("Unknown step kind: {}", s.kind))?;
    Ok(workflows::WorkflowStep {
        kind,
        model: s.model,
        prompt: s.prompt,
        label: s.label,
    })
}

fn to_workflow(w: workflows::Workflow) -> Workflow {
    Workflow {
        id: w.id,
        name: w.name,
        description: w.description,
        steps: w.steps.into_iter().map(to_step).collect(),
    }
}

fn from_workflow(w: Workflow) -> std::result::Result<workflows::Workflow, String> {
    Ok(workflows::Workflow {
        id: w.id,
        name: w.name,
        description: w.description,
        steps: w
            .steps
            .into_iter()
            .map(from_step)
            .collect::<std::result::Result<Vec<_>, String>>()?,
    })
}

#[napi]
pub fn list_workflows() -> Result<Vec<Workflow>> {
    workflows::list()
        .map_err(to_err)
        .map(|v| v.into_iter().map(to_workflow).collect())
}

#[napi]
pub fn save_workflows(wfs: Vec<Workflow>) -> Result<()> {
    let parsed: Vec<_> = wfs
        .into_iter()
        .map(from_workflow)
        .collect::<std::result::Result<Vec<_>, String>>()
        .map_err(to_err)?;
    workflows::save_all(&parsed).map_err(to_err)
}

/// Id of the workflow new sessions run by default. Empty → classifier routing.
#[napi]
pub fn get_default_workflow() -> Result<String> {
    workflows::default_id().map_err(to_err)
}

#[napi]
pub fn set_default_workflow(id: String) -> Result<()> {
    workflows::set_default_id(&id).map_err(to_err)
}

/* ---- projects ---- */

#[napi(object)]
pub struct Project {
    pub id: i64,
    pub name: String,
    pub path: String,
    pub source: String,
    pub created_at: String,
}

fn to_project(p: projects::Project) -> Project {
    Project {
        id: p.id,
        name: p.name,
        path: p.path,
        source: p.source,
        created_at: p.created_at,
    }
}

#[napi]
pub fn list_projects() -> Result<Vec<Project>> {
    projects::list_projects()
        .map_err(to_err)
        .map(|v| v.into_iter().map(to_project).collect())
}

#[napi]
pub fn add_project(path: String) -> Result<Project> {
    projects::add_project(&path).map(to_project).map_err(to_err)
}

#[napi]
pub fn remove_project(id: i64) -> Result<()> {
    projects::remove_project(id).map_err(to_err)
}

#[napi]
pub fn working_dir(beat_id: i64) -> Result<Option<String>> {
    projects::working_dir(beat_id).map_err(to_err)
}

/* ---- providers ---- */

#[napi(object)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub context_length: Option<i64>,
    pub pricing_prompt: String,
    pub pricing_completion: String,
}

fn to_model(m: providers::Model) -> Model {
    Model {
        id: m.id,
        name: m.name,
        context_length: m.context_length.map(|c| c as i64),
        pricing_prompt: m.pricing.prompt,
        pricing_completion: m.pricing.completion,
    }
}

#[napi]
pub async fn list_models() -> Result<Vec<Model>> {
    providers::list_models()
        .await
        .map_err(to_err)
        .map(|v| v.into_iter().map(to_model).collect())
}

/// Refresh the cached model list every 15 minutes on a background thread.
#[napi]
pub fn start_models_refresh() {
    std::thread::Builder::new()
        .name("pulse-models-refresh".into())
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to build refresh runtime")
                .block_on(providers::refresh_loop())
        })
        .ok();
}

/* ---- skills, tools, diff ---- */

#[napi(object)]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    pub path: String,
}

#[napi]
pub fn discover_skills() -> Result<Vec<SkillInfo>> {
    Ok(skills::discover()
        .into_iter()
        .map(|s| SkillInfo {
            name: s.name,
            description: s.description,
            path: s.path,
        })
        .collect())
}

#[napi]
pub fn unified_diff(before: String, after: String, max_lines: u32) -> String {
    diff::unified_diff(&before, &after, max_lines as usize)
}

#[napi]
pub fn strip_diff(result: String) -> String {
    tools::strip_diff(&result)
}

#[napi]
pub fn tool_definitions() -> Result<Vec<serde_json::Value>> {
    Ok(tools::definitions(&skills::discover()))
}

#[napi]
pub async fn execute_tool(name: String, arguments: String, cwd: Option<String>) -> Result<String> {
    tools::execute(&name, &arguments, cwd.as_deref())
        .await
        .map_err(to_err)
}

/* ---- harness: the agentic task runner ---- */

#[napi(object)]
pub struct ToolStep {
    pub tool: String,
    pub arguments: String,
    pub result: String,
    pub error: bool,
}

#[napi(object)]
pub struct ModelUsage {
    pub model: String,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cost_usd: f64,
}

#[napi(object)]
pub struct TaskResult {
    pub tier: String,
    pub model: String,
    pub steps: Vec<String>,
    pub tool_steps: Vec<ToolStep>,
    pub answer: String,
    pub usage: Vec<ModelUsage>,
    pub cost_usd: f64,
    pub context_percent: Option<f64>,
    pub context_full: bool,
    pub new_beat_id: Option<i64>,
}

fn to_task_result(r: harness::TaskResult) -> TaskResult {
    TaskResult {
        tier: r.tier,
        model: r.model,
        steps: r.steps,
        tool_steps: r
            .tool_steps
            .into_iter()
            .map(|s| ToolStep {
                tool: s.tool,
                arguments: s.arguments,
                result: s.result,
                error: s.error,
            })
            .collect(),
        answer: r.answer,
        usage: r
            .usage
            .into_iter()
            .map(|u| ModelUsage {
                model: u.model,
                prompt_tokens: u.prompt_tokens as i64,
                completion_tokens: u.completion_tokens as i64,
                cost_usd: u.cost_usd,
            })
            .collect(),
        cost_usd: r.cost_usd,
        context_percent: r.context_percent,
        context_full: r.context_full,
        new_beat_id: r.new_beat_id,
    }
}

#[napi]
pub fn cancel_current(beat_id: i64) {
    harness::cancel_current(beat_id)
}

/// Run one task on the harness, streaming live events to `on_event`.
/// Each event carries the beat id so several sessions can run at once.
#[napi]
pub async fn run_task(
    beat_id: i64,
    prompt: String,
    images: Vec<String>,
    #[napi(ts_arg_type = "(ev: { beatId: number } & Record<string, unknown>) => void")]
    on_event: Arc<ThreadsafeFunction<serde_json::Value, ()>>,
) -> Result<TaskResult> {
    let mut sink = {
        let on_event = on_event.clone();
        move |ev: harness::TaggedEvent| {
            let tagged = tagged_event_json(&ev);
            on_event.call(Ok(tagged), ThreadsafeFunctionCallMode::NonBlocking);
        }
    };
    harness::run_task(beat_id, prompt, images, &mut sink)
        .await
        .map(to_task_result)
        .map_err(to_err)
}

/// Run one explicit workflow by id on the beat, replacing classifier routing.
#[napi]
pub async fn run_workflow_task(
    beat_id: i64,
    workflow_id: String,
    prompt: String,
    images: Vec<String>,
    #[napi(ts_arg_type = "(ev: { beatId: number } & Record<string, unknown>) => void")]
    on_event: Arc<ThreadsafeFunction<serde_json::Value, ()>>,
) -> Result<TaskResult> {
    let mut sink = {
        let on_event = on_event.clone();
        move |ev: harness::TaggedEvent| {
            let tagged = tagged_event_json(&ev);
            on_event.call(Ok(tagged), ThreadsafeFunctionCallMode::NonBlocking);
        }
    };
    harness::run_workflow_task(beat_id, workflow_id, prompt, images, &mut sink)
        .await
        .map(to_task_result)
        .map_err(to_err)
}

/// Serialize a `TaggedEvent` for the JS callback, flattening the event fields
/// into one camelCase object (napi objects are camelCase, task events are not).
fn tagged_event_json(ev: &harness::TaggedEvent) -> serde_json::Value {
    let mut tagged = json!({
      "beatId": ev.beat_id,
      "type": match &ev.ev {
        harness::TaskEvent::Start { .. } => "start",
        harness::TaskEvent::Delta { .. } => "delta",
        harness::TaskEvent::Tool { .. } => "tool",
        harness::TaskEvent::Step { .. } => "step",
        harness::TaskEvent::StepStart { .. } => "step_start",
      },
    });
    match &ev.ev {
        harness::TaskEvent::Start { model, tier } => {
            tagged["model"] = json!(model);
            tagged["tier"] = json!(tier);
        }
        harness::TaskEvent::Delta { text } => {
            tagged["text"] = json!(text);
        }
        harness::TaskEvent::Tool {
            tool,
            arguments,
            result,
            error,
        } => {
            tagged["tool"] = json!(tool);
            tagged["arguments"] = json!(arguments);
            tagged["result"] = json!(result);
            tagged["error"] = json!(error);
        }
        harness::TaskEvent::Step { text } => {
            tagged["text"] = json!(text);
        }
        harness::TaskEvent::StepStart {
            label,
            model,
            index,
            total,
        } => {
            tagged["label"] = json!(label);
            tagged["model"] = json!(model);
            tagged["index"] = json!(index);
            tagged["total"] = json!(total);
        }
    }
    tagged
}
