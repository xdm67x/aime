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

    #[test]
    fn test_settings_roundtrip() {
        // redirect home to a temp dir so we never touch the real ~/.pulse
        let tmp = std::env::temp_dir().join(format!("pulse-test-{}", std::process::id()));
        std::env::set_var("HOME", &tmp);
        let k = "test_key";
        assert_eq!(get_setting(k).unwrap(), None);
        set_setting(k, "v1").unwrap();
        set_setting(k, "v2").unwrap(); // upsert overwrites
        assert_eq!(get_setting(k).unwrap().as_deref(), Some("v2"));
        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
