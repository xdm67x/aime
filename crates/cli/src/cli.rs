//! CLI subcommands. Aime is headless: workflows are yaml files (created in
//! ./.aime/workflows with `aime create`, or globally in
//! ~/.aime/workflows with `aime create --global`), run with `aime run
//! <workflow>` — or bare `aime run`, which lists the workflows found in
//! the current directory, ./.aime/workflows and ~/.aime/workflows and
//! opens a picker — and with `aime <workflow>`. The provider is configured with
//! `aime provider use <url|litellm|mistral|opencode|openrouter> <key>`
//! (shown with `aime provider`), `aime models` lists what the provider
//! offers, and `aime update` installs a newer release when one exists. Any
//! unknown subcommand is treated as a workflow name to run — `aime research
//! do X` runs the `research` workflow with "do X" as the message.

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "aime",
    version,
    about = "Run YAML workflows through an AI agent until each step's goal is reached.\n\nRuns work in a fresh git worktree by default (--no-worktree to disable).\nRun a workflow with: aime run (interactive picker), aime run <workflow>, or aime <workflow>",
    arg_required_else_help = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Create a blank workflow template: in ./.aime/workflows, or in
    /// ~/.aime/workflows with --global
    Create {
        /// Workflow title — used as the file name (`<slug>.yml`)
        title: String,
        /// Save the workflow to ~/.aime/workflows instead of
        /// ./.aime/workflows
        #[arg(long)]
        global: bool,
    },
    /// Open a workflow in $EDITOR
    Edit {
        /// Workflow name (or file path)
        name: String,
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
    /// Run a workflow. With a name or file path it runs directly; bare
    /// `aime run` lists the workflows found in the current directory,
    /// ./.aime/workflows and ~/.aime/workflows and opens a picker.
    /// Runs in a fresh git worktree under ~/.aime/worktrees by default;
    /// --no-worktree runs in the current directory instead
    Run {
        /// Workflow name or file path — omit to pick interactively
        name: Option<String>,
        /// Run in the current directory, not a git worktree
        #[arg(long)]
        no_worktree: bool,
    },
    /// Run a workflow by name or file path — `aime <workflow>`. Any other
    /// unknown subcommand lands here too
    #[command(external_subcommand)]
    External(Vec<String>),
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
            Command::Create { .. } => "create",
            Command::Edit { .. } => "edit",
            Command::Provider { .. } => "provider",
            Command::Models => "models",
            Command::Version => "version",
            Command::Update => "update",
            Command::Run { .. } => "run",
            Command::External(_) => "run",
        }
    }
}

/// The parsed `aime <workflow>` invocation.
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
                 name: aime <workflow> [--no-worktree]"
            ));
        }
    }
    let workflow = workflow.ok_or("No workflow given — run one with: aime <workflow>")?;
    Ok(RunArgs {
        workflow,
        no_worktree,
    })
}

/// Run a command; the Ok value is the process exit code.
pub async fn run(command: Command) -> Result<i32, String> {
    match command {
        Command::Create { title, global } => run_create(&title, global),
        Command::Edit { name } => run_edit(&name),
        Command::Provider { action } => run_provider(action).await,
        Command::Models => run_models().await,
        Command::Version => {
            println!("aime {}", env!("CARGO_PKG_VERSION"));
            Ok(0)
        }
        Command::Update => run_update().await,
        Command::Run {
            name: Some(w),
            no_worktree,
        } => Ok(crate::run::run_workflow(&w, !no_worktree).await),
        Command::Run {
            name: None,
            no_worktree,
        } => run_pick(!no_worktree).await,
        Command::External(args) => {
            let a = split_run_args(args)?;
            Ok(crate::run::run_workflow(&a.workflow, !a.no_worktree).await)
        }
    }
}

/// Bare `aime run`: list the discovered workflows and open the picker;
/// the selection then runs like any named workflow.
async fn run_pick(use_worktree: bool) -> Result<i32, String> {
    let all = crate::picker::discover_all()?;
    if all.is_empty() {
        println!("No workflows found (current directory, ./.aime/workflows + ~/.aime/workflows)");
        println!("Create one with: aime create <title> [--global]");
        return Ok(0);
    }
    match crate::picker::pick(&all)? {
        crate::picker::Pick::Selected(path) => {
            Ok(crate::run::run_workflow(&path.to_string_lossy(), use_worktree).await)
        }
        crate::picker::Pick::Cancelled(code) => Ok(code),
    }
}

