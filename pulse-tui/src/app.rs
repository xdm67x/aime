//! App state, mode switching, and action dispatch.

use pulse_core::beats::Beat;
use pulse_core::config::ModelConfig;
use pulse_core::harness::{TaggedEvent, TaskEvent};
use pulse_core::projects::Project;
use pulse_core::workflows::Workflow;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Chat,
    Sessions,
    Projects,
    Settings,
    Workflows,
}

impl Mode {
    pub fn next(self) -> Self {
        match self {
            Mode::Chat => Mode::Sessions,
            Mode::Sessions => Mode::Projects,
            Mode::Projects => Mode::Settings,
            Mode::Settings => Mode::Workflows,
            Mode::Workflows => Mode::Chat,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Mode::Chat => "Chat",
            Mode::Sessions => "Sessions",
            Mode::Projects => "Projects",
            Mode::Settings => "Settings",
            Mode::Workflows => "Workflows",
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

pub struct App {
    pub mode: Mode,
    pub beats: Vec<Beat>,
    pub active_beat_id: Option<i64>,
    pub transcript: Vec<TranscriptLine>,
    pub input: String,
    pub input_cursor: usize,
    pub projects: Vec<Project>,
    pub workflows: Vec<Workflow>,
    pub model_config: ModelConfig,
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
    pub selected_session: usize,
    pub selected_project: usize,
    pub selected_workflow: usize,
    pub settings_field: usize,
    pub settings_input: String,
    pub settings_editing: bool,
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
        let workflows = pulse_core::workflows::discover().unwrap_or_default();
        let model_config = pulse_core::config::ModelConfig::load().unwrap_or_default();
        let active_beat_id = beats.first().map(|b| b.id);

        let mut app = Self {
            mode: Mode::Chat,
            beats,
            active_beat_id,
            transcript: vec![],
            input: String::new(),
            input_cursor: 0,
            projects,
            workflows,
            model_config,
            running: true,
            task_running: false,
            error: None,
            event_tx: tx,
            event_rx: rx,
            task_handle: None,
            scroll: 0,
            popup: None,
            show_help: false,
            selected_session: 0,
            selected_project: 0,
            selected_workflow: 0,
            settings_field: 0,
            settings_input: String::new(),
            settings_editing: false,
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

    pub fn refresh_workflows(&mut self) {
        self.workflows = pulse_core::workflows::discover().unwrap_or_default();
    }

    /// Load the persisted transcript for a beat into `TranscriptLine`s.
    pub fn load_transcript(&mut self, beat_id: i64) {
        self.transcript.clear();
        self.scroll = 0;
        self.current_cost = 0.0;
        self.current_context = None;
        self.current_model.clear();
        self.current_tier.clear();
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
        self.input.clear();
        self.input_cursor = 0;

        // Handle slash commands
        if input.starts_with('/') {
            self.handle_slash_command(&input);
            return;
        }

        let beat_id = match self.active_beat_id {
            Some(id) => id,
            None => {
                self.error = Some("No active session. Create one with /new {name}.".into());
                return;
            }
        };

        if self.task_running {
            self.error = Some("A task is already running. Cancel with Ctrl+K.".into());
            return;
        }

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
            self.mode = Mode::Workflows;
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
