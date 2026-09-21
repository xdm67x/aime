//! User-defined workflows: an ordered list of steps, each pinned to a specific
//! model and optionally a custom prompt. A workflow replaces the classifier
//! routing for the task it runs — every step is executed in order, and the
//! models come straight from the step definitions, not the tier slots.
//!
//! Workflows live in the `config` table as JSON under `workflows`, and the id
//! of the workflow new beats default to under `workflow_default`.

use crate::db;
use serde::{Deserialize, Serialize};

/// How one step executes its model.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    /// Agentic loop with tools (`task_complete` to finish) — implementation work.
    Agent,
    /// Single streaming completion, no tools — planning, review, writing.
    Ask,
    /// Critique the previous step's output and produce an improved answer.
    Reflexion,
}

impl StepKind {
    pub fn as_str(self) -> &'static str {
        match self {
            StepKind::Agent => "agent",
            StepKind::Ask => "ask",
            StepKind::Reflexion => "reflexion",
        }
    }
    pub fn parse(s: &str) -> Option<StepKind> {
        match s.trim().to_ascii_lowercase().as_str() {
            "agent" => Some(StepKind::Agent),
            "ask" => Some(StepKind::Ask),
            "reflexion" => Some(StepKind::Reflexion),
            _ => None,
        }
    }
}

/// One brick of a workflow: what to do, which model does it, and an optional
/// custom prompt that replaces the built-in instruction for this step.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkflowStep {
    pub kind: StepKind,
    /// OpenRouter model id — mandatory, every brick pins its own model.
    pub model: String,
    /// User-defined instruction for this step. Empty → built-in default.
    #[serde(default)]
    pub prompt: String,
    /// Short label shown in the UI (e.g. "Plan", "Review").
    #[serde(default)]
    pub label: String,
}

/// A named, ordered workflow the user composes in Settings.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Workflow {
    /// Stable identifier (slug of the name at creation time, unique).
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub steps: Vec<WorkflowStep>,
}

const WORKFLOWS_KEY: &str = "workflows";
const WORKFLOW_DEFAULT_KEY: &str = "workflow_default";

fn default_workflows() -> Vec<Workflow> {
    // The pipeline the harness shipped with, expressed as a workflow. Models
    // stay empty: the user pins them per brick in Settings.
    vec![Workflow {
        id: "classify-implement-review".into(),
        name: "Classify → implement → review".into(),
        description: "Legacy classifier routing: route by difficulty, then \
                      implement, then review the result."
            .into(),
        steps: vec![
            WorkflowStep {
                kind: StepKind::Agent,
                model: String::new(),
                prompt: String::new(),
                label: "Route".into(),
            },
            WorkflowStep {
                kind: StepKind::Reflexion,
                model: String::new(),
                prompt: String::new(),
                label: "Review".into(),
            },
        ],
    }]
}

/// Load every saved workflow. The seeded default (an empty-model copy of the
/// classic routing) is returned on first run, before anything is saved.
pub fn list() -> Result<Vec<Workflow>, String> {
    let raw = db::get_setting(WORKFLOWS_KEY)?;
    let Some(raw) = raw else {
        return Ok(default_workflows());
    };
    let workflows: Vec<Workflow> =
        serde_json::from_str(&raw).map_err(|e| format!("Corrupted workflow settings: {e}"))?;
    Ok(workflows)
}

/// Load one workflow by id.
pub fn get(id: &str) -> Result<Option<Workflow>, String> {
    Ok(list()?.into_iter().find(|w| w.id == id))
}

/// Check a workflow list before saving: non-empty unique ids, named, with
/// at least one step, and every step pinned to a model.
pub fn validate(workflows: &[Workflow]) -> Result<(), String> {
    let mut seen = std::collections::HashSet::new();
    for w in workflows {
        if w.id.trim().is_empty() {
            return Err("Workflow ids must not be empty.".into());
        }
        if w.name.trim().is_empty() {
            return Err(format!("Workflow “{}” has no name.", w.id));
        }
        if !seen.insert(w.id.trim().to_string()) {
            return Err(format!("Duplicate workflow id: {}.", w.id));
        }
        if w.steps.is_empty() {
            return Err(format!("Workflow “{}” has no steps.", w.id));
        }
        for (i, s) in w.steps.iter().enumerate() {
            if s.model.trim().is_empty() {
                return Err(format!(
                    "Step {} of workflow “{}” has no model.",
                    i + 1,
                    w.id
                ));
            }
        }
    }
    Ok(())
}

