// App/window assembly. Window title, size and frontendDist are declarative in
// tauri.conf.json; this file only wires the command handlers. All logic lives
// in the `pulse-core` crate; each command below is a thin Tauri wrapper around
// it (plus the native folder picker, which is UI-only).

use pulse_core::{beats, config, harness, projects, providers};
use serde_json::Value;
use tauri::Emitter;

#[tauri::command]
fn save_api_key(provider: String, key: String) -> Result<(), String> {
    config::save_api_key(&provider, &key)
}

#[tauri::command]
fn get_api_key(provider: String) -> Result<Option<String>, String> {
    config::api_key(&provider)
}

#[tauri::command]
fn save_base_url(provider: String, url: String) -> Result<(), String> {
    config::save_base_url(&provider, &url)
}

#[tauri::command]
fn get_base_url(provider: String) -> Result<Option<String>, String> {
    config::base_url(&provider)
}

#[tauri::command]
fn get_model_config() -> Result<config::ModelConfig, String> {
    config::ModelConfig::load()
}

#[tauri::command]
fn save_model_config(cfg: config::ModelConfig) -> Result<(), String> {
    config::save_model_config(&cfg)
}

#[tauri::command]
async fn list_models() -> Result<Vec<providers::Model>, String> {
    providers::list_models().await
}

#[tauri::command]
async fn run_task(
    app: tauri::AppHandle,
    beat_id: i64,
    prompt: String,
    images: Option<Vec<String>>,
) -> Result<harness::TaskResult, String> {
    harness::run_task(beat_id, prompt, images.unwrap_or_default(), &mut |ev| {
        let _ = app.emit("task-event", ev);
    })
    .await
}

/// Ask the in-flight task of one beat to stop (the UI's Escape / chip ✕);
/// other sessions keep running. The running harness checks the flag between
/// rounds and inside every stream.
#[tauri::command]
fn cancel_task(beat_id: i64) {
    harness::cancel_current(beat_id)
}

#[tauri::command]
fn list_beats() -> Result<Vec<beats::Beat>, String> {
    beats::list_beats()
}

#[tauri::command]
fn get_beat_messages(id: i64) -> Result<Vec<Value>, String> {
    beats::get_beat_messages(id)
}

#[tauri::command]
fn create_beat(
    name: String,
    description: String,
    project_id: Option<i64>,
) -> Result<beats::Beat, String> {
    beats::create_beat(&name, &description, project_id)
}

#[tauri::command]
fn set_beat_archived(id: i64, archived: bool) -> Result<(), String> {
    beats::set_beat_archived(id, archived)
}

#[tauri::command]
fn delete_beat(id: i64) -> Result<String, String> {
    beats::delete_beat(id)
}

#[tauri::command]
async fn record_beat_usage(
    beat_id: i64,
    model: String,
    prompt_tokens: i64,
    completion_tokens: i64,
) -> Result<f64, String> {
    beats::record_usage(beat_id, &model, prompt_tokens, completion_tokens).await
}

#[tauri::command]
fn beat_usage_totals(beat_id: i64) -> Result<Vec<beats::UsageTotal>, String> {
    beats::usage_totals(beat_id)
}

#[tauri::command]
fn beat_context_full(id: i64) -> Result<bool, String> {
    beats::is_context_full(id)
}

#[tauri::command]
fn list_projects() -> Result<Vec<projects::Project>, String> {
    projects::list_projects()
}

#[tauri::command]
fn add_project(path: String) -> Result<projects::Project, String> {
    projects::add_project(&path)
}

#[tauri::command]
async fn clone_project(repo: String) -> Result<projects::Project, String> {
    projects::clone_project(&repo).await
}

#[tauri::command]
fn remove_project(id: i64) -> Result<(), String> {
    projects::remove_project(id)
}

/// Open a native folder picker; returns the chosen directory, if any.
#[tauri::command]
async fn pick_folder() -> Result<Option<String>, String> {
    Ok(rfd::AsyncFileDialog::new()
        .pick_folder()
        .await
        .map(|f| f.path().to_string_lossy().into_owned()))
}

fn main() {
    // tauri::async_runtime::spawn lazily boots the shared tokio runtime, so
    // this is safe before the builder runs
    tauri::async_runtime::spawn(providers::refresh_loop());
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            save_api_key,
            get_api_key,
            save_base_url,
            get_base_url,
            get_model_config,
            save_model_config,
            list_models,
            run_task,
            cancel_task,
            list_beats,
            get_beat_messages,
            create_beat,
            set_beat_archived,
            delete_beat,
            record_beat_usage,
            beat_usage_totals,
            beat_context_full,
            list_projects,
            add_project,
            clone_project,
            remove_project,
            pick_folder,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
