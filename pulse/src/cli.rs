//! CLI subcommands. Pulse is headless: workflows are yaml files (created in
//! ./.pulse/workflows with `pulse workflow new`, or globally in
//! ~/.pulse/workflows with `pulse workflow new --global`, run with
//! `pulse <workflow>`), the provider is configured with
//! `pulse provider use <url|litellm|mistral|opencode|openrouter> <key>`
//! (shown with `pulse provider`), `pulse models` lists what the provider
//! offers, and `pulse update` installs a newer release when one exists. Any
//! unknown subcommand is treated as a workflow name to run — `pulse research
//! do X` runs the `research` workflow with "do X" as the message.

use clap::{Parser, Subcommand};
use std::path::Path;

#[derive(Parser)]
#[command(
    name = "pulse",
    version,
    about = "Run YAML workflows through an AI agent until each step's goal is reached.\n\nRuns work in a fresh git worktree by default (--no-worktree to disable).\nRun a workflow with: pulse <workflow>",
    arg_required_else_help = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Manage workflow yaml files
    Workflow {
        #[command(subcommand)]
        action: WorkflowAction,
    },
    /// Show the configured provider, or configure one (`provider use`)
    Provider {
        #[command(subcommand)]
        action: Option<ProviderAction>,
    },
    /// List the models offered by the configured provider
    Models,
    /// Print the current version
    Version,
    /// Install the latest release when it is newer than this binary
    Update,
    /// Run a workflow by name or file path — no other arguments. Runs in a
    /// fresh git worktree under ~/.pulse/worktrees by default; --no-worktree
    /// runs in the current directory instead. Any unknown subcommand lands
    /// here: `pulse <workflow>`
    #[command(external_subcommand)]
    Run(Vec<String>),
}

#[derive(Subcommand)]
pub enum WorkflowAction {
    /// Create a blank workflow template: in ./.pulse/workflows, or in
    /// ~/.pulse/workflows with --global
    New {
        /// Workflow title — used as the file name (`<slug>.yml`)
        title: String,
        /// Save the workflow to ~/.pulse/workflows instead of
        /// ./.pulse/workflows
        #[arg(long)]
        global: bool,
    },
    /// List workflows in the current directory, ./.pulse/workflows and
    /// ~/.pulse/workflows
    List,
    /// Open a workflow in $EDITOR
    Edit { name: String },
}

#[derive(Subcommand)]
pub enum ProviderAction {
    /// Set the provider used by every run: an OpenAI-compatible base URL,
    /// or a known provider name (litellm, mistral, opencode, openrouter)
    /// whose host is built in — then only the API key is needed
    Use {
        /// Base URL of any OpenAI-compatible endpoint (e.g.
        /// https://api.openai.com/v1); `/chat/completions` and `/models` are
        /// appended — or a known provider name: litellm, mistral, opencode,
        /// openrouter
        url_or_provider: String,
        /// API key — pass "" when the endpoint needs no auth
        api_key: String,
    },
}

impl Command {
    /// Short name for logging: which subcommand ran, without arguments.
    pub fn label(&self) -> &'static str {
        match self {
            Command::Workflow { .. } => "workflow",
            Command::Provider { .. } => "provider",
            Command::Models => "models",
            Command::Version => "version",
            Command::Update => "update",
            Command::Run(_) => "run",
        }
    }
}

/// The parsed `pulse <workflow>` invocation.
#[derive(Debug)]
pub struct RunArgs {
    pub workflow: String,
    /// `--no-worktree`: run in the current directory, not a git worktree.
    pub no_worktree: bool,
}

