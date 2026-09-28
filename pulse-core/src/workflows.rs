//! Workflows: YAML-defined multi-step runs. Each step's prompt goes through
//! the agentic tool loop; a step that declares a `goal:` is re-run with the
//! reviewer's feedback until the model confirms the goal is reached (bounded
//! by [`MAX_GOAL_ATTEMPTS`]).
//!
//! Workflows live as files the user points at — `pulse workflow new <title>`
//! writes a blank template into `./.pulse/workflows` (or `~/.pulse/workflows`
//! with `--global`), and
//! [`find`] resolves a workflow by name (`./.pulse/workflows` first, then the
//! current directory, then `~/.pulse/workflows`) or by file path. Each step names the model that
//! runs it (step-level `model:`, falling back to the workflow-level
//! `model:`); a step with neither is an error. A step's prompt can
//! reference the final result of an earlier step with a `{{steps.<name>}}`
//! placeholder, so steps build on each other; [`Workflow::validate`]
//! rejects references that don't resolve before any provider call is made.
//! A runtime that supplies a user message (e.g. `harness::run_task` for
//! plain prompts) fills `{{prompt}}` placeholders with it — the workflow
//! CLI passes none.

use crate::harness::{self, OnEvent, TaskResult};
use crate::prompts;
use serde::Deserialize;
use serde_json::json;
use std::path::{Path, PathBuf};

/// The workflow plain prompts run through. Created by
/// `pulse workflow new base`.
pub const BASE: &str = "base";

/// How many times a step with a `goal:` runs at most (the first attempt plus
/// retries with reviewer feedback) before the workflow accepts the last
/// result and reports the goal as unmet.
pub const MAX_GOAL_ATTEMPTS: usize = 3;

/// Normalize an optional model id: trimmed, and `None` when empty.
fn non_empty(m: Option<&str>) -> Option<&str> {
    m.map(str::trim).filter(|m| !m.is_empty())
}

/// YAML `null` (a bare `key:` line in a blank template) reads as a missing
/// value — treat it as the empty string so templates parse untouched.
fn tolerant_string<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Option::<String>::deserialize(d).map(|v| v.unwrap_or_default())
}

#[derive(Clone, Debug, Deserialize)]
pub struct Workflow {
    pub name: String,
    #[serde(default, deserialize_with = "tolerant_string")]
    pub description: String,
    /// Default model for all steps. Overridden by a step-level model.
    #[serde(default)]
    pub model: Option<String>,
    pub steps: Vec<WorkflowStep>,
}

impl Workflow {
    /// Model used for auxiliary calls (compaction, history summaries): the
    /// workflow-level model, else the first step that declares one.
    pub fn default_model(&self) -> Option<&str> {
        non_empty(self.model.as_deref()).or_else(|| {
            self.steps
                .iter()
                .find_map(|s| non_empty(s.model.as_deref()))
        })
    }

    /// Model that runs `step`: the step's own `model:`, else the workflow's.
    /// `None` is an error — there is no other model source.
    pub fn step_model<'a>(&'a self, step: &'a WorkflowStep) -> Result<&'a str, String> {
        non_empty(step.model.as_deref())
            .or_else(|| non_empty(self.model.as_deref()))
            .ok_or_else(|| {
                format!(
                    "Workflow '{}' step '{}' has no model — set `model:` at the step or \
                     workflow level in the workflow file (list ids with: pulse models)",
                    self.name, step.name
                )
            })
    }

