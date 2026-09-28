//! Pulse core: the agent harness — provider dispatch, workflow-driven runs,
//! agentic tool loop, beats/projects persistence — with no UI dependencies.
//!
//! Consumed today by the `pulse` binary; designed to also back other
//! runtimes. Live progress flows through a caller-supplied [`harness::OnEvent`]
//! callback instead of a UI handle, so any runtime can drive it.

pub mod beats;
pub mod config;
pub mod db;
pub mod diff;
pub mod harness;
pub mod log;
pub mod projects;
pub mod prompts;
pub mod providers;
pub mod skills;
pub mod tools;
pub mod update;
pub mod workflows;
