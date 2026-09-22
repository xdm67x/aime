//! Render dispatch: chat view (with sessions sidebar) + popup overlays.

pub mod chat;
pub mod markdown;

use crate::app::App;
use ratatui::style::Stylize;
use ratatui::Frame;

pub fn render(frame: &mut Frame, app: &mut App) {
    chat::render(frame, app);

    // Popups overlay on top of any view
    if app.show_help {
        render_help(frame);
    }
    if let Some(ref err) = app.error {
        render_error(frame, err);
    }
}

fn render_help(frame: &mut Frame) {
    let area = frame.area();
    let w = 54.min(area.width);
    let h = 28.min(area.height.saturating_sub(1));
    let x = area.x + (area.width - w) / 2;
    let y = area.y + (area.height - h) / 2;
    let popup = ratatui::layout::Rect::new(x, y, w, h);

    let text = vec![
        ratatui::text::Line::raw("Pulse — Key Bindings").bold(),
        ratatui::text::Line::raw(""),
        ratatui::text::Line::raw("Global:"),
        ratatui::text::Line::raw("  Ctrl+C   Quit (works everywhere)"),
        ratatui::text::Line::raw("  q        Quit (from sessions)"),
        ratatui::text::Line::raw("  ?        Toggle this help (from sessions)"),
        ratatui::text::Line::raw(""),
        ratatui::text::Line::raw("Chat:"),
        ratatui::text::Line::raw("  Enter     Send prompt"),
        ratatui::text::Line::raw("  Left      Open sessions (cursor at start)"),
        ratatui::text::Line::raw("  @         Project picker (new project session)"),
        ratatui::text::Line::raw("  Ctrl+K    Cancel running task"),
        ratatui::text::Line::raw("  Ctrl+L    Clear transcript"),
        ratatui::text::Line::raw("  Ctrl+R    Refresh sessions"),
        ratatui::text::Line::raw(""),
        ratatui::text::Line::raw("Sessions (Tab or Left to open):"),
        ratatui::text::Line::raw("  j/k       Move  Enter: switch session"),
        ratatui::text::Line::raw("  a         Archive  d: delete archived"),
        ratatui::text::Line::raw("  Right/Esc Back to chat"),
        ratatui::text::Line::raw(""),
        ratatui::text::Line::raw("Mouse:"),
        ratatui::text::Line::raw("  click session     Switch to it"),
        ratatui::text::Line::raw("  click tool line   Show/hide its output"),
        ratatui::text::Line::raw("  wheel             Scroll chat / move sessions"),
        ratatui::text::Line::raw(""),
        ratatui::text::Line::raw("Slash commands:"),
        ratatui::text::Line::raw("  /new {name}     /workflow {name}"),
        ratatui::text::Line::raw("  /compact        /cancel  /clear"),
    ];

    let block = ratatui::widgets::Block::default()
        .borders(ratatui::widgets::Borders::ALL)
        .title(" Help ")
        .border_style(ratatui::style::Style::default().fg(ratatui::style::Color::Cyan));

    let p = ratatui::widgets::Paragraph::new(text)
        .block(block)
        .alignment(ratatui::layout::Alignment::Left);
    frame.render_widget(ratatui::widgets::Clear, popup);
    frame.render_widget(p, popup);
}

fn render_error(frame: &mut Frame, msg: &str) {
    let area = frame.area();
    let w = 60.min(area.width);
    let h = 5;
    let x = area.x + (area.width - w) / 2;
    let y = area.y + area.height.saturating_sub(h).saturating_sub(2);
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
    frame.render_widget(ratatui::widgets::Clear, popup);
    frame.render_widget(p, popup);
}
