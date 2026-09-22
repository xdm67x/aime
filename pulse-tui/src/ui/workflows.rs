//! Workflows view — browse, run, create, edit workflow files.

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
        .title(" Workflows (j/k: navigate, Enter: run, e: edit, n: new, r: refresh) ")
        .border_style(Style::default().fg(Color::Cyan));

    let items: Vec<ListItem> = app
        .workflows
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let prefix = if i == app.selected_workflow { ">" } else { " " };
            let model = w.model.as_deref().unwrap_or("(classifier)");
            ListItem::new(Line::from(vec![
                Span::raw(format!("{prefix} ")),
                Span::styled(
                    w.name.clone(),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!("  {}  ", w.description)),
                Span::styled(
                    format!("{} steps  model: {}", w.steps.len(), model),
                    Style::default().fg(Color::DarkGray),
                ),
            ]))
        })
        .collect();

    if items.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::raw(
                "  No workflows found. Press 'n' to create one in ~/.pulse/workflows/",
            ))
            .block(block),
            main,
        );
    } else {
        let mut state = ListState::default();
        state.select(Some(
            app.selected_workflow
                .min(app.workflows.len().saturating_sub(1)),
        ));
        frame.render_stateful_widget(List::new(items).block(block), main, &mut state);
    }

    let status = Rect::new(area.x, area.y + main_h, area.width, 1);
    let p = Paragraph::new(Line::raw(format!(
        "  {} workflows  |  Enter: run on active beat  e: edit  n: new  Tab: views  q: quit",
        app.workflows.len()
    )))
    .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(p, status);
}
