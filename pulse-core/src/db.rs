use rusqlite::{params, Connection, OptionalExtension};

/// Opens ~/.pulse/pulse.db, creating the directory and config table as needed.
pub fn open() -> Result<Connection, String> {
    let home = std::env::var("HOME").map_err(|e| e.to_string())?;
    let dir = std::path::Path::new(&home).join(".pulse");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let conn = Connection::open(dir.join("pulse.db")).map_err(|e| e.to_string())?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS config (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
        [],
    )
    .map_err(|e| e.to_string())?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS projects (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL UNIQUE,
            path TEXT NOT NULL UNIQUE,
            source TEXT NOT NULL DEFAULT 'local',
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE TABLE IF NOT EXISTS beats (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            description TEXT NOT NULL DEFAULT '',
            archived INTEGER NOT NULL DEFAULT 0,
            messages TEXT NOT NULL DEFAULT '[]',
            project_id INTEGER REFERENCES projects(id) ON DELETE SET NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE TABLE IF NOT EXISTS beat_usage (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            beat_id INTEGER NOT NULL REFERENCES beats(id),
            model TEXT NOT NULL,
            prompt_tokens INTEGER NOT NULL DEFAULT 0,
            completion_tokens INTEGER NOT NULL DEFAULT 0,
            cost_usd REAL NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );",
    )
    .map_err(|e| e.to_string())?;
    // migrate pre-description databases
    let _ = conn.execute(
        "ALTER TABLE beats ADD COLUMN description TEXT NOT NULL DEFAULT ''",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE beats ADD COLUMN messages TEXT NOT NULL DEFAULT '[]'",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE beats ADD COLUMN project_id INTEGER REFERENCES projects(id) ON DELETE SET NULL",
        [],
    );
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(|e| e.to_string())?;
    dedupe_messages(&conn);
    Ok(conn)
}

/// Older builds double-persisted base-tier answers (the agentic draft was
/// pushed as its own entry and again as the final answer, identical and
/// adjacent). Drop adjacent duplicate messages once per open.
fn dedupe_messages(conn: &Connection) {
    let Ok(mut stmt) = conn.prepare("SELECT id, messages FROM beats") else {
        return;
    };
    let Ok(rows) = stmt.query_map([], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
    }) else {
        return;
    };
    let beats: Vec<(i64, Vec<serde_json::Value>)> = rows
        .filter_map(|r| r.ok())
        .filter_map(|(id, msgs)| serde_json::from_str(&msgs).ok().map(|a| (id, a)))
        .collect();
    for (id, arr) in beats {
        let mut cleaned: Vec<serde_json::Value> = Vec::with_capacity(arr.len());
        for m in &arr {
            let dup = cleaned.last().is_some_and(|p| {
                p["role"] == m["role"] && p["model"] == m["model"] && p["content"] == m["content"]
            });
            if !dup {
                cleaned.push(m.clone());
            }
        }
        if cleaned.len() < arr.len() {
            let _ = conn.execute(
                "UPDATE beats SET messages = ?1 WHERE id = ?2",
                params![serde_json::to_string(&cleaned).unwrap_or_default(), id],
            );
        }
    }
}

