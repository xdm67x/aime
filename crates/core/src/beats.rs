use crate::{db, projects, providers};
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
    /// Session-wide token totals (all models), for the sidebar listing.
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    /// Project the beat runs in (its directory is the working directory).
    /// Set right after creation (create_beat only): worktree status shown in
    /// the UI, success or failure.
    pub worktree_status: Option<String>,
    pub project_id: Option<i64>,
    pub project_name: Option<String>,
}

const SELECT: &str = "SELECT b.id, b.name, b.description, b.archived, b.created_at, \
    COALESCE((SELECT SUM(cost_usd) FROM beat_usage WHERE beat_id = b.id), 0.0), \
    COALESCE((SELECT SUM(prompt_tokens) FROM beat_usage WHERE beat_id = b.id), 0), \
    COALESCE((SELECT SUM(completion_tokens) FROM beat_usage WHERE beat_id = b.id), 0), \
    b.project_id, p.name \
    FROM beats b LEFT JOIN projects p ON p.id = b.project_id";

fn row_to_beat(row: &Row) -> rusqlite::Result<Beat> {
    Ok(Beat {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        archived: row.get::<_, i64>(3)? != 0,
        created_at: row.get(4)?,
        cost_usd: row.get(5)?,
        prompt_tokens: row.get(6)?,
        completion_tokens: row.get(7)?,
        worktree_status: None,
        project_id: row.get(8)?,
        project_name: row.get(9)?,
    })
}

pub fn list_beats() -> Result<Vec<Beat>, String> {
    let conn = db::open()?;
    let mut stmt = conn
        .prepare(&format!("{SELECT} ORDER BY b.archived, b.id DESC"))
        .map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], row_to_beat).map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

pub fn create_beat(name: &str, description: &str, project_id: Option<i64>) -> Result<Beat, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Beat name cannot be empty".into());
    }
    let description = description.trim();
    let conn = db::open()?;
    conn.execute(
        "INSERT INTO beats (name, description, project_id) VALUES (?1, ?2, ?3)",
        params![name, description, project_id],
    )
    .map_err(|e| e.to_string())?;
    let id = conn.last_insert_rowid();
    let created_at: String = conn
        .query_row(
            "SELECT created_at FROM beats WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let project_name: Option<String> = match project_id {
        Some(pid) => conn
            .query_row(
                "SELECT path FROM projects WHERE id = ?1",
                params![pid],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?,
        None => None,
    };
    // a beat born from a project gets its own git worktree under
    // ~/.pulse/worktrees; status is always reported (UI shows it) and a
    // failure never blocks the beat — it then just runs in the project dir
    let worktree_status = project_name
        .as_deref()
        .map(|path| projects::create_worktree(id, name, path));
    crate::log::info(format!(
        "created beat {id} ({name}){}",
        project_name
            .as_deref()
            .map(|p| format!(", project at {p}"))
            .unwrap_or_default()
    ));
    Ok(Beat {
        id,
        name: name.into(),
        description: description.into(),
        archived: false,
        created_at,
        cost_usd: 0.0,
        prompt_tokens: 0,
        completion_tokens: 0,
        worktree_status,
        project_id,
        project_name,
    })
}

