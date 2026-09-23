//! Pulse — terminal app. With no subcommand it launches the TUI; workflow
//! management and release updates run as CLI subcommands before any terminal
//! setup. Settings (API keys, model slots) live in the TUI slash commands.

mod app;
mod cli;
mod event;
mod task;
mod ui;

use app::{App, Mode};
use clap::Parser;
use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers, MouseButton,
    MouseEvent, MouseEventKind,
};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::stdout;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = cli::Cli::parse();
    if let Some(command) = cli.command {
        pulse_core::log::info(format!(
            "pulse {} (CLI): {}",
            env!("CARGO_PKG_VERSION"),
            command.label()
        ));
        if let Err(e) = cli::run(command).await {
            pulse_core::log::error(format!("cli command failed: {e}"));
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    run_tui().await
}

async fn run_tui() -> Result<(), Box<dyn std::error::Error>> {
    pulse_core::log::info(format!(
        "pulse {} starting (TUI)",
        env!("CARGO_PKG_VERSION")
    ));
    // Terminal setup
    stdout().execute(EnterAlternateScreen)?;
    terminal::enable_raw_mode()?;
    stdout().execute(EnableMouseCapture)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    terminal.clear()?;

    // Spawn models refresh loop
    tokio::spawn(pulse_core::providers::refresh_loop());

    let mut app = App::new();

    // Check for a newer release at startup; a failed check only logs.
    if let Some(notice) = pulse_core::update::startup_notice(env!("CARGO_PKG_VERSION")).await {
        app.transcript.push(app::TranscriptLine::System(notice));
    }

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

        // Check if a repo clone finished
        if let Some(handle) = app.clone_handle.take_if(|h| h.is_finished()) {
            match handle.await {
                Ok(Ok(project)) => {
                    pulse_core::log::info(format!(
                        "clone finished: project {} at {}",
                        project.name, project.path
                    ));
                    app.refresh_projects();
                    app.attach_pending_project(project.id, &project.name);
                }
                Ok(Err(e)) => {
                    pulse_core::log::error(format!("clone failed: {e}"));
                    app.error = Some(format!("Clone failed: {e}"));
                }
                Err(e) => {
                    pulse_core::log::error(format!("clone task failed: {e}"));
                    app.error = Some(format!("Clone task failed: {e}"));
                }
            }
        }

        // Render
        terminal.draw(|frame| {
            ui::render(frame, &mut app);
        })?;

        // Poll input (100ms timeout so we keep draining events)
        if let Some(ev) = event::poll(100) {
            match ev {
                Event::Key(key) if key.kind == crossterm::event::KeyEventKind::Press => {
                    handle_key(&mut app, key)
                }
                Event::Mouse(mouse) => handle_mouse(&mut app, mouse),
                _ => {}
            }
        }
    }

    // Terminal teardown
    terminal::disable_raw_mode()?;
    stdout().execute(DisableMouseCapture)?;
    stdout().execute(LeaveAlternateScreen)?;
    pulse_core::log::info("TUI exited cleanly");
    Ok(())
}

fn handle_key(app: &mut App, key: KeyEvent) {
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

    // The confirmation popup captures all keys
    if app.popup.is_some() {
        handle_confirm_key(app, key);
        return;
    }

    // Input popup (add local project / clone GitHub repo) captures all keys
    if app.input_popup.is_some() {
        handle_input_popup_key(app, key);
        return;
    }

    // The `@` project popup captures typing in chat
    if app.mode == Mode::Chat && app.at_popup.is_some() {
        handle_at_popup_key(app, key);
        return;
    }

    // Global keys. In Chat mode the input owns plain characters (so prompts
    // can contain 'q' or '?'); they only act globally in Sessions mode.
    match key.code {
        KeyCode::Tab => {
            app.mode = if app.mode == Mode::Chat {
                Mode::Sessions
            } else {
                Mode::Chat
            };
            return;
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.running = false;
            return;
        }
        KeyCode::Char('q') if app.mode == Mode::Sessions => {
            if !app.task_running {
                app.running = false;
                return;
            }
        }
        KeyCode::Char('?') if app.mode == Mode::Sessions => {
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
    }
}

fn handle_confirm_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Enter => {
            if let Some(app::Popup::Confirm(_, app::ConfirmAction::DeleteBeat(id))) =
                app.popup.take()
            {
                app.delete_beat(id);
            }
        }
        KeyCode::Esc => app.popup = None,
        _ => {}
    }
}

