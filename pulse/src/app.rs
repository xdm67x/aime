//! App state, mode switching, and action dispatch.

use pulse_core::beats::Beat;
use pulse_core::harness::{TaggedEvent, TaskEvent};
use pulse_core::projects::Project;
use ratatui::layout::Rect;
use ratatui::widgets::ListState;
use std::collections::HashSet;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Chat,
    Sessions,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::Chat => "Chat",
            Mode::Sessions => "Sessions",
        }
    }
}

#[derive(Clone)]
pub enum TranscriptLine {
    User(String),
    Assistant(String),
    Tool {
        tool: String,
        arguments: String,
        result: String,
        error: bool,
    },
    Step(String),
    System(String),
    Error(String),
}

#[derive(Clone)]
#[allow(dead_code)]
pub enum Popup {
    Error(String),
    Help,
    Confirm(String, ConfirmAction),
}

#[derive(Clone)]
#[allow(dead_code, clippy::enum_variant_names)]
pub enum ConfirmAction {
    DeleteBeat(i64),
    DeleteProject(i64),
    DeleteWorkflow(String),
}

/// Autocomplete popup opened by typing `@` in the chat input.
#[derive(Clone)]
pub enum AtPopup {
    Projects { filter: String, selected: usize },
}

/// Small centered text input (add local project / clone GitHub repo).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum InputPopup {
    AddPath,
    CloneRepo,
}

/// One transcript entry's rendered row range, used to map mouse clicks back
/// to the entry they hit. `tool` is set for tool-call entries (the clickable
/// ones whose output is collapsed by default).
#[derive(Clone, Copy)]
pub struct EntryRow {
    /// First content row (before scrolling) occupied by the entry.
    pub start: usize,
    /// Height in rendered rows.
    pub height: usize,
    /// Transcript index when the entry is a tool call.
    pub tool: Option<usize>,
}

/// Screen rectangles filled in during render, read by the mouse handler.
#[derive(Clone, Copy, Default)]
pub struct UiRects {
    /// Inner rect of the sessions sidebar, when visible.
    pub sidebar: Option<Rect>,
    /// Inner rect of the transcript area.
    pub transcript: Option<Rect>,
    /// Rect of the `@` project popup, when open.
    pub at_popup: Option<Rect>,
    /// Rect of the input popup, when open.
    pub input_popup: Option<Rect>,
}

impl UiRects {
    pub fn contains(rect: Rect, x: u16, y: u16) -> bool {
        x >= rect.x && x < rect.x + rect.width && y >= rect.y && y < rect.y + rect.height
    }
}

/// Find the tool entry whose rendered rows cover `content_row`.
pub fn entry_row_at(rows: &[EntryRow], content_row: usize) -> Option<usize> {
    rows.iter()
        .find(|r| content_row >= r.start && content_row < r.start + r.height && r.tool.is_some())
        .and_then(|r| r.tool)
}

