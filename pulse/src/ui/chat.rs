//! Chat view: sessions sidebar (window while focused) + scrolling transcript
//! + input bar, with the `@` project popup and project input popups.

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{App, AtPopup, EntryRow, InputPopup, Mode, TranscriptLine};

/// Width of the sessions sidebar when the sessions window is open.
const SESSIONS_WIDTH: u16 = 32;

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let main_h = if area.height > 2 { area.height - 1 } else { 0 };
    let main = Rect::new(area.x, area.y, area.width, main_h);
    let status = Rect::new(area.x, area.y + main_h, area.width, 1);

    // The sessions sidebar is only visible while the sessions window is open;
    // chat gets the full width otherwise.
    let chat = if app.mode == Mode::Sessions {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(SESSIONS_WIDTH), Constraint::Min(1)])
            .split(main);
        render_sidebar(frame, app, chunks[0]);
        chunks[1]
    } else {
        app.rects.sidebar = None;
        main
    };

    // Split chat into transcript + input bar
    let input_h = 3;
    let chat_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(input_h)])
        .split(chat);

    render_transcript(frame, app, chat_chunks[0]);
    render_input(frame, app, chat_chunks[1]);

    // Floating overlays
    render_at_popup(frame, app, chat_chunks[1]);
    render_input_popup(frame, app);
    render_cloning(frame, app);

    render_status(frame, app, status);
}

fn render_sidebar(frame: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Sessions (j/k, Enter, click) ")
        .border_style(Style::default().fg(Color::Cyan));

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
            let name = format!(" {prefix} {}", b.name);
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

    // Keep the persistent selection valid against the current list length.
    if app
        .session_list
        .selected()
        .is_none_or(|s| s >= app.beats.len())
    {
        app.session_list
            .select(Some(app.beats.len().saturating_sub(1)));
    }
    app.rects.sidebar = Some(block.inner(area));

    frame.render_stateful_widget(
        List::new(items)
            .block(block)
            .highlight_style(Style::default().bg(Color::Cyan).fg(Color::Black))
            .highlight_symbol("> "),
        area,
        &mut app.session_list,
    );
}