pub fn set_beat_archived(id: i64, archived: bool) -> Result<(), String> {
    db::open()?
        .execute(
            "UPDATE beats SET archived = ?1 WHERE id = ?2",
            params![archived as i64, id],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// True when the beat hit its session context limit and is paused until
/// `/compact` carries it over into a fresh summarized session.
pub fn is_context_full(id: i64) -> Result<bool, String> {
    let conn = db::open()?;
    let full: Option<i64> = conn
        .query_row(
            "SELECT context_full FROM beats WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    Ok(full.unwrap_or(0) != 0)
}

pub fn set_context_full(id: i64, full: bool) -> Result<(), String> {
    db::open()?
        .execute(
            "UPDATE beats SET context_full = ?1 WHERE id = ?2",
            params![full as i64, id],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// A fresh beat holding only this summary — created by `/compact`.
pub fn create_summary_beat(source_id: i64, summary: &str) -> Result<Beat, String> {
    let (name, project_id): (String, Option<i64>) = {
        let conn = db::open()?;
        conn.query_row(
            "SELECT name || ' (compacted)', project_id FROM beats WHERE id = ?1",
            params![source_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or("Beat not found")?
    };
    let beat = create_beat(&name, "Compacted session", project_id)?;
    db::append_messages(
        beat.id,
        vec![serde_json::json!({
            "role": "system",
            "content": format!("Summary of the previous session:\n\n{summary}"),
        })],
    )?;
    Ok(beat)
}

/// All persisted messages of a beat, oldest first.
pub fn get_beat_messages(id: i64) -> Result<Vec<serde_json::Value>, String> {
    let conn = db::open()?;
    let current: String = conn
        .query_row(
            "SELECT messages FROM beats WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or("Beat not found")?;
    serde_json::from_str(&current).map_err(|e| e.to_string())
}

/// Permanently delete a beat and its usage rows. Its worktree, if any, is
/// dropped from disk.
pub fn delete_beat(id: i64) -> Result<String, String> {
    let conn = db::open()?;
    crate::log::info(format!("deleting beat {id}"));
    // fetch the worktree + parent repo before the row is gone
    let (worktree, project): (Option<String>, Option<String>) = conn
        .query_row(
            "SELECT b.worktree, p.path FROM beats b \
             LEFT JOIN projects p ON p.id = b.project_id WHERE b.id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .unwrap_or((None, None));
    conn.execute("DELETE FROM beat_usage WHERE beat_id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    let n = conn
        .execute("DELETE FROM beats WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    if n == 0 {
        return Err("Beat not found".into());
    }
    Ok(match worktree {
        Some(wt) => projects::remove_worktree(&wt, project.as_deref()),
        None => "No worktree attached".into(),
    })
}

async fn usage_cost(model_id: &str, prompt: i64, completion: i64) -> f64 {
    let models = providers::list_models().await.unwrap_or_default();
    models.iter().find(|m| m.id == model_id).map_or(0.0, |m| {
        let p: f64 = m.pricing.prompt.parse().unwrap_or(0.0);
        let c: f64 = m.pricing.completion.parse().unwrap_or(0.0);
        // Go prices differ per model but its /models list carries no pricing,
        // so usage against go/ models is recorded at $0 until OpenCode exposes
        // pricing; limits are per-subscription anyway
        p * prompt as f64 + c * completion as f64
    })
}

/// Record one model call against a beat; cost is computed from the cached
/// OpenRouter pricing at call time. Returns the recorded cost in USD.
pub async fn record_usage(
    beat_id: i64,
    model: &str,
    prompt_tokens: i64,
    completion_tokens: i64,
) -> Result<f64, String> {
    let cost = usage_cost(model, prompt_tokens, completion_tokens).await;
    db::open()?
        .execute(
            "INSERT INTO beat_usage (beat_id, model, prompt_tokens, completion_tokens, cost_usd) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![beat_id, model, prompt_tokens, completion_tokens, cost],
        )
        .map_err(|e| e.to_string())?;
    Ok(cost)
}

pub async fn record_beat_usage(
    beat_id: i64,
    model: String,
    prompt_tokens: i64,
    completion_tokens: i64,
) -> Result<f64, String> {
    record_usage(beat_id, &model, prompt_tokens, completion_tokens).await
}

/// Per-model usage totals for a whole session (beat): summed prompt/completion
/// tokens and cost, one row per model that was ever called in it.
pub fn usage_totals(beat_id: i64) -> Result<Vec<UsageTotal>, String> {
    let conn = db::open()?;
    let mut stmt = conn
        .prepare(
            "SELECT model, SUM(prompt_tokens), SUM(completion_tokens), SUM(cost_usd)
             FROM beat_usage WHERE beat_id = ?1
             GROUP BY model ORDER BY SUM(cost_usd) DESC, model",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![beat_id], |r| {
            Ok(UsageTotal {
                model: r.get(0)?,
                prompt_tokens: r.get(1)?,
                completion_tokens: r.get(2)?,
                cost_usd: r.get(3)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut out = vec![];
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

/// Session-wide usage totals for one model.
#[derive(Clone, serde::Serialize)]
pub struct UsageTotal {
    pub model: String,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cost_usd: f64,
}
