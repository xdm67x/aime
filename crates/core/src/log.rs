//! Logging for the harness and every runtime built on it. Always writes to
//! files under `~/.aime/logs/` — never to stdout/stderr, which belong to the
//! calling app's UI: stray writes there corrupt terminal apps that render the
//! screen themselves (the TUI only redraws changed cells, so foreign output
//! smears the display until restart).
//!
//! One file per day (`aime-YYYY-MM-DD.log`), so a day of logs is easy to
//! attach to a bug report and old days can be pruned. Every write is
//! best-effort: a failing write never panics and never breaks the caller.

use std::io::Write;

/// How many daily log files to keep; older ones are pruned on each write.
const RETAIN_FILES: usize = 14;

/// Append one line at info level to today's log file. Best-effort; never panics.
pub fn log(msg: impl std::fmt::Display) {
    write("INFO", &msg.to_string());
}

/// Append a debug line: chatty per-request/per-tool detail.
pub fn debug(msg: impl std::fmt::Display) {
    write("DEBUG", &msg.to_string());
}

/// Append an info line: normal lifecycle events.
pub fn info(msg: impl std::fmt::Display) {
    write("INFO", &msg.to_string());
}

/// Append a warning line: something failed but was recovered from.
pub fn warn(msg: impl std::fmt::Display) {
    write("WARN", &msg.to_string());
}

/// Append an error line: an operation failed outright.
pub fn error(msg: impl std::fmt::Display) {
    write("ERROR", &msg.to_string());
}

/// The `~/.aime/logs` directory, created on demand.
fn log_dir() -> Option<std::path::PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let dir = std::path::Path::new(&home).join(".aime").join("logs");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// One entry: `[HH:MM:SS] [LEVEL] message`, newlines in the message collapsed
/// to `\\n` so every entry is exactly one line (grep-friendly).
fn write(level: &str, msg: &str) {
    let Some(dir) = log_dir() else {
        return;
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let path = dir.join(format!("aime-{}.log", date_string(now)));
    let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    else {
        return;
    };
    let flat = msg.replace('\n', "\\n");
    let _ = writeln!(f, "[{}] [{}] {}", time_string(now), level, flat);
    prune(&dir);
}

/// Drop all but the newest `RETAIN_FILES` daily log files. Best-effort.
fn prune(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            (name.starts_with("aime-") && name.ends_with(".log")).then_some(name)
        })
        .collect();
    names.sort();
    names.truncate(names.len().saturating_sub(RETAIN_FILES));
    for name in names {
        let _ = std::fs::remove_file(dir.join(name));
    }
}

/// `HH:MM:SS` local time for a unix timestamp.
fn time_string(secs: i64) -> String {
    let tod = secs.rem_euclid(86_400);
    format!("{:02}:{:02}:{:02}", tod / 3600, (tod % 3600) / 60, tod % 60)
}

/// `YYYY-MM-DD` local date for a unix timestamp.
fn date_string(secs: i64) -> String {
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

/// Civil calendar date from days since 1970-01-01 (Howard Hinnant's
/// `civil_from_days`), so no date dependency is needed.
fn civil_from_days(z: i64) -> (i64, u64, u64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u64;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u64;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Serializes HOME-redirection across tests (set_var is process-global).
#[cfg(test)]
pub(crate) static HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    // one test because std::env::set_var("HOME") is process-global and races
    // across parallel tests (shares HOME_LOCK with the db tests)
    #[test]
    fn test_write_and_prune() {
        let _g = HOME_LOCK.lock().unwrap();
        let tmp = std::env::temp_dir().join(format!("aime-log-test-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        std::env::set_var("HOME", &tmp);

        info("hello world");
        error("boom:\nmultiline");
        let dir = tmp.join(".aime").join("logs");
        let mut files: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        files.retain(|n| n.starts_with("aime-"));
        assert_eq!(files.len(), 1);
        assert!(files[0].starts_with("aime-") && files[0].ends_with(".log"));
        let content = std::fs::read_to_string(dir.join(&files[0])).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("] [INFO] hello world"));
        assert!(lines[1].contains("] [ERROR] boom:\\nmultiline"));
        assert!(lines[1].starts_with('['));

        // prune: seed more daily files than the retention window keeps
        for i in 0..(RETAIN_FILES as i64 + 5) {
            let name = format!("aime-2000-01-{:02}.log", i + 1);
            std::fs::write(dir.join(name), "x").unwrap();
        }
        info("again");
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("aime-") && n.ends_with(".log"))
            .collect();
        names.sort();
        assert_eq!(names.len(), RETAIN_FILES);
        // today's file is the newest and always survives
        assert_eq!(names.last().unwrap(), &files[0]);

        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn test_civil_from_days() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(civil_from_days(20_651), (2026, 7, 17)); // spans a leap year
        assert_eq!(civil_from_days(20_672), (2026, 8, 7));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        assert_eq!(date_string(0), "1970-01-01");
        assert_eq!(time_string(0), "00:00:00");
        assert_eq!(time_string(86_399), "23:59:59");
    }
}
