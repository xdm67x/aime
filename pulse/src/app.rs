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
    /// Rect of the confirmation popup, when open.
    pub confirm: Option<Rect>,
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

/// Byte offset of the `char_idx`-th character of `s`, or the end of `s` when
/// `char_idx` is past the last character.
pub fn char_to_byte(s: &str, char_idx: usize) -> usize {
    match s.char_indices().nth(char_idx) {
        Some((i, _)) => i,
        None => s.len(),
    }
}

/// Insert `c` at the cursor (a character index) and step the cursor forward.
pub fn insert_at_cursor(input: &mut String, cursor: &mut usize, c: char) {
    let pos = char_to_byte(input, *cursor);
    input.insert(pos, c);
    *cursor += 1;
}

/// Delete the character before the cursor. Returns false at the very start.
pub fn remove_before_cursor(input: &mut String, cursor: &mut usize) -> bool {
    if *cursor == 0 {
        return false;
    }
    *cursor -= 1;
    let pos = char_to_byte(input, *cursor);
    input.remove(pos);
    true
}

/// Move the cursor by `delta` characters, clamped to the input length.
pub fn move_cursor(input: &str, cursor: &mut usize, delta: i32) {
    let len = input.chars().count() as i32;
    *cursor = (*cursor as i32 + delta).clamp(0, len.max(0)) as usize;
}

/// Split a leading `@mention` off a prompt: returns (mention, rest). The
/// mention is empty when the input does not start with `@word`.
pub fn split_mention(input: &str) -> (&str, &str) {
    match input.strip_prefix('@') {
        Some(rest) => match rest.find(char::is_whitespace) {
            Some(i) => (&rest[..i], rest[i..].trim_start()),
            None => (rest, ""),
        },
        None => ("", input),
    }
}

/// Remove the `@mention` fragment being completed: from the last `@` before
/// the cursor up to the cursor. Any text typed before the `@` is kept, so a
/// prompt survives picking a project from the `@` popup.
pub fn remove_mention_fragment(input: &mut String, cursor: &mut usize) {
    let end = char_to_byte(input, *cursor);
    if let Some(pos) = input[..end].rfind('@') {
        let new_cursor = input[..pos].chars().count();
        input.replace_range(pos..end, "");
        *cursor = new_cursor;
    }
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

    #[test]
    fn split_mention_separates_project_and_prompt() {
        let (m, rest) = split_mention("@pulse fix the login flow");
        assert_eq!(m, "pulse");
        assert_eq!(rest, "fix the login flow");
        let (m, rest) = split_mention("@pulse");
        assert_eq!(m, "pulse");
        assert_eq!(rest, "");
        let (m, rest) = split_mention("no mention here");
        assert_eq!(m, "");
        assert_eq!(rest, "no mention here");
        let (m, rest) = split_mention("@");
        assert_eq!(m, "");
        assert_eq!(rest, "");
    }

    #[test]
    fn remove_mention_fragment_keeps_the_prompt() {
        // Typing "hey @pu" and picking a project keeps "hey ".
        let mut input = "hey @pu".to_string();
        let mut cursor = input.chars().count();
        remove_mention_fragment(&mut input, &mut cursor);
        assert_eq!(input, "hey ");
        assert_eq!(cursor, 4);

        // No `@`: nothing changes.
        let mut input = "plain prompt".to_string();
        let mut cursor = 5;
        remove_mention_fragment(&mut input, &mut cursor);
        assert_eq!(input, "plain prompt");
        assert_eq!(cursor, 5);

        // Multi-byte characters before the fragment keep the cursor valid.
        let mut input = "hé @proj".to_string();
        let mut cursor = input.chars().count();
        remove_mention_fragment(&mut input, &mut cursor);
        assert_eq!(input, "hé ");
        assert_eq!(cursor, 3);
    }

    #[test]
    fn input_editing_is_char_safe() {
        // Regression: byte-indexed editing panicked (is_char_boundary) as soon
        // as a multi-byte character was followed by another keypress.
        let mut input = String::new();
        let mut cursor = 0;
        insert_at_cursor(&mut input, &mut cursor, 'é'); // 2 bytes
        insert_at_cursor(&mut input, &mut cursor, 'a');
        insert_at_cursor(&mut input, &mut cursor, 'x');
        assert_eq!(input, "éax");
        assert_eq!(cursor, 3);

        // Cursor movement lands on character boundaries, never mid-byte.
        move_cursor(&input, &mut cursor, -3);
        assert_eq!(cursor, 0);
        insert_at_cursor(&mut input, &mut cursor, 'z');
        assert_eq!(input, "zéax");

        // Backspace removes whole characters.
        assert!(remove_before_cursor(&mut input, &mut cursor));
        assert_eq!(input, "éax");
        move_cursor(&input, &mut cursor, 10); // clamped to length
        assert_eq!(cursor, 3);
        assert!(remove_before_cursor(&mut input, &mut cursor));
        assert_eq!(input, "éa");
        assert_eq!(char_to_byte("éa", 1), 2);
        while remove_before_cursor(&mut input, &mut cursor) {}
        assert_eq!(input, "");
        assert!(!remove_before_cursor(&mut input, &mut cursor));
    }
}

