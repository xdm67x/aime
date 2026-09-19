// App/window assembly. Window title, size and frontendDist are declarative in
// tauri.conf.json; this file only wires the command handlers.

mod beats;
mod config;
mod db;
mod harness;
mod projects;
mod prompts;
mod providers;
mod skills;
mod tools;

fn main() {
    providers::spawn_refresh_loop();
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            config::save_api_key,
            config::get_api_key,
            config::get_model_config,
            config::save_model_config,
            providers::list_models,
            harness::run_task,
            beats::list_beats,
            beats::get_beat_messages,
            beats::create_beat,
            beats::set_beat_archived,
            beats::delete_beat,
            beats::record_beat_usage,
            projects::list_projects,
            projects::add_project,
            projects::clone_project,
            projects::remove_project,
            projects::pick_folder,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
