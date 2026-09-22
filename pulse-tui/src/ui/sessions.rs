//! Sessions view — renders the sidebar as a full-width beat list.
//! Navigation: j/k to move, Enter to switch, a to archive, d to delete.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::app::App;

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let main_h = if area.height > 1 { area.height - 1 } else { 0 };
    let main = Rect::new(area.x, area.y, area.width, main_h);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Sessions (j/k: navigate, Enter: switch, a: archive, d: delete) ")
        .border_style(Style::default().fg(Color::Cyan));

    let items: Vec<ListItem> = app
        .beats
        .iter()
        .map(|b| {
            let prefix = if Some(b.id) == app.active_beat_id {
                ">"
            } else {
                " "
            };
            let archived = if b.archived { " [archived]" } else { "" };
            let cost = if b.cost_usd > 0.0 {
                format!("  ${:.4}", b.cost_usd)
            } else {
                String::new()
            };
            let line = Line::from(vec![
                Span::raw(format!("{prefix} ")),
                Span::styled(
                    b.name.clone(),
                    if Some(b.id) == app.active_beat_id {
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    },
                ),
                Span::styled(archived.to_string(), Style::default().fg(Color::DarkGray)),
                Span::styled(cost, Style::default().fg(Color::DarkGray)),
            ]);
            ListItem::new(line)
        })
        .collect();

    let mut state = ListState::default();
    state.select(Some(
        app.selected_session.min(app.beats.len().saturating_sub(1)),
    ));
    frame.render_stateful_widget(List::new(items).block(block), main, &mut state);

    let status = Rect::new(area.x, area.y + main_h, area.width, 1);
    let p = Paragraph::new(Line::raw(format!(
        "  {} sessions  |  Tab: views  Esc: back to chat  q: quit",
        app.beats.len()
    )))
    .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(p, status);
}
