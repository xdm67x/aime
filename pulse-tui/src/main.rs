//! Pulse TUI — terminal entry point, event loop, key dispatch.

mod app;
mod event;
mod task;
mod ui;

use app::{App, Mode};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::stdout;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Terminal setup
    stdout().execute(EnterAlternateScreen)?;
    terminal::enable_raw_mode()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    terminal.clear()?;

    // Spawn models refresh loop
    tokio::spawn(pulse_core::providers::refresh_loop());

    let mut app = App::new();

    // Main loop
    while app.running {
        // Drain async task events
        app.drain_events();

        // Check if task finished
        if app.task_running {
            if let Some(ref handle) = app.task_handle {
                if handle.is_finished() {
                    app.task_finished();
                }
            }
        }

        // Render
        terminal.draw(|frame| {
            ui::render(frame, &mut app);
        })?;

        // Poll input (100ms timeout so we keep draining events)
        if let Some(key) = event::poll(100) {
            handle_key(&mut app, key);
        }
    }

    // Terminal teardown
    terminal::disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}

fn handle_key(app: &mut App, key: KeyEvent) {
    // Global keys work in any mode (unless editing)
    if app.settings_editing {
        handle_settings_edit(app, key);
        return;
    }

    // Popup handling
    if app.show_help {
        match key.code {
            KeyCode::Char('?') | KeyCode::Esc => app.show_help = false,
            _ => {}
        }
        return;
    }
    if app.error.is_some() {
        if key.code == KeyCode::Esc {
            app.error = None;
        }
        return;
    }

    // Global keys
    match key.code {
        KeyCode::Tab => {
            app.mode = app.mode.next();
            return;
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.running = false;
            return;
        }
        KeyCode::Char('q') => {
            if !app.task_running {
                app.running = false;
                return;
            }
        }
        KeyCode::Char('?') => {
            app.show_help = !app.show_help;
            return;
        }
        KeyCode::Esc => {
            app.mode = Mode::Chat;
            return;
        }
        _ => {}
    }

    // Mode-specific keys
    match app.mode {
        Mode::Chat => handle_chat_key(app, key),
        Mode::Sessions => handle_sessions_key(app, key),
        Mode::Projects => handle_projects_key(app, key),
        Mode::Settings => handle_settings_key(app, key),
        Mode::Workflows => handle_workflows_key(app, key),
    }
}

fn handle_chat_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Enter => {
            app.send_input();
        }
        KeyCode::Backspace => {
            if app.input_cursor > 0 {
                app.input_cursor -= 1;
                app.input.remove(app.input_cursor);
            }
        }
        KeyCode::Left => {
            if app.input_cursor > 0 {
                app.input_cursor -= 1;
            }
        }
        KeyCode::Right => {
            if app.input_cursor < app.input.len() {
                app.input_cursor += 1;
            }
        }
        KeyCode::Up => {
            if app.scroll > 0 {
                app.scroll -= 1;
            }
        }
        KeyCode::Down => {
            app.scroll += 1;
        }
        KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.cancel_task();
        }
        KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.transcript.clear();
        }
        KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.refresh_beats();
        }
        KeyCode::Char(c) => {
            app.input.insert(app.input_cursor, c);
            app.input_cursor += 1;
        }
        _ => {}
    }
}

fn handle_sessions_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            if app.selected_session + 1 < app.beats.len() {
                app.selected_session += 1;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if app.selected_session > 0 {
                app.selected_session -= 1;
            }
        }
        KeyCode::Enter => {
            if let Some(beat) = app.beats.get(app.selected_session) {
                app.switch_beat(beat.id);
            }
        }
        KeyCode::Char('a') => {
            if let Some(beat) = app.beats.get(app.selected_session) {
                let _ = pulse_core::beats::set_beat_archived(beat.id, !beat.archived);
                app.refresh_beats();
            }
        }
        KeyCode::Char('d') => {
            if let Some(beat) = app.beats.get(app.selected_session) {
                if beat.archived {
                    let _ = pulse_core::beats::delete_beat(beat.id);
                    app.refresh_beats();
                    if app.selected_session > 0 {
                        app.selected_session -= 1;
                    }
                }
            }
        }
        _ => {}
    }
}

