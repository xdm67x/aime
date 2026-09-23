//! CLI subcommands: workflow management and release updates. These run before
//! any terminal setup and exit — the TUI only launches when no subcommand is
//! given. Settings (API keys, model slots) live in the TUI via /key, /keys,
//! /model and /models.

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "pulse", about = "Pulse — AI agent harness with a terminal UI")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Manage workflows (run them in the chat with /workflow <name>)
    Workflow {
        #[command(subcommand)]
        action: WorkflowAction,
    },
    /// Check for a newer release; --apply installs it
    Update {
        /// Download the latest release and replace the running binary
        #[arg(long)]
        apply: bool,
    },
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
            Command::Workflow { .. } => "workflow",
            Command::Update { .. } => "update",
        }
    }
}

pub async fn run(command: Command) -> Result<(), String> {
    match command {
        Command::Workflow { action } => run_workflow(action),
        Command::Update { apply } => run_update(apply).await,
    }
}

async fn run_update(apply: bool) -> Result<(), String> {
    let current = env!("CARGO_PKG_VERSION");
    if !apply {
        let release = pulse_core::update::check(current)
            .await
            .map_err(|e| format!("Update check failed: {e}"))?
            .ok_or_else(|| format!("pulse {current} is up to date"))?;
        println!("New release: {} (installed: v{current})", release.tag);
        println!("Apply with: pulse update --apply");
        return Ok(());
    }
    let tag = async {
        let release = pulse_core::update::check(current)
            .await?
            .ok_or_else(|| format!("pulse {current} is already up to date"))?;
        println!("Downloading {}…", release.tag);
        pulse_core::update::apply(&release).await
    }
    .await
    .map_err(|e| format!("Update failed: {e}"))?;
    println!("Updated to {tag} — restart pulse to run the new version.");
    Ok(())
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