fn handle_chat_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Enter => {
            app.send_input();
        }
        KeyCode::Backspace => {
            app::remove_before_cursor(&mut app.input, &mut app.input_cursor);
        }
        KeyCode::Left => {
            if app.input_cursor == 0 {
                app.mode = Mode::Sessions;
            } else {
                app::move_cursor(&app.input, &mut app.input_cursor, -1);
            }
        }
        KeyCode::Right => {
            app::move_cursor(&app.input, &mut app.input_cursor, 1);
        }
        KeyCode::Up => {
            app.follow = false;
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
        KeyCode::Char('@') => {
            app::insert_at_cursor(&mut app.input, &mut app.input_cursor, '@');
            app.refresh_projects();
            app.at_popup = Some(app::AtPopup::Projects {
                filter: String::new(),
                selected: 0,
            });
        }
        KeyCode::Char(c) => {
            app::insert_at_cursor(&mut app.input, &mut app.input_cursor, c);
        }
        _ => {}
    }
}

fn handle_at_popup_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            // Close the popup, keep the raw `@` text in the input.
            app.at_popup = None;
        }
        KeyCode::Down => app.at_move(1),
        KeyCode::Up => app.at_move(-1),
        KeyCode::Enter => select_at_entry(app),
        KeyCode::Backspace => {
            if app::remove_before_cursor(&mut app.input, &mut app.input_cursor) {
                app.update_at_popup();
            }
        }
        KeyCode::Char(c) => {
            app::insert_at_cursor(&mut app.input, &mut app.input_cursor, c);
            app.update_at_popup();
        }
        _ => {}
    }
}

fn select_at_entry(app: &mut App) {
    enum Selection {
        Project(i64, String),
        AddPath,
        CloneRepo,
    }
    let selection = match &app.at_popup {
        Some(app::AtPopup::Projects { filter, selected }) => {
            let matching = app.matching_projects(filter);
            if *selected < matching.len() {
                let p = matching[*selected];
                Selection::Project(p.id, p.name.clone())
            } else if *selected == matching.len() {
                Selection::AddPath
            } else {
                Selection::CloneRepo
            }
        }
        None => return,
    };
    app.at_popup = None;
    // Keep whatever the user typed around the `@` fragment: the prompt stays
    // in the input and the session is only created when it is sent.
    app::remove_mention_fragment(&mut app.input, &mut app.input_cursor);

    match selection {
        Selection::Project(id, name) => app.attach_pending_project(id, &name),
        Selection::AddPath => {
            app.input_popup_text.clear();
            app.input_popup = Some(app::InputPopup::AddPath);
        }
        Selection::CloneRepo => {
            app.input_popup_text.clear();
            app.input_popup = Some(app::InputPopup::CloneRepo);
        }
    }
}

fn handle_input_popup_key(app: &mut App, key: KeyEvent) {
    let popup = match app.input_popup {
        Some(p) => p,
        None => return,
    };
    match key.code {
        KeyCode::Esc => {
            app.input_popup = None;
            app.input_popup_text.clear();
        }
        KeyCode::Backspace => {
            app.input_popup_text.pop();
        }
        KeyCode::Enter => {
            let value = app.input_popup_text.trim().to_string();
            app.input_popup = None;
            app.input_popup_text.clear();
            if value.is_empty() {
                return;
            }
            match popup {
                app::InputPopup::AddPath => match pulse_core::projects::add_project(&value) {
                    Ok(project) => {
                        app.refresh_projects();
                        app.attach_pending_project(project.id, &project.name);
                    }
                    Err(e) => app.error = Some(e),
                },
                app::InputPopup::CloneRepo => {
                    // Cloning takes seconds — run it in the background and
                    // render "Cloning…" until the main loop picks it up.
                    app.clone_handle = Some(tokio::spawn(async move {
                        pulse_core::projects::clone_project(&value).await
                    }));
                }
            }
        }
        KeyCode::Char(c) => {
            app.input_popup_text.push(c);
        }
        _ => {}
    }
}

