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
        "CREATE TABLE IF NOT EXISTS beats (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            description TEXT NOT NULL DEFAULT '',
            archived INTEGER NOT NULL DEFAULT 0,
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
    let _ = conn.execute("ALTER TABLE beats ADD COLUMN description TEXT NOT NULL DEFAULT ''", []);
    conn.pragma_update(None, "foreign_keys", "ON").map_err(|e| e.to_string())?;
    Ok(conn)
}

pub fn get_setting(key: &str) -> Result<Option<String>, String> {
    open()?
        .query_row("SELECT value FROM config WHERE key = ?1", params![key], |row| {
            row.get(0)
        })
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
        conn.execute("INSERT INTO beats (name, description) VALUES ('b1', 'd1')", []).unwrap();
        let id = conn.last_insert_rowid();
        conn.execute("INSERT INTO beat_usage (beat_id, model, prompt_tokens, completion_tokens, cost_usd) VALUES (?1, 'm', 10, 5, 0.5)", params![id]).unwrap();
        // FK: usage without a beat is rejected
        assert!(conn.execute("INSERT INTO beat_usage (beat_id, model) VALUES (9999, 'm')", []).is_err());
        conn.execute("UPDATE beats SET archived = 1 WHERE id = ?1", params![id]).unwrap();
        let (archived, cost): (i64, f64) = conn.query_row(
            "SELECT b.archived, COALESCE(SUM(u.cost_usd), 0) FROM beats b LEFT JOIN beat_usage u ON u.beat_id = b.id WHERE b.id = ?1",
            params![id], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!((archived, cost), (1, 0.5));
        let (name, description): (String, String) = conn.query_row(
            "SELECT name, description FROM beats WHERE id = ?1", params![id], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!((name.as_str(), description.as_str()), ("b1", "d1"));
        // delete only works on archived beats
        conn.execute("DELETE FROM beat_usage WHERE beat_id = ?1", params![id]).unwrap();
        assert_eq!(conn.execute("DELETE FROM beats WHERE id = ?1 AND archived = 1", params![id]).unwrap(), 1);
        assert_eq!(conn.execute("DELETE FROM beats WHERE id = ?1 AND archived = 1", params![id]).unwrap(), 0);
        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