fn handle_projects_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            if app.selected_project + 1 < app.projects.len() {
                app.selected_project += 1;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if app.selected_project > 0 {
                app.selected_project -= 1;
            }
        }
        KeyCode::Char('r') => {
            if let Some(project) = app.projects.get(app.selected_project) {
                let _ = pulse_core::projects::remove_project(project.id);
                app.refresh_projects();
                if app.selected_project > 0 {
                    app.selected_project -= 1;
                }
            }
        }
        _ => {}
    }
}

fn handle_settings_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            if app.settings_field < 6 {
                app.settings_field += 1;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if app.settings_field > 0 {
                app.settings_field -= 1;
            }
        }
        KeyCode::Enter => {
            // Start editing the selected field
            app.settings_editing = true;
            app.settings_input.clear();
            // Pre-fill with current value for model fields
            match app.settings_field {
                3 => app.settings_input = app.model_config.classifier.clone(),
                4 => app.settings_input = app.model_config.high.clone(),
                5 => app.settings_input = app.model_config.base.clone(),
                6 => app.settings_input = app.model_config.low.clone(),
                _ => {}
            }
        }
        _ => {}
    }
}

fn handle_settings_edit(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.settings_editing = false;
            app.settings_input.clear();
        }
        KeyCode::Enter => {
            let value = app.settings_input.trim().to_string();
            match app.settings_field {
                0 => {
                    let _ = pulse_core::config::save_api_key("openrouter", &value);
                }
                1 => {
                    let _ = pulse_core::config::save_api_key("opencode", &value);
                }
                2 => {
                    let _ = pulse_core::config::save_api_key("litellm", &value);
                }
                3 => app.model_config.classifier = value,
                4 => app.model_config.high = value,
                5 => app.model_config.base = value,
                6 => app.model_config.low = value,
                _ => {}
            }
            if app.settings_field >= 3 {
                let _ = pulse_core::config::save_model_config(&app.model_config);
            }
            app.settings_editing = false;
            app.settings_input.clear();
        }
        KeyCode::Backspace => {
            if !app.settings_input.is_empty() {
                app.settings_input.pop();
            }
        }
        KeyCode::Char(c) => {
            app.settings_input.push(c);
        }
        _ => {}
    }
}

fn handle_workflows_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            if app.selected_workflow + 1 < app.workflows.len() {
                app.selected_workflow += 1;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if app.selected_workflow > 0 {
                app.selected_workflow -= 1;
            }
        }
        KeyCode::Enter => {
            // Run the selected workflow on the active beat
            if let Some(wf) = app.workflows.get(app.selected_workflow) {
                if let Some(beat_id) = app.active_beat_id {
                    if !app.task_running {
                        let cmd = format!("/workflow {}", wf.name);
                        app.transcript
                            .push(app::TranscriptLine::System(cmd.clone()));
                        app.task_running = true;
                        let tx = app.event_tx.clone();
                        let handle = task::spawn_task(beat_id, cmd, vec![], tx);
                        app.task_handle = Some(handle);
                        app.mode = Mode::Chat;
                    } else {
                        app.error = Some("A task is already running.".into());
                    }
                } else {
                    app.error = Some("No active session.".into());
                }
            }
        }
        KeyCode::Char('r') => {
            app.refresh_workflows();
        }
        KeyCode::Char('e') => {
            // Open in $EDITOR
            if let Some(wf) = app.workflows.get(app.selected_workflow) {
                let home = std::env::var("HOME").unwrap_or_default();
                let dir = format!("{home}/.pulse/workflows");
                for ext in &["yml", "yaml"] {
                    let path = format!("{dir}/{}.{}", wf.name, ext);
                    if std::path::Path::new(&path).is_file() {
                        let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vim".into());
                        let _ = std::process::Command::new(&editor).arg(&path).status();
                        app.refresh_workflows();
                        break;
                    }
                }
            }
        }
        KeyCode::Char('n') => {
            // Create a new workflow template
            let home = std::env::var("HOME").unwrap_or_default();
            let dir = format!("{home}/.pulse/workflows");
            let _ = std::fs::create_dir_all(&dir);
            let name = format!("workflow-{}", app.workflows.len() + 1);
            let path = format!("{dir}/{}.yml", name);
            let template = format!(
                "name: {name}\n\
                 description: A new workflow\n\
                 steps:\n\
                 - name: step1\n\
                   prompt: |\n\
                   Do something useful.\n"
            );
            if std::fs::write(&path, template).is_ok() {
                let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vim".into());
                let _ = std::process::Command::new(&editor).arg(&path).status();
                app.refresh_workflows();
            }
        }
        _ => {}
    }
}
