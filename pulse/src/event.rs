//! Crossterm input polling: keys and mouse events.

use crossterm::event::{self, Event};

/// Poll for a crossterm event with a timeout in milliseconds.
pub fn poll(timeout_ms: u64) -> Option<Event> {
    if event::poll(std::time::Duration::from_millis(timeout_ms)).ok()? {
        if let Ok(ev) = event::read() {
            return Some(ev);
        }
    }
    None
}
