//! Chat view: sessions sidebar + scrolling transcript + input bar.

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{App, Mode, TranscriptLine};

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let main_h = if area.height > 2 { area.height - 1 } else { 0 };
    let main = Rect::new(area.x, area.y, area.width, main_h);
    let status = Rect::new(area.x, area.y + main_h, area.width, 1);

    // Split into sidebar + chat
    let sidebar_w = if app.mode == Mode::Sessions { 32 } else { 24 };
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(sidebar_w), Constraint::Min(1)])
        .split(main);

    render_sidebar(frame, app, chunks[0]);

    let chat = chunks[1];
    // Split chat into transcript + input bar
    let input_h = 3;
    let chat_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(input_h)])
        .split(chat);

    render_transcript(frame, app, chat_chunks[0]);
    render_input(frame, app, chat_chunks[1]);

    render_status(frame, app, status);
}

fn render_sidebar(frame: &mut Frame, app: &App, area: Rect) {
    let title = if app.mode == Mode::Sessions {
        " Sessions (j/k, Enter) "
    } else {
        " Sessions "
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(if app.mode == Mode::Sessions {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        });

    let items: Vec<ListItem> = app
        .beats
        .iter()
        .map(|b| {
            let prefix = if Some(b.id) == app.active_beat_id {
                ">"
            } else if b.archived {
                "~"
            } else {
                " "
            };
            let name = format!(" {} {}", prefix, b.name);
            let style = if Some(b.id) == app.active_beat_id {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else if b.archived {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default()
            };
            ListItem::new(name).style(style)
        })
        .collect();

    if app.mode == Mode::Sessions {
        let mut state = ListState::default();
        state.select(Some(
            app.selected_session.min(app.beats.len().saturating_sub(1)),
        ));
        frame.render_stateful_widget(List::new(items).block(block), area, &mut state);
    } else {
        frame.render_widget(List::new(items).block(block), area);
    }
}

fn render_transcript(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Chat ")
        .border_style(Style::default().fg(Color::DarkGray));

    let lines = build_transcript_lines(app);
    let p = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((app.scroll, 0));

    frame.render_widget(p, area);
}

fn build_transcript_lines(app: &App) -> Vec<Line<'_>> {
    let mut out = Vec::new();
    for line in &app.transcript {
        match line {
            TranscriptLine::User(text) => {
                out.push(Line::from(vec![
                    Span::styled(
                        "User: ".to_string(),
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(text),
                ]));
                out.push(Line::raw(""));
            }
            TranscriptLine::Assistant(text) => {
                out.push(Line::from(vec![
                    Span::styled(
                        "Assistant: ".to_string(),
                        Style::default()
                            .fg(Color::Blue)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(text),
                ]));
                out.push(Line::raw(""));
            }
            TranscriptLine::Tool {
                tool,
                arguments,
                result,
                error,
            } => {
                let indicator = if *error { "[!]" } else { "   " };
                let color = if *error { Color::Red } else { Color::Magenta };
                out.push(Line::from(vec![
                    Span::styled(format!("  {indicator} "), Style::default().fg(color)),
                    Span::styled(format!("[{tool}] "), Style::default().fg(color)),
                    Span::styled(arguments.clone(), Style::default().fg(Color::DarkGray)),
                ]));
                let truncated = truncate_str(result, 500);
                out.push(Line::styled(
                    format!("       {truncated}"),
                    Style::default().fg(if *error { Color::Red } else { Color::Gray }),
                ));
                out.push(Line::raw(""));
            }
            TranscriptLine::Step(text) => {
                out.push(Line::from(vec![
                    Span::styled("  → ", Style::default().fg(Color::DarkGray)),
                    Span::raw(text),
                ]));
            }
            TranscriptLine::System(text) => {
                out.push(Line::from(vec![
                    Span::styled("  ", Style::default()),
                    Span::styled(text.clone(), Style::default().fg(Color::DarkGray)),
                ]));
                out.push(Line::raw(""));
            }
            TranscriptLine::Error(text) => {
                out.push(Line::styled(
                    format!("Error: {text}"),
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ));
                out.push(Line::raw(""));
            }
        }
    }
    if app.task_running {
        out.push(Line::styled(
            "  (running...)",
            Style::default().fg(Color::Yellow),
        ));
    }
    out
}

fn render_input(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let prompt_text = if app.task_running {
        "(task running — Ctrl+K to cancel)".to_string()
    } else {
        format!("> {}", app.input)
    };
    let p = Paragraph::new(prompt_text)
        .block(block)
        .alignment(Alignment::Left);
    frame.render_widget(p, area);
}

fn render_status(frame: &mut Frame, app: &App, area: Rect) {
    let mode_str = app.mode.label();
    let mut parts = vec![Span::styled(
        format!(" [{mode_str}] "),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )];
    if !app.current_model.is_empty() {
        parts.push(Span::raw(format!(
            "Model: {} ({})  ",
            app.current_model, app.current_tier
        )));
    }
    if app.current_cost > 0.0 {
        parts.push(Span::raw(format!("Cost: ${:.4}  ", app.current_cost)));
    }
    if let Some(ctx) = app.current_context {
        parts.push(Span::raw(format!("Ctx: {:.0}%", ctx)));
    }
    parts.push(Span::raw("  Tab: views  ?: help  q: quit"));

    let line = Line::from(parts);
    let p = Paragraph::new(line).style(Style::default().fg(Color::DarkGray));
    frame.render_widget(p, area);
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max])
    }
}
