//! Workflows: YAML-defined multi-step prompts stored in `~/.pulse/workflows/`.
//!
//! Every prompt runs through a workflow — there are no global model slots.
//! Each step names the model that runs it (step-level `model:`, falling back
//! to the workflow-level `model:`); a step with neither is an error. A step
//! prompt can carry a `{{prompt}}` placeholder, filled with the user's message
//! when the workflow runs implicitly (a plain prompt runs the `base`
//! workflow). Invoke one explicitly from the harness via `/workflow {name}`.

use crate::harness::{self, OnEvent, TaskResult};
use crate::prompts;
use serde::Deserialize;

/// The workflow plain prompts run through. Created by the TUI's onboarding
/// (or `pulse workflow new base`).
pub const BASE: &str = "base";

/// Normalize an optional model id: trimmed, and `None` when empty.
fn non_empty(m: Option<&str>) -> Option<&str> {
    m.map(str::trim).filter(|m| !m.is_empty())
}

#[derive(Clone, Debug, Deserialize)]
pub struct Workflow {
    pub name: String,
    #[serde(default)]
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
        non_empty(self.model.as_deref())
            .or_else(|| self.steps.iter().find_map(|s| non_empty(s.model.as_deref())))
    }

    /// Model that runs `step`: the step's own `model:`, else the workflow's.
    /// `None` is an error — there is no other model source.
    pub fn step_model<'a>(&'a self, step: &'a WorkflowStep) -> Result<&'a str, String> {
        non_empty(step.model.as_deref())
            .or_else(|| non_empty(self.model.as_deref()))
            .ok_or_else(|| {
                format!(
                    "Workflow '{}' step '{}' has no model — set `model:` at the step or \
                     workflow level in ~/.pulse/workflows/{}.yml (list ids with /models).",
                    self.name, step.name, self.name
                )
            })
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
}

fn workflows_dir() -> Result<std::path::PathBuf, String> {
    let home = std::env::var("HOME").map_err(|e| e.to_string())?;
    let dir = std::path::PathBuf::from(home)
        .join(".pulse")
        .join("workflows");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// Scan `~/.pulse/workflows/*.yml` and `*.yaml`, parse each into a [`Workflow`].
pub fn discover() -> Result<Vec<Workflow>, String> {
    let dir = workflows_dir()?;
    let mut workflows = vec![];
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext != "yml" && ext != "yaml" {
                continue;
            }
            let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            let wf: Workflow = serde_yaml::from_str(&content)
                .map_err(|e| format!("Failed to parse {}: {e}", path.display()))?;
            workflows.push(wf);
        }
    }
    workflows.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(workflows)
}

/// Load a single workflow by name (looks for `{name}.yml` then `{name}.yaml`).
pub fn load(name: &str) -> Result<Workflow, String> {
    let dir = workflows_dir()?;
    for ext in &["yml", "yaml"] {
        let path = dir.join(format!("{name}.{ext}"));
        if path.is_file() {
            let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            return serde_yaml::from_str(&content).map_err(|e| {
                crate::log::warn(format!("failed to parse workflow {}: {e}", path.display()));
                format!("Failed to parse {}: {e}", path.display())
            });
        }
    }
    crate::log::warn(format!("workflow '{name}' not found in {}", dir.display()));
    Err(format!("Workflow '{name}' not found in {}", dir.display()))
}

/// Write a single-step workflow file (`{name}.yml`) — the onboarding's
/// "create the base workflow" path. Returns the file's path.
pub fn create(
    name: &str,
    description: &str,
    model: &str,
    step_prompt: &str,
) -> Result<std::path::PathBuf, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Workflow name cannot be empty".into());
    }
    let dir = workflows_dir()?;
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

/// Execute a workflow against an existing beat. Each step runs as a separate
/// `run_task_with_model` call on the same `beat_id`, so prior context
/// accumulates. `user_prompt` (the user's message, when the workflow runs
/// implicitly) fills `{{prompt}}` placeholders in step prompts. The
/// `on_event` callback receives live `TaggedEvent`s.
pub async fn run(
    beat_id: i64,
    workflow: &Workflow,
    user_prompt: Option<&str>,
    images: &[String],
    on_event: OnEvent<'_>,
) -> Result<WorkflowRunResult, String> {
    let mut steps = Vec::with_capacity(workflow.steps.len());
    let mut total_cost = 0.0;
    crate::log::info(format!(
        "beat {beat_id}: workflow '{}' starting ({} steps)",
        workflow.name,
        workflow.steps.len()
    ));

    for (idx, step) in workflow.steps.iter().enumerate() {
        let model = workflow.step_model(step)?;
        let prompt = match user_prompt {
            Some(p) => prompts::fill(&step.prompt, &[("prompt", p)]),
            None => step.prompt.clone(),
        };
        // the user's images belong to the user's message — attach them to
        // the step that carries it (always the first in practice)
        let imgs = if idx == 0 { images.to_vec() } else { vec![] };
        let result: TaskResult = Box::pin(harness::run_task_with_model(
            beat_id,
            prompt,
            model.to_string(),
            workflow.name.clone(),
            imgs,
            on_event,
        ))
        .await?;

        crate::log::info(format!(
            "beat {beat_id}: workflow '{}' step '{}' done (model={}, cost=${:.4})",
            workflow.name, step.name, result.model, result.cost_usd
        ));
        total_cost += result.cost_usd;
        steps.push(WorkflowStepResult {
            name: step.name.clone(),
            answer: result.answer.clone(),
            model: result.model.clone(),
            cost_usd: result.cost_usd,
        });
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
        assert_eq!(wf.step_model(&wf.steps[1]).unwrap(), "OpenRouter - openai/gpt-4o");
        assert_eq!(
            wf.default_model().unwrap(),
            "OpenRouter - anthropic/claude-3.5-sonnet"
        );
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
        assert_eq!(wf.step_model(&wf.steps[1]).unwrap(), "OpenRouter - openai/gpt-4o");
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
