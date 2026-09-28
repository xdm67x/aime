//! The `pulse run` workflow picker: a one-screen TUI (crossterm only — no
//! ratatui, to keep the binary lean) that lists the workflows discovered in
//! the current directory, ./.pulse/workflows and ~/.pulse/workflows. Type to
//! filter, move with the arrows, Enter runs the selection, Esc cancels. UI
//! code lives in the binary: pulse-core stays UI-agnostic.

use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute, queue,
    style::{Attribute, Color, ResetColor, SetAttribute, SetForegroundColor},
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use pulse_core::workflows::{self, Workflow};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// What the user did with the picker: chose a workflow (by file path), or
/// cancelled — with the exit code to use (130 for Ctrl-C, 0 for Esc).
pub enum Pick {
    Selected(PathBuf),
    Cancelled(i32),
}

/// One selectable workflow: metadata plus the file it was loaded from.
pub struct Entry {
    name: String,
    desc: String,
    /// Path as shown in the list (home abbreviated to `~`).
    display: String,
    /// Path to run — never abbreviated, `workflows::find` takes it as-is.
    path: PathBuf,
}

impl Entry {
    fn from_wf(wf: Workflow, path: PathBuf) -> Self {
        Self {
            display: abbreviate(&path),
            path,
            name: wf.name,
            desc: wf.description,
        }
    }

    fn matches(&self, filter: &str) -> bool {
        let f = filter.to_lowercase();
        self.name.to_lowercase().contains(&f) || self.desc.to_lowercase().contains(&f)
    }
}

/// Discover every workflow the picker offers: the current directory,
/// ./.pulse/workflows, then ~/.pulse/workflows.
pub fn discover_all() -> Result<Vec<Entry>, String> {
    let mut out = vec![];
    for dir in [
        Path::new(".").to_path_buf(),
        Path::new("./.pulse/workflows").to_path_buf(),
        workflows::dir()?,
    ] {
        for (wf, path) in workflows::discover_dir(&dir) {
            out.push(Entry::from_wf(wf, path));
        }
    }
    Ok(out)
}

/// Abbreviate the home prefix to `~` for display.
fn abbreviate(path: &Path) -> String {
    if let Ok(home) = std::env::var("HOME") {
        if let Ok(rest) = path.strip_prefix(&home) {
            return format!("~/{}", rest.display());
        }
    }
    path.display().to_string()
}

/// Pure picker state, unit-testable without a terminal: the type-to-filter
/// text and the cursor into the filtered list. Any filter edit resets the
/// cursor — the list underneath changed.
struct State {
    filter: String,
    cursor: usize,
}

impl State {
    fn new() -> Self {
        Self {
            filter: String::new(),
            cursor: 0,
        }
    }

    /// The entries visible with the current filter, in discovery order.
    fn visible<'a>(&self, all: &'a [Entry]) -> Vec<&'a Entry> {
        if self.filter.is_empty() {
            return all.iter().collect();
        }
        all.iter().filter(|e| e.matches(&self.filter)).collect()
    }

    fn up(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    fn down(&mut self, all: &[Entry]) {
        let n = self.visible(all).len();
        self.cursor = (self.cursor + 1).min(n.saturating_sub(1));
    }

    fn push_filter(&mut self, c: char) {
        self.filter.push(c);
        self.cursor = 0;
    }

    fn pop_filter(&mut self) {
        self.filter.pop();
        self.cursor = 0;
    }

    fn clear_filter(&mut self) {
        self.filter.clear();
        self.cursor = 0;
    }
}

/* ---- drawing ---- */

const TITLE: &str = "Run a workflow";
const HINTS: &str = "↑/↓ move · type to filter · enter run · esc cancel";
const NO_MATCH: &str = "no match — backspace to clear the filter";

/// Truncate to `w` characters, marking a cut with `…`.
fn trunc(s: &str, w: usize) -> String {
    if s.chars().count() <= w {
        return s.to_string();
    }
    let mut out: String = s.chars().take(w.saturating_sub(1)).collect();
    if w > 0 {
        out.push('…');
    }
    out
}

