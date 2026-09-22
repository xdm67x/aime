//! Render dispatch: routes to the active view.

pub mod chat;
pub mod sessions;
pub mod projects;
pub mod settings;
pub mod workflows;

use ratatui::Frame;
use ratatui::style::Stylize;
use crate::app::App;

pub fn render(frame: &mut Frame, app: &mut App) {
    match app.mode {
        Mode::Chat => {
            chat::render(frame, app);
        }
        Mode::Sessions => {
            sessions::render(frame, app);
        }
        Mode::Projects => {
            projects::render(frame, app);
        }
        Mode::Settings => {
            settings::render(frame, app);
        }
        Mode::Workflows => {
            workflows::render(frame, app);
        }
    }

    // Popups overlay on top of any view
    if app.show_help {
        render_help(frame);
    }
    if let Some(ref err) = app.error {
        render_error(frame, err);
    }
}

use crate::app::Mode;

fn render_help(frame: &mut Frame) {
    let area = frame.area();
    let w = 50.min(area.width);
    let h = 18;
    let x = area.x + (area.width - w) / 2;
    let y = area.y + (area.height - h) / 2;
    let popup = ratatui::layout::Rect::new(x, y, w, h);

    let text = vec![
        ratatui::text::Line::raw("Pulse TUI — Key Bindings").bold(),
        ratatui::text::Line::raw(""),
        ratatui::text::Line::raw("Global:"),
        ratatui::text::Line::raw("  Tab       Cycle views"),
        ratatui::text::Line::raw("  q/Ctrl+C  Quit"),
        ratatui::text::Line::raw("  Esc       Close popup / back to chat"),
        ratatui::text::Line::raw("  ?         Toggle this help"),
        ratatui::text::Line::raw(""),
        ratatui::text::Line::raw("Chat:"),
        ratatui::text::Line::raw("  Enter     Send prompt"),
        ratatui::text::Line::raw("  Ctrl+K    Cancel running task"),
        ratatui::text::Line::raw("  Ctrl+L    Clear transcript"),
        ratatui::text::Line::raw("  Ctrl+R    Refresh sessions"),
        ratatui::text::Line::raw(""),
        ratatui::text::Line::raw("Slash commands:"),
        ratatui::text::Line::raw("  /new {name}     /workflow {name}"),
        ratatui::text::Line::raw("  /compact        /cancel  /clear"),
        ratatui::text::Line::raw(""),
        ratatui::text::Line::raw("Press ? or Esc to close."),
    ];

    let block = ratatui::widgets::Block::default()
        .borders(ratatui::widgets::Borders::ALL)
        .title(" Help ")
        .border_style(ratatui::style::Style::default().fg(ratatui::style::Color::Cyan));

    let p = ratatui::widgets::Paragraph::new(text)
        .block(block)
        .alignment(ratatui::layout::Alignment::Left);
    frame.render_widget(p, popup);
}

fn render_error(frame: &mut Frame, msg: &str) {
    let area = frame.area();
    let w = 60.min(area.width);
    let h = 5;
    let x = area.x + (area.width - w) / 2;
    let y = area.y + (area.height - h) - 2;
    let popup = ratatui::layout::Rect::new(x, y, w, h);

    let block = ratatui::widgets::Block::default()
        .borders(ratatui::widgets::Borders::ALL)
        .title(" Error ")
        .border_style(ratatui::style::Style::default().fg(ratatui::style::Color::Red));

    let text = ratatui::text::Text::from(vec![
        ratatui::text::Line::raw(msg),
        ratatui::text::Line::raw(""),
        ratatui::text::Line::raw("Press Esc to dismiss."),
    ]);
    let p = ratatui::widgets::Paragraph::new(text)
        .block(block)
        .alignment(ratatui::layout::Alignment::Center);
    frame.render_widget(p, popup);
}