fn render_transcript(frame: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Chat ")
        .border_style(Style::default().fg(Color::DarkGray));
    let inner = block.inner(area);

    // Build each entry's lines, measure its wrapped height, and keep a
    // row-range map so mouse clicks can be traced back to entries.
    let mut all_lines: Vec<Line> = Vec::new();
    let mut rows: Vec<EntryRow> = Vec::new();
    let mut y = 0usize;
    for (idx, entry) in app.transcript.iter().enumerate() {
        let (lines, tool) = build_entry_lines(&app.expanded_tools, idx, entry);
        let height = Paragraph::new(lines.clone())
            .wrap(Wrap { trim: false })
            .line_count(inner.width);
        rows.push(EntryRow {
            start: y,
            height,
            tool,
        });
        y += height;
        all_lines.extend(lines);
    }
    if app.task_running {
        all_lines.push(Line::styled(
            "  (running...)",
            Style::default().fg(Color::Yellow),
        ));
    }

    app.entry_rows = rows;
    app.rects.transcript = Some(inner);

    // Clamp scrolling to the actual content and keep the view pinned to the
    // bottom while following new output.
    let max_scroll = y.saturating_sub(inner.height as usize) as u16;
    if app.follow || app.scroll > max_scroll {
        app.scroll = max_scroll;
    }
    if app.scroll >= max_scroll {
        app.follow = true;
    }

    let p = Paragraph::new(all_lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((app.scroll, 0));
    frame.render_widget(p, area);
}

/// Lines for one transcript entry. The second return value is the transcript
/// index for tool entries (whose output can be toggled), `None` otherwise.
fn build_entry_lines(
    expanded: &std::collections::HashSet<usize>,
    idx: usize,
    entry: &TranscriptLine,
) -> (Vec<Line<'static>>, Option<usize>) {
    let mut out = Vec::new();
    let mut is_tool = None;
    match entry {
        TranscriptLine::User(text) => {
            out.push(Line::from(vec![
                Span::styled(
                    "User: ".to_string(),
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(text.clone()),
            ]));
            out.push(Line::raw(""));
        }
        TranscriptLine::Assistant(text) => {
            out.push(Line::styled(
                "Assistant",
                Style::default()
                    .fg(Color::Blue)
                    .add_modifier(Modifier::BOLD),
            ));
            out.extend(super::markdown::markdown_lines(text));
            out.push(Line::raw(""));
        }
        TranscriptLine::Tool {
            tool,
            arguments,
            result,
            error,
        } => {
            let is_expanded = expanded.contains(&idx);
            // Error calls keep their [!] marker; otherwise show the toggle state.
            let marker = match (*error, is_expanded) {
                (true, _) => "[!]",
                (false, true) => "▾",
                (false, false) => "▸",
            };
            let color = if *error { Color::Red } else { Color::Magenta };
            let mut spans = vec![
                Span::styled(format!(" {marker} "), Style::default().fg(color)),
                Span::styled(format!("[{tool}] "), Style::default().fg(color)),
                Span::styled(arguments.clone(), Style::default().fg(Color::DarkGray)),
            ];
            if !is_expanded {
                let n = result.lines().count();
                let hint = match n {
                    0 => " (no output)".to_string(),
                    1 => " (1 line — click to show)".to_string(),
                    n => format!(" ({n} lines — click to show)"),
                };
                spans.push(Span::styled(hint, Style::default().fg(Color::DarkGray)));
            }
            out.push(Line::from(spans));
            if is_expanded {
                let out_color = if *error { Color::Red } else { Color::Gray };
                for line in truncate_str(result, 4000).lines() {
                    out.push(Line::styled(
                        format!("     {line}"),
                        Style::default().fg(out_color),
                    ));
                }
            }
            out.push(Line::raw(""));
            is_tool = Some(idx);
        }
        TranscriptLine::Step(text) => {
            out.push(Line::from(vec![
                Span::styled("  → ", Style::default().fg(Color::DarkGray)),
                Span::raw(text.clone()),
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
    (out, is_tool)
}

fn render_input(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner_w = area.width.saturating_sub(2) as usize;
    let prompt_text = if app.task_running {
        "(task running — Ctrl+K to cancel)".to_string()
    } else {
        let mut shown = format!("> {}", app.input);
        if app.input_cursor >= app.input.len() {
            shown.push('_');
        }
        // Show the tail of the input when it overflows the bar.
        let chars: Vec<char> = shown.chars().collect();
        let start = chars.len().saturating_sub(inner_w);
        chars[start..].iter().collect()
    };
    let p = Paragraph::new(prompt_text)
        .block(block)
        .alignment(Alignment::Left);
    frame.render_widget(p, area);
}

/// `@` project autocomplete, floating above the input bar.
fn render_at_popup(frame: &mut Frame, app: &mut App, input_area: Rect) {
    let Some(AtPopup::Projects { filter, selected }) = &app.at_popup else {
        app.rects.at_popup = None;
        return;
    };
    let matching = app.matching_projects(filter);
    let mut items: Vec<ListItem> = matching
        .iter()
        .map(|p| ListItem::new(format!(" {}", p.name)))
        .collect();
    items.push(ListItem::new(" + Add local project…"));
    items.push(ListItem::new(" + Clone GitHub repo…"));

    let w = 40.min(input_area.width);
    let h = (items.len() as u16 + 2).min(10);
    let popup = Rect::new(input_area.x, input_area.y.saturating_sub(h), w, h);
    app.rects.at_popup = Some(popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" @ Projects ")
        .border_style(Style::default().fg(Color::Cyan));
    let mut state = ListState::default();
    state.select(Some(*selected));
    // Erase the chat text underneath so the popup is opaque.
    frame.render_widget(ratatui::widgets::Clear, popup);
    frame.render_stateful_widget(
        List::new(items)
            .block(block)
            .highlight_style(Style::default().bg(Color::Cyan).fg(Color::Black)),
        popup,
        &mut state,
    );
}

/// Small centered text input for add-local-project / clone-repo.
fn render_input_popup(frame: &mut Frame, app: &mut App) {
    let Some(popup) = app.input_popup else {
        app.rects.input_popup = None;
        return;
    };
    let area = frame.area();
    let w = 60.min(area.width);
    let h = 6.min(area.height);
    let x = area.x + (area.width - w) / 2;
    let y = area.y + (area.height - h) / 2;
    let popup_area = Rect::new(x, y, w, h);
    app.rects.input_popup = Some(popup_area);

    let (title, hint) = match popup {
        InputPopup::AddPath => (" Add local project ", "Directory path"),
        InputPopup::CloneRepo => (" Clone GitHub repo ", "owner/repo or GitHub URL"),
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(Color::Cyan));
    let text = vec![
        Line::styled(hint.to_string(), Style::default().fg(Color::DarkGray)),
        Line::raw(""),
        Line::raw(format!("> {}_", app.input_popup_text)),
        Line::raw(""),
        Line::styled(
            "Enter: confirm  Esc: cancel",
            Style::default().fg(Color::DarkGray),
        ),
    ];
    let p = ratatui::widgets::Paragraph::new(text).block(block);
    frame.render_widget(ratatui::widgets::Clear, popup_area);
    frame.render_widget(p, popup_area);
}

/// "Cloning…" indicator while a background repo clone is running.
fn render_cloning(frame: &mut Frame, app: &App) {
    if app.clone_handle.is_none() {
        return;
    }
    let area = frame.area();
    let w = 30.min(area.width);
    let h = 3.min(area.height);
    let x = area.x + (area.width - w) / 2;
    let y = area.y + (area.height - h) / 2;
    let popup = Rect::new(x, y, w, h);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Cloning ")
        .border_style(Style::default().fg(Color::Cyan));
    let p = Paragraph::new("Cloning…")
        .block(block)
        .alignment(Alignment::Center);
    frame.render_widget(ratatui::widgets::Clear, popup);
    frame.render_widget(p, popup);
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
        let model = truncate_str(&app.current_model, 24);
        let tier = if app.current_tier.is_empty() {
            String::new()
        } else {
            format!(" ({})", app.current_tier)
        };
        parts.push(Span::raw(format!("Model: {model}{tier}  ")));
    }
    if app.current_cost > 0.0 {
        parts.push(Span::raw(format!("Cost: ${:.4}  ", app.current_cost)));
    }
    if let Some(ctx) = app.current_context {
        parts.push(Span::raw(format!("Ctx: {:.0}%", ctx)));
    }
    parts.push(Span::raw(
        "  Tab/Left: sessions  @: project  wheel: scroll  Ctrl+C: quit",
    ));

    let line = Line::from(parts);
    let p = Paragraph::new(line).style(Style::default().fg(Color::DarkGray));
    frame.render_widget(p, area);
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let head: String = s.chars().take(max).collect();
        format!("{head}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_entry(result: &str) -> TranscriptLine {
        TranscriptLine::Tool {
            tool: "bash".into(),
            arguments: "ls -la".into(),
            result: result.into(),
            error: false,
        }
    }

    #[test]
    fn tool_entry_is_collapsed_by_default() {
        let expanded = std::collections::HashSet::new();
        let (lines, tool) = build_entry_lines(&expanded, 3, &tool_entry("line1\nline2\nline3"));
        assert_eq!(tool, Some(3));
        // Header + blank line, no output.
        assert_eq!(lines.len(), 2);
        let header = lines[0].to_string();
        assert!(header.contains("[bash]"));
        assert!(header.contains("ls -la"));
        assert!(header.contains("3 lines"));
        assert!(!header.contains("line1"));
    }

    #[test]
    fn tool_entry_expanded_shows_output() {
        let mut expanded = std::collections::HashSet::new();
        expanded.insert(3);
        let (lines, tool) = build_entry_lines(&expanded, 3, &tool_entry("line1\nline2"));
        assert_eq!(tool, Some(3));
        // Header + 2 output lines + blank.
        assert_eq!(lines.len(), 4);
        assert!(lines[1].to_string().contains("line1"));
        assert!(lines[2].to_string().contains("line2"));
    }

    #[test]
    fn tool_entry_empty_result_has_no_output_hint() {
        let expanded = std::collections::HashSet::new();
        let (lines, _) = build_entry_lines(&expanded, 0, &tool_entry(""));
        assert!(lines[0].to_string().contains("(no output)"));
    }

    #[test]
    fn tool_entry_marks_errors() {
        let entry = TranscriptLine::Tool {
            tool: "bash".into(),
            arguments: "oops".into(),
            result: "boom".into(),
            error: true,
        };
        let (lines, tool) = build_entry_lines(&std::collections::HashSet::new(), 0, &entry);
        assert_eq!(tool, Some(0));
        assert!(lines[0].to_string().contains("[!]"));
    }

    #[test]
    fn truncate_str_is_char_safe() {
        assert_eq!(truncate_str("héllo wörld", 5), "héllo…");
        assert_eq!(truncate_str("short", 10), "short");
    }
}