/// Split the raw external-subcommand args: the single bare word is the
/// workflow name/path, `--no-worktree` is a flag, and anything else — extra
/// words or unknown flags — is an error: the run command takes only a
/// workflow name.
pub fn split_run_args(args: Vec<String>) -> Result<RunArgs, String> {
    let mut workflow: Option<String> = None;
    let mut no_worktree = false;
    for a in args {
        if a == "--no-worktree" {
            no_worktree = true;
        } else if a.starts_with('-') && a != "-" {
            return Err(format!(
                "Unknown flag: {a} — the run command only takes --no-worktree"
            ));
        } else if workflow.is_none() {
            workflow = Some(a);
        } else {
            return Err(format!(
                "Unexpected argument: '{a}' — the run command takes only a workflow \
                 name: pulse <workflow> [--no-worktree]"
            ));
        }
    }
    let workflow = workflow.ok_or("No workflow given — run one with: pulse <workflow>")?;
    Ok(RunArgs {
        workflow,
        no_worktree,
    })
}

/// Run a command; the Ok value is the process exit code.
pub async fn run(command: Command) -> Result<i32, String> {
    match command {
        Command::Workflow { action } => run_workflow_cmd(action),
        Command::Provider { action } => run_provider(action).await,
        Command::Models => run_models().await,
        Command::Version => {
            println!("pulse {}", env!("CARGO_PKG_VERSION"));
            Ok(0)
        }
        Command::Update => run_update().await,
        Command::Run(args) => {
            let a = split_run_args(args)?;
            Ok(crate::run::run_workflow(&a.workflow, !a.no_worktree).await)
        }
    }
}

/* ---- workflow ---- */

fn run_workflow_cmd(action: WorkflowAction) -> Result<i32, String> {
    match action {
        WorkflowAction::New { title, global } => {
            let name = crate::run::slug(&title);
            if name.is_empty() {
                return Err(format!("'{title}' is not a usable workflow name"));
            }
            let (dir, dir_label) = if global {
                (pulse_core::workflows::dir()?, "~/.pulse/workflows")
            } else {
                (pulse_core::workflows::local_dir()?, "./.pulse/workflows")
            };
            let path = dir.join(format!("{name}.yml"));
            if path.is_file() {
                return Err(format!(
                    "Workflow already exists: {} ({dir_label})",
                    path.display()
                ));
            }
            std::fs::write(&path, pulse_core::workflows::template(&name))
                .map_err(|e| format!("Failed to write {}: {e}", path.display()))?;
            pulse_core::log::info(format!("workflow created: {}", path.display()));
            println!("Created {} in {dir_label}", path.display());
            println!("Set a `model:` in it (list ids with: pulse models), then run: pulse {name}");
            Ok(0)
        }
        WorkflowAction::List => {
            let mut found = pulse_core::workflows::discover_dir(Path::new("."));
            found.extend(pulse_core::workflows::discover_dir(Path::new(
                "./.pulse/workflows",
            )));
            found.extend(pulse_core::workflows::discover_dir(
                &pulse_core::workflows::dir()?,
            ));
            if found.is_empty() {
                println!("No workflows found (current directory, ./.pulse/workflows + ~/.pulse/workflows)");
                println!("Create one with: pulse workflow new <title> [--global]");
                return Ok(0);
            }
            let width = found
                .iter()
                .map(|(wf, _)| wf.name.len())
                .max()
                .unwrap_or(8)
                .max(8);
            for (wf, path) in found {
                let desc = if wf.description.is_empty() {
                    "-"
                } else {
                    &wf.description
                };
                println!("{:<width$}  {desc}  ({})", wf.name, path.display());
            }
            Ok(0)
        }
        WorkflowAction::Edit { name } => {
            let (_, path) = pulse_core::workflows::find(&name)?;
            open_editor(&path.to_string_lossy());
            Ok(0)
        }
    }
}

/* ---- provider ---- */

