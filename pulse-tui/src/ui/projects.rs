//! Projects view — list, add local, clone GitHub, remove.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::app::App;

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let main_h = area.height.saturating_sub(1);
    let main = Rect::new(area.x, area.y, area.width, main_h);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Projects (j/k: navigate, a: add local, c: clone, r: remove) ")
        .border_style(Style::default().fg(Color::Cyan));

    let items: Vec<ListItem> = app
        .projects
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let prefix = if i == app.selected_project { ">" } else { " " };
            ListItem::new(Line::from(vec![
                Span::raw(format!("{prefix} ")),
                Span::styled(format!("[{}] ", i + 1), Style::default().fg(Color::Cyan)),
                Span::styled(
                    p.name.clone(),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!("  {}", p.path)),
                Span::styled(
                    format!("  ({})", p.source),
                    Style::default().fg(Color::DarkGray),
                ),
            ]))
        })
        .collect();

    if items.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::raw(
                "  No projects. Press 'a' to add a local directory.",
            ))
            .block(block),
            main,
        );
    } else {
        let mut state = ListState::default();
        state.select(Some(
            app.selected_project
                .min(app.projects.len().saturating_sub(1)),
        ));
        frame.render_stateful_widget(List::new(items).block(block), main, &mut state);
    }

    let status = Rect::new(area.x, area.y + main_h, area.width, 1);
    let p = Paragraph::new(Line::raw(format!(
        "  {} projects  |  a: add  c: clone  r: remove  Tab: views  q: quit",
        app.projects.len()
    )))
    .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(p, status);
}