/* ---- workflows ---- */

fn run_create(title: &str, global: bool) -> Result<i32, String> {
    let name = crate::run::slug(title);
    if name.is_empty() {
        return Err(format!("'{title}' is not a usable workflow name"));
    }
    let (dir, dir_label) = if global {
        (aime_core::workflows::dir()?, "~/.aime/workflows")
    } else {
        (aime_core::workflows::local_dir()?, "./.aime/workflows")
    };
    let path = dir.join(format!("{name}.yml"));
    if path.is_file() {
        return Err(format!(
            "Workflow already exists: {} ({dir_label})",
            path.display()
        ));
    }
    std::fs::write(&path, aime_core::workflows::template(&name))
        .map_err(|e| format!("Failed to write {}: {e}", path.display()))?;
    aime_core::log::info(format!("workflow created: {}", path.display()));
    println!("Created {} in {dir_label}", path.display());
    println!("Set a `model:` in it (list ids with: aime models), then run: aime {name}");
    Ok(0)
}

fn run_edit(name: &str) -> Result<i32, String> {
    let (_, path) = aime_core::workflows::find(name)?;
    open_editor(&path.to_string_lossy());
    Ok(0)
}

/* ---- provider ---- */

async fn run_provider(action: Option<ProviderAction>) -> Result<i32, String> {
    // `aime provider` with no arguments: show what is configured.
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
            "Set one with: aime provider use \
             <url|litellm|mistral|opencode|openrouter> <api_key>"
        );
        return Ok(0);
    };
    let name = match aime_core::config::parse_provider_target(&url_or_provider)? {
        aime_core::config::ProviderTarget::Known(key) => {
            let p = aime_core::providers::provider_by_key(key)
                .ok_or_else(|| format!("Unknown provider: {key}"))?;
            aime_core::config::save_api_key(key, &api_key)?;
            aime_core::config::save_provider_name(p.name())?;
            println!("Provider saved: {}", p.name());
            p.name().to_string()
        }
        aime_core::config::ProviderTarget::Url(url) => {
            aime_core::config::save_provider(&url, &api_key)?;
            println!("Provider saved: {url}");
            "Custom".into()
        }
    };
    // verify the configuration by asking the endpoint for its models
    match aime_core::providers::list_models_of(&name).await {
        Ok(models) => println!(
            "Provider reachable — {} model(s) available (list them: aime models)",
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
    let models = aime_core::providers::list_models_of(p.name())
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
    let release = aime_core::update::check(current)
        .await
        .map_err(|e| format!("Update check failed: {e}"))?;
    let Some(release) = release else {
        println!("aime {current} is up to date");
        return Ok(0);
    };
    println!("New release: {} (installed: v{current})", release.tag);
    let tag = aime_core::update::apply(&release)
        .await
        .map_err(|e| format!("Update failed: {e}"))?;
    println!("Updated to {tag} — restart aime to run the new version.");
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
        let a = run(&["ship"]).unwrap();
        assert_eq!(a.workflow, "ship");
        assert!(!a.no_worktree);
        let a = run(&["ship", "--no-worktree"]).unwrap();
        assert_eq!(a.workflow, "ship");
        assert!(a.no_worktree);
        let a = run(&["--no-worktree", "./flows/ship.yml"]).unwrap();
        assert_eq!(a.workflow, "./flows/ship.yml");
        assert!(a.no_worktree);
        assert!(run(&["ship", "fix", "it"])
            .unwrap_err()
            .contains("takes only a workflow"));
        assert!(run(&["ship", "--yes"])
            .unwrap_err()
            .contains("Unknown flag: --yes"));
        assert!(run(&["ship", "--"])
            .unwrap_err()
            .contains("Unknown flag: --"));
        assert!(run(&["--no-worktree"]).is_err());
    }
}
