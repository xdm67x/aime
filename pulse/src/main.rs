//! Pulse — headless workflow runner. A workflow is a yaml file (created with
//! `pulse workflow new <title>` in the current directory); `pulse <workflow>`
//! executes its steps through the agentic tool loop until each step's goal
//! is reached, writing a markdown report of everything that happened into
//! the current directory while the terminal shows which step is running.
//! The provider (url + api key) is set with `pulse provider use`, and its
//! models are listed with `pulse models`.

mod cli;
mod run;

use clap::Parser;

#[tokio::main]
async fn main() {
    let cli = cli::Cli::parse();
    pulse_core::log::info(format!(
        "pulse {} (CLI): {}",
        env!("CARGO_PKG_VERSION"),
        cli.command.label()
    ));
    match cli::run(cli.command).await {
        Ok(code) if code != 0 => std::process::exit(code),
        Ok(_) => {}
        Err(e) => {
            pulse_core::log::error(format!("cli command failed: {e}"));
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    }
}