/// A horizontal frame line, with an optional label after the left corner.
fn frame_line(w: usize, left: char, right: char, label: &str) -> String {
    let l = if label.is_empty() {
        String::new()
    } else {
        format!("─ {label} ")
    };
    let fill = w.saturating_sub(2 + l.chars().count());
    format!("{left}{l}{}{right}", "─".repeat(fill))
}

fn dim(out: &mut io::Stdout) -> io::Result<()> {
    queue!(out, SetForegroundColor(Color::DarkGrey))
}

fn cyan(out: &mut io::Stdout) -> io::Result<()> {
    queue!(out, SetForegroundColor(Color::Cyan))
}

/// Draws the whole frame; the screen is cleared first, so no diffing.
fn draw(out: &mut io::Stdout, st: &State, all: &[Entry]) -> io::Result<()> {
    let (w, h) = terminal::size().unwrap_or((80, 24));
    // a terminal can report a degenerate size (e.g. a pty before its first
    // resize) — clamp to something drawable instead of overflowing
    let (w, h) = ((w as usize).max(40), (h as usize).max(8));
    let inner = w.saturating_sub(4).max(1);
    let visible = st.visible(all);
    // rows between the two dividers — the frame, filter line and footer
    // take the other five
    let shown = h.saturating_sub(5).max(1);
    let top = (st.cursor + 1).saturating_sub(shown);
    let name_w = all
        .iter()
        .map(|e| e.name.chars().count())
        .max()
        .unwrap_or(8)
        .clamp(8, 24);

    queue!(out, Clear(ClearType::All), MoveTo(0, 0))?;

    // frame top + filter line + divider
    dim(out)?;
    out.write_all(frame_line(w, '┌', '┐', TITLE).as_bytes())?;
    queue!(out, MoveTo(0, 1))?;
    out.write_all("│ ".as_bytes())?;
    out.write_all(trunc(&format!("filter > {}_", st.filter), inner - 1).as_bytes())?;
    queue!(out, MoveTo(0, 2))?;
    out.write_all(frame_line(w, '├', '┤', "").as_bytes())?;

    // items (or the no-match note)
    if visible.is_empty() {
        queue!(out, MoveTo(0, 3))?;
        dim(out)?;
        out.write_all(format!("│ {}", trunc(NO_MATCH, inner - 1)).as_bytes())?;
    } else {
        for (i, e) in visible.iter().enumerate().skip(top).take(shown) {
            let row = 3 + (i - top) as u16;
            let sel = i == st.cursor;
            queue!(out, MoveTo(0, row))?;
            if sel {
                cyan(out)?;
                out.write_all("│ ▸ ".as_bytes())?;
                queue!(
                    out,
                    SetForegroundColor(Color::White),
                    SetAttribute(Attribute::Bold)
                )?;
                out.write_all(trunc(&e.name, name_w).as_bytes())?;
                queue!(out, SetAttribute(Attribute::Reset), ResetColor)?;
            } else {
                dim(out)?;
                out.write_all("│   ".as_bytes())?;
                out.write_all(trunc(&e.name, name_w).as_bytes())?;
            }
            // description, then the path right-aligned at the frame edge
            let desc_col = 4 + name_w + 2;
            let desc_w = inner.saturating_sub(name_w + 6 + e.display.chars().count() + 1);
            let desc_s = trunc(&e.desc, desc_w);
            if !desc_s.is_empty() && desc_col + desc_s.chars().count() < w - 2 {
                queue!(out, MoveTo(desc_col as u16, row))?;
                if sel {
                    out.write_all(desc_s.as_bytes())?;
                } else {
                    dim(out)?;
                    out.write_all(desc_s.as_bytes())?;
                }
            }
            let path_s = trunc(&e.display, inner.saturating_sub(2));
            queue!(out, MoveTo((w - 2 - path_s.chars().count()) as u16, row))?;
            dim(out)?;
            out.write_all(path_s.as_bytes())?;
            queue!(out, ResetColor)?;
        }
    }

    // footer + frame bottom
    let last = h.saturating_sub(1) as u16;
    queue!(out, MoveTo(0, last.saturating_sub(1)))?;
    dim(out)?;
    out.write_all(format!("│ {}", trunc(HINTS, inner - 1)).as_bytes())?;
    queue!(out, MoveTo(0, last))?;
    out.write_all(frame_line(w, '└', '┘', "").as_bytes())?;
    queue!(out, ResetColor)?;
    out.flush()
}

