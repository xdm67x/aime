//! Pulse core: the agent harness — provider dispatch, tiered model routing,
//! agentic tool loop, beats/projects persistence — with no UI dependencies.
//!
//! Consumed today by the Tauri app (`src-tauri`); designed to also back a
//! future CLI. Live progress flows through a caller-supplied [`harness::OnEvent`]
//! callback instead of a UI handle, so any runtime can drive it.

pub mod beats;
pub mod config;
pub mod db;
pub mod diff;
pub mod harness;
pub mod projects;
pub mod prompts;
pub mod providers;
pub mod skills;
pub mod tools;
pub mod workflows;