pub struct App {
    pub mode: Mode,
    pub beats: Vec<Beat>,
    pub active_beat_id: Option<i64>,
    pub transcript: Vec<TranscriptLine>,
    pub input: String,
    /// Cursor position in the input, counted in characters (not bytes) so
    /// multi-byte characters can be typed and edited safely.
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
    /// Prompts typed while a task was running. The first one is sent when
    /// the current task finishes.
    pub queue: Vec<String>,
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
            queue: Vec::new(),
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
        let upto = &self.input[..char_to_byte(&self.input, self.input_cursor)];
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

    /// Attach a project picked via `@`: no session is created yet, and the
    /// input is preserved (callers strip only the `@` fragment). The beat
    /// (with its git worktree) is created when the next prompt is sent, and
    /// the prompt text names the session. The project itself is never part of
    /// the prompt — the session runs inside the project directory.
    pub fn attach_pending_project(&mut self, project_id: i64, name: &str) {
        self.pending_project = Some((project_id, name.to_string()));
        self.mode = Mode::Chat;
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

    /// Delete a beat and its worktree. When the active session is deleted,
    /// fall back to the first remaining one (or an empty view if none).
    pub fn delete_beat(&mut self, beat_id: i64) {
        match pulse_core::beats::delete_beat(beat_id) {
            Ok(_) => {
                if self.active_beat_id == Some(beat_id) {
                    self.active_beat_id = None;
                    self.pending_project = None;
                    self.transcript.clear();
                }
                self.refresh_beats();
                if self.active_beat_id.is_none() {
                    if let Some(beat) = self.beats.first() {
                        self.switch_beat(beat.id);
                    }
                }
                self.session_select(self.session_selected());
            }
            Err(e) => self.error = Some(e),
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

    /// Mark the task as finished and refresh state. If prompts were queued
    /// while it ran, send the first one.
    pub fn task_finished(&mut self) {
        self.task_running = false;
        self.task_handle = None;
        if let Some(id) = self.active_beat_id {
            self.load_transcript(id);
        }
        self.refresh_beats();
        if let Some(prompt) = self.queue.first().cloned() {
            self.queue.remove(0);
            self.dispatch_prompt(&prompt);
        }
    }

    /// Cancel the running task.
    pub fn cancel_task(&mut self) {
        if let Some(id) = self.active_beat_id {
            pulse_core::harness::cancel_current(id);
        }
    }

    /// Resolve a typed `@mention` to a project: exact (case-insensitive)
    /// name match, else a unique prefix match. Returns (id, name).
    pub fn find_project_by_mention(&self, mention: &str) -> Option<(i64, String)> {
        let m = mention.to_lowercase();
        if let Some(p) = self.projects.iter().find(|p| p.name.to_lowercase() == m) {
            return Some((p.id, p.name.clone()));
        }
        let mut prefix = self
            .projects
            .iter()
            .filter(|p| p.name.to_lowercase().starts_with(&m));
        prefix
            .next()
            .filter(|_| prefix.next().is_none())
            .map(|p| (p.id, p.name.clone()))
    }

    /// Send the current input as a prompt. While a task runs, plain prompts
    /// are queued instead and sent when it finishes.
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

        if self.task_running {
            self.queue.push(input);
            self.input.clear();
            self.input_cursor = 0;
            return;
        }

        self.dispatch_prompt(&input);
    }

    /// Process a prompt that is ready to run now: resolve a leading `@project`
    /// mention, create the pending project session if any, and start the task.
    fn dispatch_prompt(&mut self, input: &str) {
        // A leading `@project` mention binds the next session to that project
        // and is never part of the prompt sent to the model.
        let mut prompt = input.to_string();
        let (mention, rest) = split_mention(input);
        if !mention.is_empty() {
            match self.find_project_by_mention(mention) {
                Some((id, name)) => {
                    if rest.is_empty() {
                        // Just the mention: attach and wait for the prompt.
                        self.attach_pending_project(id, &name);
                        self.input.clear();
                        self.input_cursor = 0;
                        return;
                    }
                    prompt = rest.to_string();
                    self.pending_project = Some((id, name));
                }
                None => {
                    self.error = Some(format!("Unknown project: @{mention}"));
                    self.restore_input(input);
                    return;
                }
            }
        }

        // A project picked via `@` (popup or mention): create the session (and
        // its worktree) now, named after the prompt.
        if let Some((project_id, _)) = self.pending_project.take() {
            let name = session_name_from_prompt(&prompt);
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
                self.restore_input(input);
                return;
            }
        };

        self.input.clear();
        self.input_cursor = 0;
        self.transcript.push(TranscriptLine::User(prompt.clone()));
        self.task_running = true;
        let tx = self.event_tx.clone();
        let handle = crate::task::spawn_task(beat_id, prompt, vec![], tx);
        self.task_handle = Some(handle);
    }

    /// Put a prompt that failed to dispatch back into the input bar so it is
    /// not lost. No-op when the bar already holds text (the interactive path
    /// keeps it there).
    fn restore_input(&mut self, text: &str) {
        if self.input.is_empty() {
            self.input = text.to_string();
            self.input_cursor = self.input.chars().count();
        }
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