async fn run_provider(action: Option<ProviderAction>) -> Result<i32, String> {
    // `pulse provider` with no arguments: show what is configured.
    let Some(ProviderAction::Use {
        url_or_provider,
        api_key,
    }) = action
    else {
        match crate::run::provider_label() {
            Ok(label) => println!("provider: {label}"),
            Err(_) => println!("no provider"),
        }
        println!(
            "Set one with: pulse provider use \
             <url|litellm|mistral|opencode|openrouter> <api_key>"
        );
        return Ok(0);
    };
    let name = match pulse_core::config::parse_provider_target(&url_or_provider)? {
        pulse_core::config::ProviderTarget::Known(key) => {
            let p = pulse_core::providers::provider_by_key(key)
                .ok_or_else(|| format!("Unknown provider: {key}"))?;
            pulse_core::config::save_api_key(key, &api_key)?;
            pulse_core::config::save_provider_name(p.name())?;
            println!("Provider saved: {}", p.name());
            p.name().to_string()
        }
        pulse_core::config::ProviderTarget::Url(url) => {
            pulse_core::config::save_provider(&url, &api_key)?;
            println!("Provider saved: {url}");
            "Custom".into()
        }
    };
    // verify the configuration by asking the endpoint for its models
    match pulse_core::providers::list_models_of(&name).await {
        Ok(models) => println!(
            "Provider reachable — {} model(s) available (list them: pulse models)",
            models.len()
        ),
        Err(e) => {
            println!("Warning: provider configured, but the check failed: {e}");
        }
    }
    Ok(0)
}

/* ---- models ---- */

async fn run_models() -> Result<i32, String> {
    let p = crate::run::require_provider()?;
    let label = crate::run::provider_label()?;
    let models = pulse_core::providers::list_models_of(p.name())
        .await
        .map_err(|e| format!("{e} (provider: {label})"))?;
    if models.is_empty() {
        println!("The provider {label} offers no models.");
        return Ok(0);
    }
    println!("Models offered by {label}:\n");
    let width = models
        .iter()
        .map(|m| crate::run::bare_id(&m.id).len())
        .max()
        .unwrap_or(5)
        .max(5);
    println!(
        "{:<width$}  {:>12}  PRICE (per 1M tokens: in / out)",
        "MODEL", "CONTEXT"
    );
    for m in &models {
        println!(
            "{:<width$}  {:>12}  {}",
            crate::run::bare_id(&m.id),
            m.context_length
                .map(|c| c.to_string())
                .unwrap_or_else(|| "-".into()),
            crate::run::price_per_m(&m.pricing),
        );
    }
    Ok(0)
}

/* ---- update ---- */

async fn run_update() -> Result<i32, String> {
    let current = env!("CARGO_PKG_VERSION");
    let release = pulse_core::update::check(current)
        .await
        .map_err(|e| format!("Update check failed: {e}"))?;
    let Some(release) = release else {
        println!("pulse {current} is up to date");
        return Ok(0);
    };
    println!("New release: {} (installed: v{current})", release.tag);
    let tag = pulse_core::update::apply(&release)
        .await
        .map_err(|e| format!("Update failed: {e}"))?;
    println!("Updated to {tag} — restart pulse to run the new version.");
    Ok(0)
}

fn open_editor(path: &str) {
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vim".into());
    let _ = std::process::Command::new(&editor).arg(path).status();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str]) -> Result<RunArgs, String> {
        split_run_args(args.iter().map(|s| s.to_string()).collect())
    }

    #[test]
    fn test_split_run_args() {
        // bare run
        let a = run(&["ship"]).unwrap();
        assert_eq!(a.workflow, "ship");
        assert!(!a.no_worktree);
        // the flag in any position
        let a = run(&["ship", "--no-worktree"]).unwrap();
        assert_eq!(a.workflow, "ship");
        assert!(a.no_worktree);
        let a = run(&["--no-worktree", "./flows/ship.yml"]).unwrap();
        assert_eq!(a.workflow, "./flows/ship.yml");
        assert!(a.no_worktree);
        // the run command takes only a workflow name
        assert!(run(&["ship", "fix", "it"])
            .unwrap_err()
            .contains("takes only a workflow"));
        assert!(run(&["ship", "--yes"])
            .unwrap_err()
            .contains("Unknown flag: --yes"));
        assert!(run(&["ship", "--"])
            .unwrap_err()
            .contains("Unknown flag: --"));
        // no workflow at all
        assert!(run(&["--no-worktree"]).is_err());
    }
}
