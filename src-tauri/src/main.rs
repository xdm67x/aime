// App/window assembly. Window title, size and frontendDist are declarative in
// tauri.conf.json; this file only wires the command handlers.

mod config;
mod db;
mod openrouter;

fn main() {
    openrouter::spawn_refresh_loop();
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            config::save_api_key,
            config::get_api_key,
            openrouter::list_models,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