/* ---- terminal guard ---- */

/// Raw mode + the alternate screen, restored on drop — including panics,
/// so a crash can never leave the user's terminal broken.
struct Tui;

impl Tui {
    fn enter() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, Hide)?;
        Ok(Self)
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), Show, LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}

/* ---- event loop ---- */

/// Open the picker over `all`. Blocks until the user chooses or cancels;
/// the terminal is restored on every path.
pub fn pick(all: &[Entry]) -> Result<Pick, String> {
    if all.is_empty() {
        return Ok(Pick::Cancelled(0));
    }
    let mut out = io::stdout();
    let _tui = Tui::enter().map_err(|e| {
        format!("Failed to open the picker ({e}); run a workflow directly instead: pulse run <workflow>")
    })?;
    let mut st = State::new();
    loop {
        draw(&mut out, &st, all).map_err(|e| format!("picker: {e}"))?;
        let ev = event::read().map_err(|e| format!("picker: {e}"))?;
        let Event::Key(k) = ev else {
            continue; // resize → redraw; mouse etc. → ignore
        };
        if k.kind != KeyEventKind::Press {
            continue;
        }
        match k.code {
            KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                return Ok(Pick::Cancelled(crate::run::CANCELLED))
            }
            KeyCode::Esc => return Ok(Pick::Cancelled(0)),
            KeyCode::Up => st.up(),
            KeyCode::Down => st.down(all),
            KeyCode::Enter => {
                if let Some(e) = st.visible(all).get(st.cursor) {
                    return Ok(Pick::Selected(e.path.clone()));
                }
            }
            KeyCode::Backspace => st.pop_filter(),
            KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => st.clear_filter(),
            KeyCode::Char(c) => st.push_filter(c),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, desc: &str) -> Entry {
        Entry {
            name: name.into(),
            desc: desc.into(),
            display: format!("./{name}.yml"),
            path: PathBuf::from(format!("./{name}.yml")),
        }
    }

    #[test]
    fn test_trunc() {
        assert_eq!(trunc("hello", 5), "hello");
        assert_eq!(trunc("hello", 10), "hello");
        assert_eq!(trunc("hello world", 6), "hello…");
        assert_eq!(trunc("", 3), "");
        assert_eq!(trunc("abc", 0), "");
    }

    #[test]
    fn test_frame_line() {
        let l = frame_line(12, '┌', '┐', "hi");
        assert!(l.starts_with("┌─ hi ─") && l.ends_with('┐') && l.chars().count() == 12);
        let l = frame_line(8, '└', '┘', "");
        assert_eq!(l.chars().count(), 8);
    }

    #[test]
    fn test_filter_matches_name_and_desc() {
        let e = entry("Ship", "Release the branch");
        assert!(e.matches("shi"));
        assert!(e.matches("RELEASE"));
        assert!(!e.matches("deploy"));
    }

    #[test]
    fn test_visible_filters_and_cursor_resets() {
        let all = vec![entry("ship", "Ship it"), entry("review", "Review it")];
        let mut st = State::new();
        assert_eq!(st.visible(&all).len(), 2);
        st.down(&all);
        assert_eq!(st.cursor, 1);
        // any filter edit resets the cursor
        st.push_filter('s');
        assert_eq!(st.cursor, 0);
        let vis = st.visible(&all);
        assert_eq!(vis.len(), 1);
        assert_eq!(vis[0].name, "ship");
        // backspace restores the full list
        st.pop_filter();
        assert_eq!(st.visible(&all).len(), 2);
        // no match → empty, Enter is a no-op, down clamps to 0
        st.push_filter('z');
        assert!(st.visible(&all).is_empty());
        st.down(&all);
        assert_eq!(st.cursor, 0);
    }

    #[test]
    fn test_cursor_clamps_to_list_end() {
        let all = vec![entry("a", ""), entry("b", "")];
        let mut st = State::new();
        st.down(&all);
        st.down(&all);
        st.down(&all);
        assert_eq!(st.cursor, 1);
        st.up();
        st.up();
        st.up();
        assert_eq!(st.cursor, 0);
    }
}