fn handle_sessions_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Down => app.session_move(1),
        KeyCode::Up => app.session_move(-1),
        KeyCode::Right => {
            app.mode = Mode::Chat;
        }
        KeyCode::Enter => {
            if let Some(beat) = app.beats.get(app.session_selected()) {
                app.switch_beat(beat.id);
            }
        }
        KeyCode::Char('a') => {
            if let Some(beat) = app.beats.get(app.session_selected()) {
                let _ = pulse_core::beats::set_beat_archived(beat.id, !beat.archived);
                app.refresh_beats();
            }
        }
        KeyCode::Char('d') => {
            if let Some(beat) = app.beats.get(app.session_selected()) {
                if Some(beat.id) == app.active_beat_id && app.task_running {
                    app.error =
                        Some("Cancel the running task before deleting this session.".into());
                } else {
                    app.popup = Some(app::Popup::Confirm(
                        format!("Delete \"{}\"? Its worktree is removed too.", beat.name),
                        app::ConfirmAction::DeleteBeat(beat.id),
                    ));
                }
            }
        }
        _ => {}
    }
}

fn handle_mouse(app: &mut App, mouse: MouseEvent) {
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => handle_click(app, mouse.column, mouse.row),
        MouseEventKind::ScrollUp => handle_scroll(app, mouse.column, mouse.row, -1),
        MouseEventKind::ScrollDown => handle_scroll(app, mouse.column, mouse.row, 1),
        _ => {}
    }
}

fn handle_click(app: &mut App, x: u16, y: u16) {
    // Overlays first: they sit on top of everything else.
    if app.show_help {
        app.show_help = false;
        return;
    }
    if app.error.is_some() {
        app.error = None;
        return;
    }
    // Confirmation popup: click inside to confirm, outside to cancel.
    if let Some(rect) = app.rects.confirm {
        if app::UiRects::contains(rect, x, y) {
            if let Some(app::Popup::Confirm(_, app::ConfirmAction::DeleteBeat(id))) =
                app.popup.take()
            {
                app.delete_beat(id);
            }
        } else {
            app.popup = None;
        }
        return;
    }
    if let Some(rect) = app.rects.input_popup {
        if !app::UiRects::contains(rect, x, y) {
            app.input_popup = None;
            app.input_popup_text.clear();
        }
        return;
    }
    if let Some(rect) = app.rects.at_popup {
        if app::UiRects::contains(rect, x, y) {
            // Row inside the popup (minus its top border), clamped to entries.
            let idx = (y - rect.y).saturating_sub(1) as usize;
            if idx >= app.at_entries() {
                return; // clicked the bottom border or empty space
            }
            if let Some(app::AtPopup::Projects { selected, .. }) = &mut app.at_popup {
                *selected = idx;
            }
            select_at_entry(app);
        } else {
            app.at_popup = None;
        }
        return;
    }

    // Sessions sidebar: click a row to select it and switch.
    if let Some(rect) = app.rects.sidebar {
        if app::UiRects::contains(rect, x, y) {
            let idx = app.session_offset() + (y - rect.y) as usize;
            app.session_select(idx);
            if let Some(beat) = app.beats.get(app.session_selected()) {
                app.switch_beat(beat.id);
            }
            return;
        }
    }

    // Transcript: click a tool-call entry to show/hide its output.
    if let Some(rect) = app.rects.transcript {
        if app::UiRects::contains(rect, x, y) {
            let content_row = (y - rect.y) as usize + app.scroll as usize;
            if let Some(idx) = app.tool_at_row(content_row) {
                app.toggle_tool(idx);
            } else {
                app.mode = Mode::Chat;
            }
            return;
        }
    }

    // Anywhere else in the chat: focus the input.
    app.mode = Mode::Chat;
}

fn handle_scroll(app: &mut App, x: u16, y: u16, delta: i32) {
    // Over the sidebar, the wheel moves the session selection.
    if let Some(rect) = app.rects.sidebar {
        if app::UiRects::contains(rect, x, y) {
            app.session_move(delta);
            return;
        }
    }
    // Everywhere else it scrolls the transcript.
    if delta < 0 {
        app.follow = false;
        app.scroll = app.scroll.saturating_sub(1);
    } else {
        app.scroll = app.scroll.saturating_add(1);
    }
}
