//! Aime — headless workflow runner. A workflow is a yaml file (created with
//! `aime create <title>` in ./.aime/workflows, or globally with
//! `aime create <title> --global`); `aime run <workflow>` — or bare
//! `aime run`, which lists the available workflows and opens a picker —
//! and `aime <workflow>`
//! execute its steps through the agentic tool loop until each step's goal
//! is reached, writing a markdown report of everything that happened into
//! the current directory while the terminal shows which step is running.
//! The provider is set with `aime provider use` (a base URL, or a known
//! provider name + API key) and shown with `aime provider`; its models are
//! listed with `aime models`.

mod cli;
mod picker;
mod run;

use clap::Parser;

#[tokio::main]
async fn main() {
    let cli = cli::Cli::parse();
    aime_core::log::info(format!(
        "aime {} (CLI): {}",
        env!("CARGO_PKG_VERSION"),
        cli.command.label()
    ));
    match cli::run(cli.command).await {
        Ok(code) if code != 0 => std::process::exit(code),
        Ok(_) => {}
        Err(e) => {
            aime_core::log::error(format!("cli command failed: {e}"));
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    }
}
