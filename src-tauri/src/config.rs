use crate::db;

const OPENROUTER_API_KEY: &str = "openrouter_api_key";

pub fn openrouter_key() -> Result<Option<String>, String> {
    db::get_setting(OPENROUTER_API_KEY)
}

#[tauri::command]
pub fn save_api_key(key: String) -> Result<(), String> {
    db::set_setting(OPENROUTER_API_KEY, key.trim())
}

#[tauri::command]
pub fn get_api_key() -> Result<Option<String>, String> {
    openrouter_key()
}
