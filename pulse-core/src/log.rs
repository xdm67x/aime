//! Debug/error logging for the harness. Always writes to
//! `~/.pulse/pulse.log` — never to stdout/stderr, which belong to the calling
//! app's UI: stray writes there corrupt terminal apps that render the screen
//! themselves (the TUI only redraws changed cells, so foreign output smears
//! the display until restart).

use std::io::Write;

/// Append one line to `~/.pulse/pulse.log`. Best-effort; never panics.
pub fn log(msg: impl std::fmt::Display) {
    let home = std::env::var("HOME").unwrap_or_default();
    let path = std::path::Path::new(&home).join(".pulse").join("pulse.log");
    let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    else {
        return;
    };
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let _ = writeln!(f, "[{ts}] {msg}");
}