/// Overwrite the saved workflow list. Validates before saving.
pub fn save_all(workflows: &[Workflow]) -> Result<(), String> {
    validate(workflows)?;
    db::set_setting(
        WORKFLOWS_KEY,
        &serde_json::to_string(workflows).map_err(|e| e.to_string())?,
    )
}

/// Id of the workflow new sessions run by default. Empty → classifier
/// routing (no workflow).
pub fn default_id() -> Result<String, String> {
    Ok(db::get_setting(WORKFLOW_DEFAULT_KEY)?.unwrap_or_default())
}

/// Set the default workflow id. An empty string restores classifier routing.
pub fn set_default_id(id: &str) -> Result<(), String> {
    db::set_setting(WORKFLOW_DEFAULT_KEY, id.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(kind: StepKind, model: &str) -> WorkflowStep {
        WorkflowStep {
            kind,
            model: model.into(),
            prompt: String::new(),
            label: String::new(),
        }
    }

    #[test]
    fn test_step_kind_roundtrip() {
        for k in [StepKind::Agent, StepKind::Ask, StepKind::Reflexion] {
            assert_eq!(StepKind::parse(k.as_str()), Some(k));
        }
        assert_eq!(StepKind::parse("AGENT"), Some(StepKind::Agent));
        assert_eq!(StepKind::parse(" nope "), None);
    }

    #[test]
    fn test_workflow_json_roundtrip() {
        let w = Workflow {
            id: "wf".into(),
            name: "Test".into(),
            description: String::new(),
            steps: vec![step(StepKind::Agent, "m1"), step(StepKind::Ask, "m2")],
        };
        let json = serde_json::to_string(&w).unwrap();
        assert!(json.contains("\"kind\":\"agent\""));
        let back: Workflow = serde_json::from_str(&json).unwrap();
        assert_eq!(back.steps.len(), 2);
        assert_eq!(back.steps[0].kind, StepKind::Agent);
        assert_eq!(back.steps[1].model, "m2");
        // missing optional fields default
        let parsed: Workflow =
            serde_json::from_str(r#"{"id":"w","name":"n","steps":[{"kind":"ask","model":"m"}]}"#)
                .unwrap();
        assert_eq!(parsed.steps[0].prompt, "");
        assert_eq!(parsed.steps[0].label, "");
    }

    #[test]
    fn test_save_all_validation() {
        let ok = vec![Workflow {
            id: "a".into(),
            name: "A".into(),
            description: String::new(),
            steps: vec![step(StepKind::Agent, "m")],
        }];
        assert!(validate(&ok).is_ok());

        let dupe = vec![
            Workflow {
                id: "a".into(),
                name: "A".into(),
                description: String::new(),
                steps: vec![step(StepKind::Agent, "m")],
            },
            Workflow {
                id: "a".into(),
                name: "B".into(),
                description: String::new(),
                steps: vec![step(StepKind::Agent, "m")],
            },
        ];
        let err = validate(&dupe).unwrap_err();
        assert!(err.contains("Duplicate workflow id"));

        let empty_model = vec![Workflow {
            id: "c".into(),
            name: "C".into(),
            description: String::new(),
            steps: vec![step(StepKind::Agent, " ")],
        }];
        let err = validate(&empty_model).unwrap_err();
        assert!(err.contains("has no model"));

        let no_steps = vec![Workflow {
            id: "d".into(),
            name: "D".into(),
            description: String::new(),
            steps: vec![],
        }];
        let err = validate(&no_steps).unwrap_err();
        assert!(err.contains("has no steps"));
    }
}
