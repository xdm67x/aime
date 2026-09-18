use crate::{db, openrouter};
use rusqlite::{params, OptionalExtension, Row};
use serde::Serialize;

#[derive(Serialize)]
pub struct Beat {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub archived: bool,
    pub created_at: String,
    pub cost_usd: f64,
}

const SELECT: &str = "SELECT b.id, b.name, b.description, b.archived, b.created_at, \
    COALESCE((SELECT SUM(cost_usd) FROM beat_usage WHERE beat_id = b.id), 0.0) FROM beats b";

fn row_to_beat(row: &Row) -> rusqlite::Result<Beat> {
    Ok(Beat {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        archived: row.get::<_, i64>(3)? != 0,
        created_at: row.get(4)?,
        cost_usd: row.get(5)?,
    })
}

#[tauri::command]
pub fn list_beats() -> Result<Vec<Beat>, String> {
    let conn = db::open()?;
    let mut stmt = conn
        .prepare(&format!("{SELECT} ORDER BY b.archived, b.id DESC"))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], row_to_beat)
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn create_beat(name: String, description: String) -> Result<Beat, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Beat name cannot be empty".into());
    }
    let description = description.trim();
    let conn = db::open()?;
    conn.execute(
        "INSERT INTO beats (name, description) VALUES (?1, ?2)",
        params![name, description],
    )
    .map_err(|e| e.to_string())?;
    let id = conn.last_insert_rowid();
    let created_at: String = conn
        .query_row("SELECT created_at FROM beats WHERE id = ?1", params![id], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    Ok(Beat {
        id,
        name: name.into(),
        description: description.into(),
        archived: false,
        created_at,
        cost_usd: 0.0,
    })
}

#[tauri::command]
pub fn set_beat_archived(id: i64, archived: bool) -> Result<(), String> {
    db::open()?
        .execute(
            "UPDATE beats SET archived = ?1 WHERE id = ?2",
            params![archived as i64, id],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// All persisted messages of a beat, oldest first.
#[tauri::command]
pub fn get_beat_messages(id: i64) -> Result<Vec<serde_json::Value>, String> {
    let conn = db::open()?;
    let current: String = conn
        .query_row("SELECT messages FROM beats WHERE id = ?1", params![id], |r| r.get(0))
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or("Beat not found")?;
    serde_json::from_str(&current).map_err(|e| e.to_string())
}

/// Permanently delete an archived beat and its usage rows.
#[tauri::command]
pub fn delete_beat(id: i64) -> Result<(), String> {
    let conn = db::open()?;
    conn.execute("DELETE FROM beat_usage WHERE beat_id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    let n = conn
        .execute("DELETE FROM beats WHERE id = ?1 AND archived = 1", params![id])
        .map_err(|e| e.to_string())?;
    if n == 0 {
        return Err("Beat not found or not archived".into());
    }
    Ok(())
}

async fn usage_cost(model_id: &str, prompt: i64, completion: i64) -> f64 {
    let models = openrouter::list_models().await.unwrap_or_default();
    models
        .iter()
        .find(|m| m.id == model_id)
        .map_or(0.0, |m| {
            let p: f64 = m.pricing.prompt.parse().unwrap_or(0.0);
            let c: f64 = m.pricing.completion.parse().unwrap_or(0.0);
            p * prompt as f64 + c * completion as f64
        })
}

/// Record one model call against a beat; cost is computed from the cached
/// OpenRouter pricing at call time. Returns the recorded cost in USD.
#[tauri::command]
pub async fn record_beat_usage(
    beat_id: i64,
    model: String,
    prompt_tokens: i64,
    completion_tokens: i64,
) -> Result<f64, String> {
    let cost = usage_cost(&model, prompt_tokens, completion_tokens).await;
    db::open()?
        .execute(
            "INSERT INTO beat_usage (beat_id, model, prompt_tokens, completion_tokens, cost_usd) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![beat_id, model, prompt_tokens, completion_tokens, cost],
        )
        .map_err(|e| e.to_string())?;
    Ok(cost)
}