pub fn get_setting(key: &str) -> Result<Option<String>, String> {
    open()?
        .query_row(
            "SELECT value FROM config WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())
}

pub fn set_setting(key: &str, value: &str) -> Result<(), String> {
    open()?
        .execute(
            "INSERT INTO config (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Append message entries to a beat's messages JSON array (stored as JSON text,
/// SQLite's JSONB storage). Each entry gets a `ts` timestamp.
pub fn append_messages(beat_id: i64, entries: Vec<serde_json::Value>) -> Result<(), String> {
    let conn = open()?;
    let current: String = conn
        .query_row(
            "SELECT messages FROM beats WHERE id = ?1",
            params![beat_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or("Beat not found")?;
    let ts: String = conn
        .query_row("SELECT datetime('now')", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    let mut arr: Vec<serde_json::Value> = serde_json::from_str(&current).unwrap_or_default();
    for mut e in entries {
        e["ts"] = serde_json::json!(ts);
        arr.push(e);
    }
    conn.execute(
        "UPDATE beats SET messages = ?1 WHERE id = ?2",
        params![
            serde_json::to_string(&arr).map_err(|e| e.to_string())?,
            beat_id
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // one test because std::env::set_var("HOME") is process-global and races across parallel tests
    #[test]
    fn test_db_roundtrip() {
        // redirect home to a temp dir so we never touch the real ~/.pulse
        let tmp = std::env::temp_dir().join(format!("pulse-test-{}", std::process::id()));
        std::env::set_var("HOME", &tmp);
        let k = "test_key";
        assert_eq!(get_setting(k).unwrap(), None);
        set_setting(k, "v1").unwrap();
        set_setting(k, "v2").unwrap(); // upsert overwrites
        assert_eq!(get_setting(k).unwrap().as_deref(), Some("v2"));

        let conn = open().unwrap();
        conn.execute(
            "INSERT INTO beats (name, description) VALUES ('b1', 'd1')",
            [],
        )
        .unwrap();
        let id = conn.last_insert_rowid();
        conn.execute("INSERT INTO beat_usage (beat_id, model, prompt_tokens, completion_tokens, cost_usd) VALUES (?1, 'm', 10, 5, 0.5)", params![id]).unwrap();
        // FK: usage without a beat is rejected
        assert!(conn
            .execute(
                "INSERT INTO beat_usage (beat_id, model) VALUES (9999, 'm')",
                []
            )
            .is_err());
        conn.execute("UPDATE beats SET archived = 1 WHERE id = ?1", params![id])
            .unwrap();
        let (archived, cost): (i64, f64) = conn.query_row(
            "SELECT b.archived, COALESCE(SUM(u.cost_usd), 0) FROM beats b LEFT JOIN beat_usage u ON u.beat_id = b.id WHERE b.id = ?1",
            params![id], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!((archived, cost), (1, 0.5));
        let (name, description): (String, String) = conn
            .query_row(
                "SELECT name, description FROM beats WHERE id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((name.as_str(), description.as_str()), ("b1", "d1"));
        // messages JSON array roundtrip
        append_messages(
            id,
            vec![serde_json::json!({"role": "user", "content": "hi"})],
        )
        .unwrap();
        append_messages(
            id,
            vec![serde_json::json!({"role": "assistant", "content": "yo"})],
        )
        .unwrap();
        let msgs: String = conn
            .query_row(
                "SELECT messages FROM beats WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        let arr: Vec<serde_json::Value> = serde_json::from_str(&msgs).unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["content"], "hi");
        assert!(arr[1]["ts"].as_str().unwrap().len() == 19); // datetime('now') format
        // adjacent duplicate cleanup: append an identical reply, reopen, dedupe
        append_messages(
            id,
            vec![serde_json::json!({"role": "assistant", "content": "yo"})],
        )
        .unwrap();
        open().unwrap();
        let msgs: String = conn
            .query_row(
                "SELECT messages FROM beats WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        let arr: Vec<serde_json::Value> = serde_json::from_str(&msgs).unwrap();
        assert_eq!(arr.len(), 2); // identical adjacent reply removed
                                                             // appending to a nonexistent beat fails
        assert!(append_messages(9999, vec![serde_json::json!({"a": 1})]).is_err());
        // delete only works on archived beats
        conn.execute("DELETE FROM beat_usage WHERE beat_id = ?1", params![id])
            .unwrap();
        assert_eq!(
            conn.execute(
                "DELETE FROM beats WHERE id = ?1 AND archived = 1",
                params![id]
            )
            .unwrap(),
            1
        );
        assert_eq!(
            conn.execute(
                "DELETE FROM beats WHERE id = ?1 AND archived = 1",
                params![id]
            )
            .unwrap(),
            0
        );
        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
