// App/window assembly. Window title, size and frontendDist are declarative in
// tauri.conf.json; this file only wires the command handlers.

mod beats;
mod config;
mod db;
mod harness;
mod openrouter;

fn main() {
    openrouter::spawn_refresh_loop();
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            config::save_api_key,
            config::get_api_key,
            openrouter::list_models,
            openrouter::send_message,
            harness::run_council,
            beats::list_beats,
            beats::get_beat_messages,
            beats::create_beat,
            beats::set_beat_archived,
            beats::delete_beat,
            beats::record_beat_usage,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
