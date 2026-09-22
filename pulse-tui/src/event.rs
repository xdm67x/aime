//! Crossterm input polling and key mapping.

use crossterm::event::{self, Event, KeyEvent};

/// Poll for a crossterm event with a timeout in milliseconds.
pub fn poll(timeout_ms: u64) -> Option<KeyEvent> {
    if event::poll(std::time::Duration::from_millis(timeout_ms)).ok()? {
        if let Ok(Event::Key(key)) = event::read() {
            return Some(key);
        }
    }
    None
}
