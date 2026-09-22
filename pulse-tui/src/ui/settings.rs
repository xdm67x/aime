//! Settings view — API keys, model configuration, LiteLLM base URL.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::app::App;

const FIELD_LABELS: [&str; 7] = [
    "OpenRouter API Key",
    "OpenCode API Key",
    "LiteLLM API Key",
    "Classifier Model",
    "High Model",
    "Base Model",
    "Low Model",
];

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let main_h = if area.height > 1 { area.height - 1 } else { 0 };
    let main = Rect::new(area.x, area.y, area.width, main_h);

    let chunks = Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([
            Constraint::Length(6),
            Constraint::Length(1),
            Constraint::Min(1),
        ])
        .split(main);

    // API Keys section
    let api_block = Block::default()
        .borders(Borders::TOP)
        .title(" API Keys ")
        .title_style(Style::default().fg(Color::Yellow));
    let api_lines = render_api_keys(app);
    frame.render_widget(Paragraph::new(api_lines).block(api_block), chunks[0]);

    // Model config section
    let model_block = Block::default()
        .borders(Borders::TOP)
        .title(" Model Configuration ")
        .title_style(Style::default().fg(Color::Yellow));
    let model_lines = render_models(app);
    frame.render_widget(Paragraph::new(model_lines).block(model_block), chunks[2]);

    let status = Rect::new(area.x, area.y + main_h, area.width, 1);
    let p = Paragraph::new(Line::raw(
        "  Enter: edit field  Esc: cancel edit  Tab: views  q: quit",
    ))
    .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(p, status);
}

fn render_api_keys(app: &App) -> Vec<Line<'_>> {
    let keys = [
        get_masked_key(app, "openrouter"),
        get_masked_key(app, "opencode"),
        get_masked_key(app, "litellm"),
    ];
    let mut lines = Vec::new();
    for (i, (label, value)) in FIELD_LABELS.iter().take(3).zip(keys.iter()).enumerate() {
        let selected = i == app.settings_field;
        let prefix = if selected { ">" } else { " " };
        let editing = selected && app.settings_editing;
        let display = if editing {
            format!("{}_", app.settings_input)
        } else {
            value.to_string()
        };
        lines.push(Line::from(vec![
            Span::raw(format!("{prefix} ")),
            Span::styled(
                format!("{label}:  "),
                Style::default().fg(if selected { Color::Cyan } else { Color::White }),
            ),
            Span::styled(display, Style::default().fg(Color::Green)),
        ]));
    }
    lines
}

fn render_models(app: &App) -> Vec<Line<'_>> {
    let models = [
        &app.model_config.classifier,
        &app.model_config.high,
        &app.model_config.base,
        &app.model_config.low,
    ];
    let mut lines = Vec::new();
    for (i, (label, value)) in FIELD_LABELS.iter().skip(3).zip(models.iter()).enumerate() {
        let field_idx = i + 3;
        let selected = field_idx == app.settings_field;
        let prefix = if selected { ">" } else { " " };
        let editing = selected && app.settings_editing;
        let display = if editing {
            format!("{}_", app.settings_input)
        } else {
            value.to_string()
        };
        lines.push(Line::from(vec![
            Span::raw(format!("{prefix} ")),
            Span::styled(
                format!("{label}:  "),
                Style::default().fg(if selected { Color::Cyan } else { Color::White }),
            ),
            Span::styled(display, Style::default().fg(Color::Green)),
        ]));
    }

    // LiteLLM base URL
    let base_url = pulse_core::config::get_base_url("litellm")
        .ok()
        .flatten()
        .unwrap_or_default();
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled("LiteLLM Base URL:  ", Style::default().fg(Color::White)),
        Span::styled(base_url, Style::default().fg(Color::Green)),
    ]));
    lines
}

fn get_masked_key(_app: &App, provider: &str) -> String {
    match pulse_core::config::get_api_key(provider) {
        Ok(Some(key)) if key.len() > 8 => {
            format!("{}…{}", &key[..4], &key[key.len() - 4..])
        }
        Ok(Some(key)) if !key.is_empty() => "****".to_string(),
        _ => "(not set)".to_string(),
    }
}
