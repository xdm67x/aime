//! Workflows: YAML-defined multi-step prompts stored in `~/.pulse/workflows/`.
//!
//! Each workflow defines a sequence of steps. A step can specify a model
//! directly (bypassing the classifier) or fall back to the workflow-level
//! default model, or to normal classifier routing when neither is set.
//! Invoked from the harness via `/workflow {name}`.

use crate::harness::{self, OnEvent, TaskResult};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub struct Workflow {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Optional default model for all steps. Overridden by a step-level model.
    #[serde(default)]
    pub model: Option<String>,
    pub steps: Vec<WorkflowStep>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct WorkflowStep {
    pub name: String,
    pub prompt: String,
    /// Per-step model override. `None` → workflow default → classifier routing.
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
    /// Resolved model id, or "classifier" when normal routing was used.
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

/// Execute a workflow against an existing beat. Each step runs as a separate
/// `run_task` or `run_task_with_model` call on the same `beat_id`, so prior
/// context accumulates. The `on_event` callback receives live `TaggedEvent`s.
pub async fn run(
    beat_id: i64,
    workflow: &Workflow,
    on_event: OnEvent<'_>,
) -> Result<WorkflowRunResult, String> {
    let mut steps = Vec::with_capacity(workflow.steps.len());
    let mut total_cost = 0.0;
    crate::log::info(format!(
        "beat {beat_id}: workflow '{}' starting ({} steps)",
        workflow.name,
        workflow.steps.len()
    ));

    for step in &workflow.steps {
        let model = step.model.as_deref().or(workflow.model.as_deref());
        let result: TaskResult = match model {
            Some(m) => {
                Box::pin(harness::run_task_with_model(
                    beat_id,
                    step.prompt.clone(),
                    m.to_string(),
                    vec![],
                    on_event,
                ))
                .await?
            }
            None => {
                Box::pin(harness::run_task(
                    beat_id,
                    step.prompt.clone(),
                    vec![],
                    on_event,
                ))
                .await?
            }
        };

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
}