    /// Everything a run needs up front: at least one step, a resolvable
    /// model for every step, and `{{steps.<name>}}` references (the
    /// whitespace-tolerant `{{ steps.<name> }}` spelling included) that
    /// only point at steps running earlier. Fails before any provider
    /// call is made.
    pub fn validate(&self) -> Result<(), String> {
        if self.steps.is_empty() {
            return Err(format!("Workflow '{}' has no steps", self.name));
        }
        let mut done: Vec<&str> = Vec::with_capacity(self.steps.len());
        for step in &self.steps {
            self.step_model(step)?;
            for name in step_refs(&step.prompt) {
                if !done.contains(&name.as_str()) {
                    return Err(format!(
                        "Workflow '{}' step '{}' uses '{{{{steps.{name}}}}}' but no earlier step is named '{name}' — steps can only reference the results of steps that run before them",
                        self.name, step.name
                    ));
                }
            }
            done.push(step.name.as_str());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct WorkflowStep {
    pub name: String,
    pub prompt: String,
    /// Per-step model override. `None` → the workflow-level model (required
    /// somewhere — a step with neither fails at run time).
    #[serde(default)]
    pub model: Option<String>,
    /// What must be true when the step is done. When set, the step re-runs
    /// with reviewer feedback until the model confirms the goal is reached.
    #[serde(default)]
    pub goal: Option<String>,
}

/// `{{steps.<name>}}` references in a prompt, in order of appearance. Names
/// are taken verbatim between the marker and the closing `}}`; whitespace
/// inside the braces is tolerated (`{{ steps.<name> }}` names the same
/// step as `{{steps.<name>}}`).
fn step_refs(prompt: &str) -> Vec<String> {
    let mut refs = Vec::new();
    let mut rest = prompt;
    while let Some(open) = rest.find("{{") {
        let Some(close) = rest[open + 2..].find("}}") else {
            break;
        };
        let inner = rest[open + 2..open + 2 + close].trim();
        if let Some(name) = inner.strip_prefix("steps.") {
            if !name.is_empty() && !name.contains("{{") {
                refs.push(name.to_string());
            }
        }
        rest = &rest[open + 4 + close..];
    }
    refs
}

/// The prompt a step runs with: `{{steps.<name>}}` placeholders filled with
/// the answers of the steps that ran before it (the first earlier step wins
/// when names repeat), and `{{prompt}}` with the runtime-supplied message
/// when there is one. [`Workflow::validate`] guarantees the references
/// resolve; anything else stays visible, like `prompts::fill` does for
/// template typos.
fn effective_prompt(
    step: &WorkflowStep,
    user_prompt: Option<&str>,
    done: &[WorkflowStepResult],
) -> String {
    let mut vars: Vec<(String, String)> = done
        .iter()
        .map(|s| (format!("steps.{}", s.name), s.answer.clone()))
        .collect();
    if let Some(p) = user_prompt {
        vars.push(("prompt".to_string(), p.to_string()));
    }
    let vars: Vec<(&str, &str)> = vars.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    prompts::fill(&step.prompt, &vars)
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct WorkflowRunResult {
    pub steps: Vec<WorkflowStepResult>,
    pub total_cost_usd: f64,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct WorkflowStepResult {
    pub name: String,
    pub answer: String,
    /// Resolved model id that ran the step.
    pub model: String,
    pub cost_usd: f64,
    /// For goal steps: how many runs the step took (1 = first try). `None`
    /// when the step declares no goal.
    pub attempts: Option<usize>,
    /// For goal steps: whether the goal was confirmed reached. `None` when
    /// the step declares no goal; `Some(false)` means the step still did not
    /// meet its goal after [`MAX_GOAL_ATTEMPTS`] runs.
    pub goal_met: Option<bool>,
}

/// Progress hooks for a workflow run. Every hook is host-runtime machinery —
/// the CLI uses them to print step progress and build the markdown report;
/// [`run`] wraps [`run_hooked`] with no-op hooks.
pub struct RunHooks<'a> {
    /// A step is starting: 0-based index, the step, and the effective prompt
    /// (placeholders filled, goal appended) about to run.
    pub on_step_start: &'a mut (dyn FnMut(usize, &WorkflowStep, &str) + Send),
    /// A step finished — its goal verified when it declares one.
    pub on_step_done: &'a mut (dyn FnMut(&WorkflowStepResult) + Send),
    /// A step's goal was not reached and it is about to run again:
    /// the step, the 1-based attempt number about to start, the reviewer's
    /// reason.
    pub on_goal_retry: &'a mut (dyn FnMut(&WorkflowStep, usize, &str) + Send),
    /// Live harness events for the currently running step.
    pub on_event: OnEvent<'a>,
}

/// The directory of "installed" (global) workflows: `~/.pulse/workflows`.
pub fn dir() -> Result<PathBuf, String> {
    let home = std::env::var("HOME").map_err(|e| e.to_string())?;
    let dir = PathBuf::from(home).join(".pulse").join("workflows");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// The directory of local workflows for the current directory:
/// `./.pulse/workflows` (created if missing).
pub fn local_dir() -> Result<PathBuf, String> {
    let dir = PathBuf::from(".").join(".pulse").join("workflows");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// `./.pulse/workflows` without creating it — for read-only lookups.
fn local_dir_opt() -> PathBuf {
    PathBuf::from(".").join(".pulse").join("workflows")
}

/// Load a single workflow by name from `~/.pulse/workflows` (looks for
/// `{name}.yml` then `{name}.yaml`). Use [`find`] for the full name/path
/// lookup the CLI does.
pub fn load(name: &str) -> Result<Workflow, String> {
    let dir = dir()?;
    for ext in &["yml", "yaml"] {
        let path = dir.join(format!("{name}.{ext}"));
        if path.is_file() {
            let content = std::fs::read_to_string(&path)
                .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
            return serde_yaml::from_str(&content).map_err(|e| {
                crate::log::warn(format!("failed to parse workflow {}: {e}", path.display()));
                format!("Failed to parse {}: {e}", path.display())
            });
        }
    }
    crate::log::warn(format!("workflow '{name}' not found in {}", dir.display()));
    Err(format!("Workflow '{name}' not found in {}", dir.display()))
}

/// Every workflow-shaped yaml file in `dir`; files that don't parse as a
/// workflow are skipped. Sorted by workflow name.
pub fn discover_dir(dir: &Path) -> Vec<(Workflow, PathBuf)> {
    let mut out = vec![];
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext != "yml" && ext != "yaml" {
                continue;
            }
            if let Ok((wf, _)) = load_file(&path) {
                out.push((wf, path));
            }
        }
    }
    out.sort_by(|a, b| a.0.name.cmp(&b.0.name));
    out
}

/// Scan `~/.pulse/workflows/*.yml` and `*.yaml`, parse each into a [`Workflow`].
pub fn discover() -> Result<Vec<Workflow>, String> {
    let dir = dir()?;
    let mut workflows = vec![];
    for (wf, _) in discover_dir(&dir) {
        workflows.push(wf);
    }
    Ok(workflows)
}

/// Parse one workflow file.
pub fn load_file(path: &Path) -> Result<(Workflow, PathBuf), String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
    let wf: Workflow = serde_yaml::from_str(&content)
        .map_err(|e| format!("Failed to parse {}: {e}", path.display()))?;
    Ok((wf, path.to_path_buf()))
}

/// Resolve a workflow by name or file path. A value with a path separator or
/// a `.yml`/`.yaml` suffix is treated as a path; otherwise `{name}.yml` /
/// `{name}.yaml` is looked up in `./.pulse/workflows` first, then the current
/// directory, then in `~/.pulse/workflows`.
pub fn find(name_or_path: &str) -> Result<(Workflow, PathBuf), String> {
    let s = name_or_path.trim();
    if s.is_empty() {
        return Err("Workflow name cannot be empty".into());
    }
    if s.contains('/') || s.ends_with(".yml") || s.ends_with(".yaml") {
        let path = PathBuf::from(s);
        if !path.is_file() {
            return Err(format!("Workflow file not found: {}", path.display()));
        }
        return load_file(&path);
    }
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    for dir in [local_dir_opt(), cwd, dir()?] {
        for ext in ["yml", "yaml"] {
            let path = dir.join(format!("{s}.{ext}"));
            if path.is_file() {
                return load_file(&path);
            }
        }
    }
    Err(format!(
        "Workflow '{s}' not found — looked for ./.pulse/workflows/{s}.yml, ./{s}.yml and ~/.pulse/workflows/{s}.yml"
    ))
}

/// The blank workflow template written by `pulse workflow new <title>` into
/// the current directory. It parses as-is (that's the test), but needs a
/// `model:` before it can run.
pub fn template(name: &str) -> String {
    format!(
        "# Pulse workflow — run it with: pulse {name}\n\
         # Model ids: pulse models\n\
         \n\
         name: {name}\n\
         description: # what this workflow does\n\
         model: # required — a model id from `pulse models`\n\
         \n\
         steps:\n\
         \x20 - name: step-1\n\
         \x20   # Optional: when set, the step re-runs with reviewer feedback\n\
         \x20   # until the goal is confirmed reached (max {MAX_GOAL_ATTEMPTS} runs):\n\
         \x20   goal: |\n\
         \x20     What must be true when this step is done.\n\
         \x20   # Optional: {{{{steps.<name>}}}} inserts the final result of an\n\
         \x20   # earlier step into this prompt, so steps can build on each\n\
         \x20   # other (whitespace inside the braces is tolerated):\n\
         \x20   prompt: |\n\
         \x20     Instructions for the model.\n"
    )
}

/// Write a single-step workflow file (`{name}.yml`) into `~/.pulse/workflows`.
/// Returns the file's path.
pub fn create(
    name: &str,
    description: &str,
    model: &str,
    step_prompt: &str,
) -> Result<PathBuf, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Workflow name cannot be empty".into());
    }
    let dir = dir()?;
    let path = dir.join(format!("{name}.yml"));
    if path.is_file() {
        return Err(format!("Workflow already exists: {}", path.display()));
    }
    let indented_prompt: String = step_prompt
        .lines()
        .map(|l| format!("      {l}\n"))
        .collect();
    let yaml = format!(
        "name: {}\ndescription: {}\nmodel: {}\nsteps:\n  - name: respond\n    prompt: |\n{}",
        yaml_scalar(name),
        yaml_scalar(description.trim()),
        yaml_scalar(model.trim()),
        indented_prompt,
    );
    std::fs::write(&path, yaml).map_err(|e| format!("Failed to write {}: {e}", path.display()))?;
    crate::log::info(format!("workflow created: {}", path.display()));
    Ok(path)
}

/// Quote a string as a single-quoted YAML scalar.
fn yaml_scalar(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/* ---- goal verification ---- */

/// Ask the step's model to verify its result against the step's goal.
/// Returns (achieved, reason). A malformed verdict counts as achieved: the
/// step's agentic loop already ran to completion, and a broken verifier
/// must not block the workflow.
async fn goal_reached(model: &str, goal: &str, answer: &str) -> Result<(bool, String), String> {
    let prompt = prompts::fill(prompts::GOAL_CHECK, &[("goal", goal), ("result", answer)]);
    let r = crate::providers::chat_completion(
        model,
        &[json!({"role": "user", "content": prompt})],
        &[],
        Some(0.0),
        Some(256),
        true,
        None,
    )
    .await?;
    Ok(parse_goal_verdict(&r.content).unwrap_or_else(|| {
        crate::log::warn(format!(
            "unparseable goal verdict, accepting step result: {}",
            r.content.trim()
        ));
        (true, String::new())
    }))
}

/// Pull `{"achieved": bool, "reason": "…"}` out of a model reply, tolerating
/// code fences and prose around the JSON object.
fn parse_goal_verdict(text: &str) -> Option<(bool, String)> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end < start {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(&text[start..=end]).ok()?;
    let achieved = v["achieved"].as_bool()?;
    let reason = v["reason"].as_str().unwrap_or_default().trim().to_string();
    Some((achieved, reason))
}

/* ---- execution ---- */

/// Execute a workflow against an existing beat: every step runs through the
/// agentic loop, and a step with a `goal:` re-runs until the goal is
/// confirmed. `user_prompt` fills `{{prompt}}` placeholders when a runtime
/// supplies a message (the workflow CLI passes none). No-op hooks; hosts
/// that want progress use [`run_hooked`].
pub async fn run(
    beat_id: i64,
    workflow: &Workflow,
    user_prompt: Option<&str>,
    images: &[String],
    on_event: OnEvent<'_>,
) -> Result<WorkflowRunResult, String> {
    let mut on_step_start = |_idx: usize, _step: &WorkflowStep, _prompt: &str| {};
    let mut on_step_done = |_r: &WorkflowStepResult| {};
    let mut on_goal_retry = |_step: &WorkflowStep, _attempt: usize, _reason: &str| {};
    run_hooked(
        beat_id,
        workflow,
        user_prompt,
        images,
        RunHooks {
            on_step_start: &mut on_step_start,
            on_step_done: &mut on_step_done,
            on_goal_retry: &mut on_goal_retry,
            on_event,
        },
    )
    .await
}

/// [`run`] with progress hooks. Each step runs as a separate
/// `run_task_with_model` call on the same `beat_id`, so prior context
/// accumulates; a step with a `goal:` whose result fails verification runs
/// again with the reviewer's reason fed back, at most [`MAX_GOAL_ATTEMPTS`]
/// times. `user_prompt` fills `{{prompt}}` placeholders when the calling
/// runtime supplies a message.
pub async fn run_hooked(
    beat_id: i64,
    workflow: &Workflow,
    user_prompt: Option<&str>,
    images: &[String],
    hooks: RunHooks<'_>,
) -> Result<WorkflowRunResult, String> {
    let RunHooks {
        on_step_start,
        on_step_done,
        on_goal_retry,
        on_event,
    } = hooks;
    workflow.validate()?;
    let mut steps = Vec::with_capacity(workflow.steps.len());
    let mut total_cost = 0.0;
    crate::log::info(format!(
        "beat {beat_id}: workflow '{}' starting ({} steps)",
        workflow.name,
        workflow.steps.len()
    ));

    for (idx, step) in workflow.steps.iter().enumerate() {
        let model = workflow.step_model(step)?;
        let goal = non_empty(step.goal.as_deref());
        let mut prompt = effective_prompt(step, user_prompt, &steps);
        if let Some(goal) = goal {
            prompt.push_str("\n\nGoal — work until this is fully reached:\n");
            prompt.push_str(goal);
        }
        // the user's images belong to the user's message — attach them to
        // the step that carries it (always the first in practice)
        let mut imgs = if idx == 0 { images.to_vec() } else { vec![] };

        on_step_start(idx, step, &prompt);

        let mut attempts = 0usize;
        let mut goal_met: Option<bool> = None;
        let result: TaskResult = loop {
            attempts += 1;
            let r: TaskResult = Box::pin(harness::run_task_with_model(
                beat_id,
                prompt.clone(),
                model.to_string(),
                workflow.name.clone(),
                imgs.clone(),
                on_event,
            ))
            .await?;
            let Some(goal) = goal else {
                break r;
            };
            let (achieved, reason) = match goal_reached(model, goal, &r.answer).await {
                Ok(v) => v,
                Err(e) => {
                    // the step itself completed; a broken verifier must not
                    // kill the run — accept the result and carry the reason
                    crate::log::warn(format!(
                        "beat {beat_id}: goal check failed, accepting step result: {e}"
                    ));
                    goal_met = Some(true);
                    break r;
                }
            };
            if achieved {
                goal_met = Some(true);
                break r;
            }
            if attempts >= MAX_GOAL_ATTEMPTS {
                goal_met = Some(false);
                break r;
            }
            crate::log::info(format!(
                "beat {beat_id}: workflow '{}' step '{}' goal not reached (attempt {attempts}/{MAX_GOAL_ATTEMPTS})",
                workflow.name, step.name
            ));
            on_goal_retry(step, attempts + 1, &reason);
            prompt = prompts::fill(prompts::GOAL_RETRY, &[("goal", goal), ("reason", &reason)]);
            imgs.clear();
        };

        let step_cost = result.cost_usd;
        let sr = WorkflowStepResult {
            name: step.name.clone(),
            answer: result.answer.clone(),
            model: result.model.clone(),
            cost_usd: step_cost,
            attempts: goal.map(|_| attempts),
            goal_met,
        };
        crate::log::info(format!(
            "beat {beat_id}: workflow '{}' step '{}' done (model={}, attempts={}, goal_met={:?}, cost=${:.4})",
            workflow.name, sr.name, sr.model, attempts, sr.goal_met, sr.cost_usd
        ));
        total_cost += step_cost;
        on_step_done(&sr);
        steps.push(sr);
    }

    Ok(WorkflowRunResult {
        steps,
        total_cost_usd: total_cost,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_workflow_yaml() {
        let yaml = r#"
name: code-review
description: Automated code review
model: OpenRouter - anthropic/claude-3.5-sonnet
steps:
  - name: analyze
    prompt: |
      Analyze the codebase structure.
  - name: report
    prompt: |
      Generate a detailed code review report.
    model: OpenRouter - openai/gpt-4o
"#;
        let wf: Workflow = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(wf.name, "code-review");
        assert_eq!(wf.description, "Automated code review");
        assert_eq!(
            wf.model.as_deref(),
            Some("OpenRouter - anthropic/claude-3.5-sonnet")
        );
        assert_eq!(wf.steps.len(), 2);
        assert_eq!(wf.steps[0].name, "analyze");
        assert!(wf.steps[0].model.is_none());
        assert!(wf.steps[0].goal.is_none());
        assert_eq!(wf.steps[1].name, "report");
        assert_eq!(
            wf.steps[1].model.as_deref(),
            Some("OpenRouter - openai/gpt-4o")
        );
        // model resolution: step override > workflow default
        assert_eq!(
            wf.step_model(&wf.steps[0]).unwrap(),
            "OpenRouter - anthropic/claude-3.5-sonnet"
        );
        assert_eq!(
            wf.step_model(&wf.steps[1]).unwrap(),
            "OpenRouter - openai/gpt-4o"
        );
        assert_eq!(
            wf.default_model().unwrap(),
            "OpenRouter - anthropic/claude-3.5-sonnet"
        );
        // runnable as-is
        assert!(wf.validate().is_ok());
    }

    #[test]
    fn test_step_refs_extracted() {
        assert_eq!(step_refs("no refs here"), Vec::<String>::new());
        assert_eq!(
            step_refs("Use {{steps.analyze}} then {{steps.build}}. and"),
            vec!["analyze", "build"]
        );
        assert_eq!(
            step_refs("{{steps.a}} {{steps.b}} {{steps.a}}"),
            vec!["a", "b", "a"]
        );
        assert_eq!(step_refs("{{prompt}} and {{steps.x}}"), vec!["x"]);
        assert_eq!(step_refs("{{steps.unterminated"), Vec::<String>::new());
        assert_eq!(step_refs("{{steps.}}"), Vec::<String>::new());
        assert_eq!(step_refs("{{steps.a{{b}}"), Vec::<String>::new());
        // whitespace inside the braces is tolerated
        assert_eq!(step_refs("{{ steps.a }}"), vec!["a"]);
        assert_eq!(step_refs("{{ steps.a.output }}"), vec!["a.output"]);
    }

    #[test]
    fn test_effective_prompt_fills_step_results() {
        let step = WorkflowStep {
            name: "report".into(),
            prompt: "Based on {{steps.analyze}} (and {{prompt}}), report.".into(),
            model: None,
            goal: None,
        };
        let done = vec![WorkflowStepResult {
            name: "analyze".into(),
            answer: "3 modules".into(),
            model: "m".into(),
            cost_usd: 0.0,
            attempts: None,
            goal_met: None,
        }];
        assert_eq!(
            effective_prompt(&step, Some("go deep"), &done),
            "Based on 3 modules (and go deep), report."
        );
        assert_eq!(
            effective_prompt(&step, None, &done),
            "Based on 3 modules (and {{prompt}}), report."
        );
        assert_eq!(
            effective_prompt(&step, None, &[]),
            "Based on {{steps.analyze}} (and {{prompt}}), report."
        );

        // padded spelling fills the same answer
        let step = WorkflowStep {
            name: "report".into(),
            prompt: "Risk level: {{ steps.analyze }}".into(),
            model: None,
            goal: None,
        };
        assert_eq!(
            effective_prompt(&step, None, &done),
            "Risk level: 3 modules"
        );
    }

    #[test]
    fn test_validate_step_refs() {
        let yaml = r#"
name: chain
model: m
steps:
  - name: build
    prompt: "Start from {{steps.report}}."
  - name: report
    prompt: Report.
"#;
        let wf: Workflow = serde_yaml::from_str(yaml).unwrap();
        let err = wf.validate().unwrap_err();
        assert!(
            err.contains("no earlier step is named 'report'"),
            "got: {err}"
        );

        let yaml = r#"
name: loop
model: m
steps:
  - name: build
    prompt: "Echo {{steps.build}}."
"#;
        let wf: Workflow = serde_yaml::from_str(yaml).unwrap();
        let err = wf.validate().unwrap_err();
        assert!(
            err.contains("no earlier step is named 'build'"),
            "got: {err}"
        );

        let yaml = r#"
name: chain-ok
model: m
steps:
  - name: build
    prompt: Build.
  - name: report
    prompt: "Report on {{steps.build}}."
  - name: ship
    prompt: "Ship {{steps.build}} via {{steps.report}}."
"#;
        let wf: Workflow = serde_yaml::from_str(yaml).unwrap();
        assert!(wf.validate().is_ok());

        let yaml = r#"
name: typo
model: m
steps:
  - name: a
    prompt: A.
  - name: b
    prompt: "Use {{steps.c}}."
"#;
        let wf: Workflow = serde_yaml::from_str(yaml).unwrap();
        let err = wf.validate().unwrap_err();
        assert!(err.contains("no earlier step is named 'c'"), "got: {err}");

        // padded spelling resolves like the exact one
        let yaml = r#"
name: output-ref-ok
model: m
steps:
  - name: build
    prompt: Build.
  - name: report
    prompt: "Report on {{ steps.build }} and {{steps.build}}."
"#;
        let wf: Workflow = serde_yaml::from_str(yaml).unwrap();
        assert!(wf.validate().is_ok());

        // .output is not a suffix the engine strips — it is part of the name,
        // so such a reference fails loudly instead of silently staying put
        let yaml = r#"
name: output-ref
model: m
steps:
  - name: build
    prompt: "Use {{ steps.build.output }}."
"#;
        let wf: Workflow = serde_yaml::from_str(yaml).unwrap();
        let err = wf.validate().unwrap_err();
        assert!(
            err.contains("no earlier step is named 'build.output'"),
            "got: {err}"
        );

        let yaml = r#"
name: output-ref-loop
model: m
steps:
  - name: build
    prompt: "Use {{ steps.report }}."
  - name: report
    prompt: Report.
"#;
        let wf: Workflow = serde_yaml::from_str(yaml).unwrap();
        let err = wf.validate().unwrap_err();
        assert!(
            err.contains("no earlier step is named 'report'"),
            "got: {err}"
        );
    }

    #[tokio::test]
    async fn test_run_fails_fast_on_unresolvable_step_ref() {
        let yaml = r#"
name: runtime
model: m
steps:
  - name: a
    prompt: "Use {{steps.missing}}."
"#;
        let wf: Workflow = serde_yaml::from_str(yaml).unwrap();
        let err = run(1, &wf, None, &[], &mut |_| {}).await.unwrap_err();
        assert!(
            err.contains("no earlier step is named 'missing'"),
            "got: {err}"
        );
    }

    #[test]
    fn test_parse_workflow_goal() {
        let yaml = r#"
name: ship
model: gpt-4o
steps:
  - name: build
    goal: |
      The project compiles with no errors.
    prompt: Fix the build.
"#;
        let wf: Workflow = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            wf.steps[0].goal.as_deref().unwrap().trim(),
            "The project compiles with no errors."
        );
    }

    #[test]
    fn test_parse_workflow_null_fields() {
        // the blank template carries bare `key:` lines — they must parse
        let wf: Workflow = serde_yaml::from_str(
            "name: x\ndescription:\nmodel:\nsteps:\n  - name: s\n    prompt: p\n",
        )
        .unwrap();
        assert_eq!(wf.description, "");
        assert!(wf.model.is_none());
        assert!(wf.validate().is_err()); // no model anywhere
    }

    #[test]
    fn test_parse_workflow_no_model() {
        let yaml = r#"
name: simple
description: A simple workflow
steps:
  - name: step1
    prompt: Do something
"#;
        let wf: Workflow = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(wf.name, "simple");
        assert!(wf.model.is_none());
        assert_eq!(wf.steps.len(), 1);
        assert!(wf.steps[0].model.is_none());
        // no model anywhere: both resolvers refuse
        assert!(wf.step_model(&wf.steps[0]).is_err());
        assert!(wf.default_model().is_none());
    }

    #[test]
    fn test_default_model_falls_back_to_first_step_model() {
        let yaml = r#"
name: mixed
steps:
  - name: a
    prompt: x
  - name: b
    prompt: y
    model: OpenRouter - openai/gpt-4o
"#;
        let wf: Workflow = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(wf.default_model().unwrap(), "OpenRouter - openai/gpt-4o");
        assert!(wf.step_model(&wf.steps[0]).is_err());
        assert_eq!(
            wf.step_model(&wf.steps[1]).unwrap(),
            "OpenRouter - openai/gpt-4o"
        );
    }

    #[test]
    fn test_validate_no_steps() {
        let yaml = "\nname: empty\nmodel: m\nsteps: []\n";
        let wf: Workflow = serde_yaml::from_str(yaml).unwrap();
        assert!(wf.validate().unwrap_err().contains("no steps"));
    }

    #[test]
    fn test_parse_workflow_missing_name() {
        let yaml = r#"
description: No name
steps:
  - name: x
    prompt: y
"#;
        let result: Result<Workflow, _> = serde_yaml::from_str(yaml);
        assert!(result.is_err());
    }

    #[test]
    fn test_template_parses() {
        let wf: Workflow = serde_yaml::from_str(&template("blank")).unwrap();
        assert_eq!(wf.name, "blank");
        assert_eq!(wf.steps.len(), 1);
        assert!(wf.steps[0].goal.is_some());
        assert!(wf.validate().is_err()); // template ships without a model
    }

    #[test]
    fn test_parse_goal_verdict() {
        // plain JSON
        assert_eq!(
            parse_goal_verdict(r#"{"achieved": false, "reason": "tests still fail"}"#),
            Some((false, "tests still fail".into()))
        );
        // fenced / prose-wrapped
        let fenced = "Here you go:\n```json\n{\"achieved\": true, \"reason\": \"all good\"}\n```\n";
        assert_eq!(parse_goal_verdict(fenced), Some((true, "all good".into())));
        // missing reason is fine
        assert_eq!(
            parse_goal_verdict(r#"{"achieved": true}"#),
            Some((true, "".into()))
        );
        // not JSON at all
        assert_eq!(parse_goal_verdict("looks done to me"), None);
        // achieved not a boolean
        assert_eq!(parse_goal_verdict(r#"{"achieved": "yes"}"#), None);
    }

    #[test]
    fn test_find_by_path() {
        // path form: no HOME access, no cwd dependence
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wf.yml");
        std::fs::write(
            &path,
            "name: found\nmodel: m\nsteps:\n  - name: s\n    prompt: p\n",
        )
        .unwrap();
        let (wf, found) = find(path.to_str().unwrap()).unwrap();
        assert_eq!(wf.name, "found");
        assert_eq!(found, path);
        // a .yml path that doesn't exist
        let missing = dir.path().join("nope.yml");
        assert!(find(missing.to_str().unwrap())
            .unwrap_err()
            .contains("not found"));
    }

    #[test]
    fn test_discover_empty_dir() {
        // discover() creates the dir if missing and returns empty when no files exist.
        // The ~/.pulse/workflows dir may have files from other tests; just check it
        // doesn't panic and returns a vec.
        let result = discover();
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_run_requires_a_model() {
        let yaml = r#"
name: modelless
steps:
  - name: step1
    prompt: Do something
"#;
        let wf: Workflow = serde_yaml::from_str(yaml).unwrap();
        let err = run(1, &wf, None, &[], &mut |_| {}).await.unwrap_err();
        assert!(err.contains("has no model"), "got: {err}");
    }

    #[test]
    fn test_create_writes_loadable_workflow() {
        // shares log::HOME_LOCK: std::env::set_var("HOME") is process-global
        // and races across parallel tests (also with the db/log tests)
        let _g = crate::log::HOME_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", dir.path());
        let path = create(
            BASE,
            "the default workflow",
            "OpenRouter - anthropic/claude-3.5-sonnet",
            "{{prompt}}",
        )
        .unwrap();
        assert!(path.is_file());
        let wf = load(BASE).unwrap();
        assert_eq!(wf.name, BASE);
        assert_eq!(wf.description, "the default workflow");
        assert_eq!(
            wf.default_model().unwrap(),
            "OpenRouter - anthropic/claude-3.5-sonnet"
        );
        assert_eq!(wf.steps.len(), 1);
        assert_eq!(wf.steps[0].prompt.trim(), "{{prompt}}");
        // discover() picks it up
        assert!(discover().unwrap().iter().any(|w| w.name == BASE));
    }
}