/// Derive a short session name from a prompt: drop any leading `@project`
/// mentions, then keep the first few words.
pub fn session_name_from_prompt(prompt: &str) -> String {
    let mut words: Vec<String> = vec![];
    for word in prompt.split_whitespace() {
        if words.is_empty() && word.starts_with('@') {
            continue; // the @project mention is not part of the name
        }
        let word = word.trim_matches(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_'));
        if !word.is_empty() {
            words.push(word.to_string());
        }
        if words.len() >= 4 {
            break;
        }
    }
    let mut name = words.join(" ");
    if name.chars().count() > 28 {
        let trimmed: String = name.chars().take(28).collect();
        name = trimmed.trim_end().to_string();
    }
    if name.is_empty() {
        return "New session".into();
    }
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_row_at_finds_tool_entries() {
        let rows = vec![
            EntryRow {
                start: 0,
                height: 2,
                tool: None,
            },
            EntryRow {
                start: 2,
                height: 2,
                tool: Some(1),
            },
            EntryRow {
                start: 4,
                height: 6,
                tool: Some(2),
            },
        ];
        assert_eq!(entry_row_at(&rows, 0), None); // non-tool entry
        assert_eq!(entry_row_at(&rows, 2), Some(1));
        assert_eq!(entry_row_at(&rows, 3), Some(1)); // blank row of entry 1
        assert_eq!(entry_row_at(&rows, 9), Some(2)); // last row of entry 2
        assert_eq!(entry_row_at(&rows, 10), None); // past the end
    }

    #[test]
    fn session_name_skips_mention_and_caps() {
        assert_eq!(
            session_name_from_prompt("@pulse fix the login flow now"),
            "Fix the login flow"
        );
        assert_eq!(session_name_from_prompt("add tests"), "Add tests");
    }

    #[test]
    fn session_name_caps_length_and_falls_back() {
        let long = " ".repeat(0) + &"word ".repeat(10);
        let name = session_name_from_prompt(&long);
        assert!(name.chars().count() <= 28);
        assert_eq!(session_name_from_prompt("@project"), "New session");
    }
}

pub struct App {
    pub mode: Mode,
    pub beats: Vec<Beat>,
    pub active_beat_id: Option<i64>,
    pub transcript: Vec<TranscriptLine>,
    pub input: String,
    pub input_cursor: usize,
    pub projects: Vec<Project>,
    pub running: bool,
    pub task_running: bool,
    pub error: Option<String>,
    pub event_tx: mpsc::UnboundedSender<TaggedEvent>,
    pub event_rx: mpsc::UnboundedReceiver<TaggedEvent>,
    pub task_handle: Option<JoinHandle<()>>,
    pub scroll: u16,
    #[allow(dead_code)]
    pub popup: Option<Popup>,
    pub show_help: bool,
    pub session_list: ListState,
    pub at_popup: Option<AtPopup>,
    pub input_popup: Option<InputPopup>,
    pub input_popup_text: String,
    pub clone_handle: Option<JoinHandle<Result<Project, String>>>,
    /// Tool-call entries whose output is expanded (transcript indices).
    pub expanded_tools: HashSet<usize>,
    /// Rendered row ranges of transcript entries (filled during render).
    pub entry_rows: Vec<EntryRow>,
    /// Screen rects of interactive areas (filled during render).
    pub rects: UiRects,
    /// Project picked via `@`: the session (and its worktree) is only created
    /// when the next prompt is sent. Holds (project id, project name).
    pub pending_project: Option<(i64, String)>,
    /// Keep the transcript pinned to the bottom as new output arrives.
    pub follow: bool,
    pub current_model: String,
    pub current_tier: String,
    pub current_cost: f64,
    pub current_context: Option<f64>,
}

impl App {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel::<TaggedEvent>();
        let beats = pulse_core::beats::list_beats().unwrap_or_default();
        let projects = pulse_core::projects::list_projects().unwrap_or_default();
        let active_beat_id = beats.first().map(|b| b.id);

        let mut app = Self {
            mode: Mode::Chat,
            beats,
            active_beat_id,
            transcript: vec![],
            input: String::new(),
            input_cursor: 0,
            projects,
            running: true,
            task_running: false,
            error: None,
            event_tx: tx,
            event_rx: rx,
            task_handle: None,
            scroll: 0,
            popup: None,
            show_help: false,
            session_list: ListState::default(),
            at_popup: None,
            input_popup: None,
            input_popup_text: String::new(),
            clone_handle: None,
            expanded_tools: HashSet::new(),
            entry_rows: Vec::new(),
            rects: UiRects::default(),
            pending_project: None,
            follow: true,
            current_model: String::new(),
            current_tier: String::new(),
            current_cost: 0.0,
            current_context: None,
        };
        if let Some(id) = active_beat_id {
            app.load_transcript(id);
        }
        app
    }

    pub fn refresh_beats(&mut self) {
        self.beats = pulse_core::beats::list_beats().unwrap_or_default();
    }

    pub fn refresh_projects(&mut self) {
        self.projects = pulse_core::projects::list_projects().unwrap_or_default();
    }

    /// Currently selected session index in the sidebar list.
    pub fn session_selected(&self) -> usize {
        self.session_list.selected().unwrap_or(0)
    }

    /// Select a session row, clamped to the list length.
    pub fn session_select(&mut self, index: usize) {
        self.session_list
            .select(Some(index.min(self.beats.len().saturating_sub(1))));
    }

    /// Move the session selection by `delta` rows, clamped.
    pub fn session_move(&mut self, delta: i32) {
        if self.beats.is_empty() {
            return;
        }
        let next = (self.session_selected() as i32 + delta).clamp(0, self.beats.len() as i32 - 1);
        self.session_select(next as usize);
    }

    /// First visible session row in the sidebar (list scroll offset).
    pub fn session_offset(&self) -> usize {
        self.session_list.offset()
    }

    /// Toggle expanded output for a tool-call transcript entry.
    pub fn toggle_tool(&mut self, transcript_idx: usize) {
        if !self.expanded_tools.remove(&transcript_idx) {
            self.expanded_tools.insert(transcript_idx);
        }
    }

    /// Map a transcript content row to the tool entry it lands on, if any.
    pub fn tool_at_row(&self, content_row: usize) -> Option<usize> {
        entry_row_at(&self.entry_rows, content_row)
    }

    /// Projects matching the `@` popup filter (case-insensitive substring).
    pub fn matching_projects(&self, filter: &str) -> Vec<&Project> {
        let f = filter.to_lowercase();
        self.projects
            .iter()
            .filter(|p| p.name.to_lowercase().contains(&f))
            .collect()
    }

    /// Number of entries in the `@` popup: matching projects + 2 fixed actions.
    pub fn at_entries(&self) -> usize {
        match &self.at_popup {
            Some(AtPopup::Projects { filter, .. }) => self.matching_projects(filter).len() + 2,
            None => 0,
        }
    }

    /// Recompute the `@` popup filter from the input (text after the `@` up to
    /// the cursor). Closes the popup when the `@` is gone or the user typed
    /// whitespace after it.
    pub fn update_at_popup(&mut self) {
        let mut selected = match &self.at_popup {
            Some(AtPopup::Projects { selected, .. }) => *selected,
            None => return,
        };
        let upto = &self.input[..self.input_cursor];
        let Some(pos) = upto.rfind('@') else {
            self.at_popup = None;
            return;
        };
        let f = &upto[pos + 1..];
        if f.contains(char::is_whitespace) {
            self.at_popup = None;
            return;
        }
        let entries = self.matching_projects(f).len() + 2;
        if selected >= entries {
            selected = entries.saturating_sub(1);
        }
        self.at_popup = Some(AtPopup::Projects {
            filter: f.to_string(),
            selected,
        });
    }

    /// Move the `@` popup selection, clamped to the entry count.
    pub fn at_move(&mut self, delta: i32) {
        let entries = self.at_entries() as i32;
        if let Some(AtPopup::Projects { selected, .. }) = &mut self.at_popup {
            let next = (*selected as i32 + delta).clamp(0, (entries - 1).max(0));
            *selected = next as usize;
        }
    }

    /// Attach a project picked via `@`: no session is created yet. The beat
    /// (with its git worktree) is created when the next prompt is sent, and
    /// the prompt text names the session. The project itself is never part of
    /// the prompt — the session runs inside the project directory.
    pub fn attach_pending_project(&mut self, project_id: i64, name: &str) {
        self.pending_project = Some((project_id, name.to_string()));
        self.mode = Mode::Chat;
        self.input.clear();
        self.input_cursor = 0;
        self.transcript.push(TranscriptLine::System(format!(
            "→ next prompt starts a new session in {name}"
        )));
    }

    /// Load the persisted transcript for a beat into `TranscriptLine`s.
    pub fn load_transcript(&mut self, beat_id: i64) {
        self.transcript.clear();
        self.scroll = 0;
        self.follow = true;
        // Expansion state is keyed by transcript index — meaningless after a reload.
        self.expanded_tools.clear();
        self.current_cost = 0.0;
        self.current_context = None;
        self.current_model.clear();
        self.current_tier.clear();
        // Populate the status bar from the session's recorded usage.
        if let Ok(totals) = pulse_core::beats::usage_totals(beat_id) {
            self.current_cost = totals.iter().map(|t| t.cost_usd).sum();
            if let Some(top) = totals.first() {
                self.current_model = top.model.clone();
            }
        }
        match pulse_core::beats::get_beat_messages(beat_id) {
            Ok(messages) => {
                for m in messages {
                    let role = m["role"].as_str().unwrap_or("");
                    let content = m["content"].as_str().unwrap_or("").to_string();
                    match role {
                        "user" => {
                            if !content.trim().is_empty() {
                                self.transcript.push(TranscriptLine::User(content));
                            }
                        }
                        "assistant" => {
                            if !content.trim().is_empty() {
                                self.transcript.push(TranscriptLine::Assistant(content));
                            }
                        }
                        "tool" => {
                            let tool = m["model"].as_str().unwrap_or("tool").to_string();
                            let arguments = m["arguments"].as_str().unwrap_or("").to_string();
                            let result = content.clone();
                            let error = m["error"].as_bool().unwrap_or(false);
                            self.transcript.push(TranscriptLine::Tool {
                                tool,
                                arguments,
                                result,
                                error,
                            });
                        }
                        "system" if !content.trim().is_empty() => {
                            self.transcript.push(TranscriptLine::System(content));
                        }
                        _ => {}
                    }
                }
            }
            Err(e) => {
                self.transcript.push(TranscriptLine::Error(format!(
                    "Failed to load messages: {e}"
                )));
            }
        }
    }

    /// Switch to a different beat/session.
    pub fn switch_beat(&mut self, beat_id: i64) {
        self.pending_project = None;
        self.active_beat_id = Some(beat_id);
        self.load_transcript(beat_id);
        self.mode = Mode::Chat;
    }

    /// Create a new beat and switch to it.
    pub fn new_beat(&mut self, name: &str) {
        match pulse_core::beats::create_beat(name, "", None) {
            Ok(beat) => {
                self.refresh_beats();
                self.switch_beat(beat.id);
            }
            Err(e) => {
                self.error = Some(e);
            }
        }
    }

    /// Drain all pending task events from the channel and append to transcript.
    pub fn drain_events(&mut self) {
        while let Ok(ev) = self.event_rx.try_recv() {
            self.handle_task_event(ev);
        }
    }

    fn handle_task_event(&mut self, ev: TaggedEvent) {
        if Some(ev.beat_id) != self.active_beat_id {
            return;
        }
        match ev.ev {
            TaskEvent::Start { model, tier } => {
                self.current_model = model.clone();
                self.current_tier = tier.clone();
                self.transcript
                    .push(TranscriptLine::System(format!("→ {model} ({tier})")));
            }
            TaskEvent::Delta { text } => {
                if let Some(TranscriptLine::Assistant(existing)) = self.transcript.last_mut() {
                    existing.push_str(&text);
                } else {
                    self.transcript.push(TranscriptLine::Assistant(text));
                }
            }
            TaskEvent::Tool {
                tool,
                arguments,
                result,
                error,
            } => {
                self.transcript.push(TranscriptLine::Tool {
                    tool,
                    arguments,
                    result,
                    error,
                });
            }
            TaskEvent::Step { text } => {
                self.transcript.push(TranscriptLine::Step(text));
            }
        }
    }

    /// Mark the task as finished and refresh state.
    pub fn task_finished(&mut self) {
        self.task_running = false;
        self.task_handle = None;
        if let Some(id) = self.active_beat_id {
            self.load_transcript(id);
        }
        self.refresh_beats();
    }

    /// Cancel the running task.
    pub fn cancel_task(&mut self) {
        if let Some(id) = self.active_beat_id {
            pulse_core::harness::cancel_current(id);
        }
    }

    /// Send the current input as a prompt.
    pub fn send_input(&mut self) {
        let input = self.input.trim().to_string();
        if input.is_empty() {
            return;
        }

        // Handle slash commands
        if input.starts_with('/') {
            self.input.clear();
            self.input_cursor = 0;
            self.handle_slash_command(&input);
            return;
        }

        // Errors below keep the prompt in the input bar so it is not lost.
        if self.task_running {
            self.error = Some("A task is already running. Cancel with Ctrl+K.".into());
            return;
        }

        // A project picked via `@`: create the session (and its worktree) now,
        // named after the prompt.
        if let Some((project_id, _)) = self.pending_project.take() {
            let name = session_name_from_prompt(&input);
            match pulse_core::beats::create_beat(&name, "", Some(project_id)) {
                Ok(beat) => {
                    self.refresh_beats();
                    self.active_beat_id = Some(beat.id);
                    self.load_transcript(beat.id);
                    if let Some(status) = &beat.worktree_status {
                        self.transcript.push(TranscriptLine::System(status.clone()));
                    }
                }
                Err(e) => {
                    self.pending_project = Some((project_id, String::new()));
                    self.error = Some(e);
                    return;
                }
            }
        }

        let beat_id = match self.active_beat_id {
            Some(id) => id,
            None => {
                self.error = Some("No active session. Create one with /new {name}.".into());
                return;
            }
        };

        self.input.clear();
        self.input_cursor = 0;
        self.transcript.push(TranscriptLine::User(input.clone()));
        self.task_running = true;
        let tx = self.event_tx.clone();
        let handle = crate::task::spawn_task(beat_id, input, vec![], tx);
        self.task_handle = Some(handle);
    }

    fn handle_slash_command(&mut self, input: &str) {
        let cmd = input.trim();
        if let Some(name) = cmd.strip_prefix("/new ") {
            self.new_beat(name.trim());
        } else if cmd == "/new" {
            self.error = Some("Usage: /new {session name}".into());
        } else if cmd == "/cancel" {
            self.cancel_task();
        } else if cmd == "/clear" {
            self.transcript.clear();
        } else if cmd == "/compact" {
            self.send_slash_to_task("/compact".into());
        } else if let Some(wf) = cmd.strip_prefix("/workflow ") {
            self.send_slash_to_task(format!("/workflow {}", wf.trim()));
        } else if cmd == "/workflow" {
            self.error = Some("Usage: /workflow {name} — list with: pulse workflow list".into());
        } else {
            self.error = Some(format!("Unknown command: {cmd}"));
        }
    }

    fn send_slash_to_task(&mut self, command: String) {
        let beat_id = match self.active_beat_id {
            Some(id) => id,
            None => {
                self.error = Some("No active session.".into());
                return;
            }
        };
        if self.task_running {
            self.error = Some("A task is already running.".into());
            return;
        }
        self.transcript
            .push(TranscriptLine::System(command.clone()));
        self.task_running = true;
        let tx = self.event_tx.clone();
        let handle = crate::task::spawn_task(beat_id, command, vec![], tx);
        self.task_handle = Some(handle);
    }
}
