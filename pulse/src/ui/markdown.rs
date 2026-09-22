//! Lightweight markdown rendering for the chat transcript: headings, fenced
//! code blocks, quotes, and inline bold / italic / code styling.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// Render a markdown-ish text block into styled lines.
pub fn markdown_lines(text: &str) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let mut in_code = false;
    for raw in text.lines() {
        if raw.trim_start().starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            out.push(Line::styled(
                format!("  {raw}"),
                Style::default().fg(Color::Gray),
            ));
            continue;
        }
        if is_heading(raw) {
            out.push(Line::from(Span::styled(
                raw.to_string(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )));
        } else if raw.trim_start().starts_with("> ") || raw.trim() == ">" {
            let inner = raw.trim_start().trim_start_matches('>').trim_start();
            out.push(Line::from(parse_inline(
                inner,
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            )));
        } else {
            out.push(Line::from(parse_inline(raw, Style::default())));
        }
    }
    if out.is_empty() {
        out.push(Line::raw(""));
    }
    out
}

/// `#` up to `######`, followed by a space.
fn is_heading(line: &str) -> bool {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    (1..=6).contains(&hashes) && line[hashes..].starts_with(' ')
}

/// Split text into spans, styling `**bold**`, `*italic*`, and `` `code` ``.
/// Unclosed markers are left as literal text.
fn parse_inline(text: &str, base: Style) -> Vec<Span<'static>> {
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut buf = String::new();
    let mut rest = text;

    while !rest.is_empty() {
        let found = find_open(rest);
        let Some((pos, tok, style)) = found else {
            buf.push_str(rest);
            break;
        };
        buf.push_str(&rest[..pos]);
        let after = &rest[pos + tok.len()..];
        match after.find(tok) {
            Some(end) => {
                if !buf.is_empty() {
                    out.push(Span::styled(std::mem::take(&mut buf), base));
                }
                out.push(Span::styled(after[..end].to_string(), style));
                rest = &after[end + tok.len()..];
            }
            None => {
                buf.push_str(&rest[pos..pos + tok.len()]);
                rest = after;
            }
        }
    }
    if !buf.is_empty() {
        out.push(Span::styled(buf, base));
    }
    if out.is_empty() {
        out.push(Span::styled(String::new(), base));
    }
    out
}

/// Earliest opening token in `s`: `**` before `*`, both before `` ` ``.
fn find_open(s: &str) -> Option<(usize, &'static str, Style)> {
    let candidates: [(&str, Style); 3] = [
        ("**", Style::default().add_modifier(Modifier::BOLD)),
        ("`", Style::default().fg(Color::Yellow)),
        ("*", Style::default().add_modifier(Modifier::ITALIC)),
    ];
    let mut best: Option<(usize, &'static str, Style)> = None;
    for (tok, style) in candidates {
        if let Some(pos) = s.find(tok) {
            let better = match best {
                None => true,
                Some((bpos, btok, _)) => pos < bpos || (pos == bpos && tok.len() > btok.len()),
            };
            if better {
                best = Some((pos, tok, style));
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_bold_and_code() {
        let spans = parse_inline("a **b** c", Style::default());
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].content, "a ");
        assert_eq!(spans[1].content, "b");
        assert!(spans[1].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(spans[2].content, " c");

        let spans = parse_inline("run `cargo test` now", Style::default());
        assert_eq!(spans[1].content, "cargo test");
        assert_eq!(spans[1].style.fg, Some(Color::Yellow));
    }

    #[test]
    fn unclosed_marker_stays_literal() {
        let spans = parse_inline("a * b", Style::default());
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, "a * b");
    }

    #[test]
    fn headings_quotes_and_code_blocks() {
        let lines = markdown_lines("# Title\n\nsome text\n\n```rust\nfn x() {}\n```\n> note");
        assert!(lines[0].to_string().contains("# Title"));
        // blank line preserved
        assert_eq!(lines[1].to_string(), "");
        assert!(lines[2].to_string().contains("some text"));
        // fence lines themselves are not rendered
        assert!(lines[4].to_string().contains("fn x() {}"));
        assert_eq!(lines[5].to_string(), "note");
    }

    #[test]
    fn underscores_are_not_emphasis() {
        // snake_case identifiers must not be mangled.
        let spans = parse_inline("my_var_name", Style::default());
        assert_eq!(spans[0].content, "my_var_name");
    }
}
