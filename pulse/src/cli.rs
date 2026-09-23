//! CLI subcommands: settings and workflow management. These run before any
//! terminal setup and exit — the TUI only launches when no subcommand is given.

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "pulse", about = "Pulse — AI agent harness with a terminal UI")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Manage settings: API keys and the four model slots
    Settings {
        #[command(subcommand)]
        action: SettingsAction,
    },
    /// Manage workflows (run them in the chat with /workflow <name>)
    Workflow {
        #[command(subcommand)]
        action: WorkflowAction,
    },
}

#[derive(Subcommand)]
pub enum SettingsAction {
    /// List providers and model slots
    List,
    /// Set a field: openrouter-key, opencode-key, litellm-key,
    /// litellm-base-url, classifier, high, base, low
    Set { field: String, value: String },
}

#[derive(Subcommand)]
pub enum WorkflowAction {
    /// List discovered workflows
    List,
    /// Create a workflow from a template and open it in $EDITOR
    New { name: String },
    /// Open an existing workflow in $EDITOR
    Edit { name: String },
}

impl Command {
    /// Short name for logging: which subcommand ran, without arguments.
    pub fn label(&self) -> &'static str {
        match self {
            Command::Settings { .. } => "settings",
            Command::Workflow { .. } => "workflow",
        }
    }
}

pub fn run(command: Command) -> Result<(), String> {
    match command {
        Command::Settings { action } => run_settings(action),
        Command::Workflow { action } => run_workflow(action),
    }
}

fn run_settings(action: SettingsAction) -> Result<(), String> {
    match action {
        SettingsAction::List => {
            for provider in ["openrouter", "opencode", "litellm"] {
                let key = pulse_core::config::get_api_key(provider)
                    .map_err(|e| format!("Failed to read settings: {e}"))?;
                let display = match key {
                    Some(k) if k.len() > 4 => format!("{}…", &k[..4]),
                    Some(_) => "****".to_string(),
                    None => "(not set)".to_string(),
                };
                println!("{provider}-key: {display}");
            }
            let litellm_base_url = pulse_core::config::get_base_url("litellm")
                .map_err(|e| format!("Failed to read settings: {e}"))?;
            match litellm_base_url {
                Some(url) if !url.is_empty() => println!("litellm-base-url: {url}"),
                _ => println!("litellm-base-url: (not set)"),
            }
            let config = pulse_core::config::ModelConfig::load()
                .map_err(|e| format!("Failed to read settings: {e}"))?;
            println!("classifier: {}", or_unset(&config.classifier));
            println!("high:       {}", or_unset(&config.high));
            println!("base:       {}", or_unset(&config.base));
            println!("low:        {}", or_unset(&config.low));
            Ok(())
        }
        SettingsAction::Set { field, value } => {
            match field.as_str() {
                "openrouter-key" => pulse_core::config::save_api_key("openrouter", &value),
                "opencode-key" => pulse_core::config::save_api_key("opencode", &value),
                "litellm-key" => pulse_core::config::save_api_key("litellm", &value),
                "litellm-base-url" => pulse_core::config::save_base_url("litellm", &value),
                "classifier" | "high" | "base" | "low" => {
                    let mut config = pulse_core::config::ModelConfig::load()
                        .map_err(|e| format!("Failed to read settings: {e}"))?;
                    match field.as_str() {
                        "classifier" => config.classifier = value,
                        "high" => config.high = value,
                        "base" => config.base = value,
                        "low" => config.low = value,
                        _ => unreachable!(),
                    }
                    pulse_core::config::save_model_config(&config)
                }
                _ => Err(format!(
                    "Unknown field: {field}\nValid fields: openrouter-key, opencode-key, \
                     litellm-key, litellm-base-url, classifier, high, base, low"
                )),
            }?;
            pulse_core::log::info(format!("settings set: {field}"));
            println!("Saved {field}");
            Ok(())
        }
    }
}

fn or_unset(value: &str) -> &str {
    if value.is_empty() {
        "(not set)"
    } else {
        value
    }
}

fn run_workflow(action: WorkflowAction) -> Result<(), String> {
    match action {
        WorkflowAction::List => {
            let workflows = pulse_core::workflows::discover()
                .map_err(|e| format!("Failed to discover workflows: {e}"))?;
            if workflows.is_empty() {
                println!("No workflows found in ~/.pulse/workflows");
                println!("Create one with: pulse workflow new <name>");
                return Ok(());
            }
            for wf in workflows {
                println!("{} — {}", wf.name, wf.description);
            }
            Ok(())
        }
        WorkflowAction::New { name } => {
            if name.trim().is_empty() {
                return Err("Workflow name cannot be empty".into());
            }
            let path = workflow_path(&name);
            if std::path::Path::new(&path).is_file() {
                return Err(format!("Workflow already exists: {path}"));
            }
            let template = format!(
                "name: {name}\n\
                 description: A new workflow\n\
                 steps:\n\
                 \x20 - name: step1\n\
                 \x20   prompt: |\n\
                 \x20     Do something useful.\n"
            );
            std::fs::write(&path, template).map_err(|e| format!("Failed to write {path}: {e}"))?;
            pulse_core::log::info(format!("workflow created: {path}"));
            open_editor(&path);
            println!("Created {path}");
            Ok(())
        }
        WorkflowAction::Edit { name } => {
            let path = find_workflow_file(&name)
                .ok_or_else(|| format!("Workflow not found: ~/.pulse/workflows/{name}.yml"))?;
            open_editor(&path);
            Ok(())
        }
    }
}

fn workflow_path(name: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let dir = format!("{home}/.pulse/workflows");
    let _ = std::fs::create_dir_all(&dir);
    format!("{dir}/{name}.yml")
}

fn find_workflow_file(name: &str) -> Option<String> {
    let home = std::env::var("HOME").unwrap_or_default();
    for ext in ["yml", "yaml"] {
        let path = format!("{home}/.pulse/workflows/{name}.{ext}");
        if std::path::Path::new(&path).is_file() {
            return Some(path);
        }
    }
    None
}

fn open_editor(path: &str) {
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vim".into());
    let _ = std::process::Command::new(&editor).arg(path).status();
}
