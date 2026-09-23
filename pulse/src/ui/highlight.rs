//! Lightweight terminal syntax highlighting for file-content tool results
//! (`read_file` / `write_file` / `edit_file`). One pass per line: comments,
//! strings, numbers, keywords, types, and function calls. No parsing trees —
//! just enough coloring to make code readable in the transcript.

use ratatui::style::{Color, Style};
use ratatui::text::Span;

fn comment_style() -> Style {
    Style::default().fg(Color::DarkGray)
}
fn string_style() -> Style {
    Style::default().fg(Color::Green)
}
fn number_style() -> Style {
    Style::default().fg(Color::Yellow)
}
fn keyword_style() -> Style {
    Style::default().fg(Color::Magenta)
}
fn type_style() -> Style {
    Style::default().fg(Color::Cyan)
}
fn function_style() -> Style {
    Style::default().fg(Color::Blue)
}

/// Map a file path to a language id. Reuses the core's extension map so the
/// TUI and the core agree on which files count as which language.
pub fn lang_for_path(path: &str) -> Option<&'static str> {
    pulse_core::diff::lang_for_path(path)
}

/// Highlight one line of `lang` source. `base` styles plain text (the caller
/// picks the tone — e.g. dim inside a diff). `None` lang → plain.
pub fn highlight_line(line: &str, lang: Option<&str>, base: Style) -> Vec<Span<'static>> {
    let Some(lang) = lang else {
        return vec![Span::styled(line.to_string(), base)];
    };
    let chars: Vec<char> = line.chars().collect();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut plain = String::new();
    let mut i = 0usize;

    macro_rules! flush {
        () => {
            if !plain.is_empty() {
                spans.push(Span::styled(std::mem::take(&mut plain), base));
            }
        };
    }

    while i < chars.len() {
        // Comment: the marker through end of line.
        if comment_at(&chars, i, lang) {
            flush!();
            let rest: String = chars[i..].iter().collect();
            spans.push(Span::styled(rest, comment_style()));
            break;
        }
        let c = chars[i];
        // Strings: quoted with backslash escapes; backticks in the JS family.
        if c == '"' || c == '\'' || (c == '`' && backtick_strings(lang)) {
            flush!();
            let start = i;
            i += 1;
            while i < chars.len() {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    i += 2;
                } else if chars[i] == c {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            let s: String = chars[start..i].iter().collect();
            spans.push(Span::styled(s, string_style()));
            continue;
        }
        // Numbers: digit-led runs (covers hex / floats loosely).
        if c.is_ascii_digit() {
            flush!();
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '.' || chars[i] == '_') {
                i += 1;
            }
            let s: String = chars[start..i].iter().collect();
            spans.push(Span::styled(s, number_style()));
            continue;
        }
        // Identifiers → keyword / type / function / plain.
        if is_ident_start(c) {
            let start = i;
            while i < chars.len() && is_ident_char(chars[i]) {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let next = chars[i..].iter().find(|c| !c.is_whitespace());
            let style = if is_keyword(&word) {
                keyword_style()
            } else if is_type(&word) || starts_uppercase(&word) {
                type_style()
            } else if next == Some(&'(') {
                function_style()
            } else {
                plain.push_str(&word);
                continue;
            };
            flush!();
            spans.push(Span::styled(word, style));
            continue;
        }
        plain.push(c);
        i += 1;
    }
    flush!();
    if spans.is_empty() {
        spans.push(Span::styled(String::new(), base));
    }
    spans
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == '$'
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

fn starts_uppercase(word: &str) -> bool {
    word.chars().next().is_some_and(|c| c.is_uppercase())
}

/// Whether a comment marker starts at index `i`.
fn comment_at(chars: &[char], i: usize, lang: &str) -> bool {
    comment_markers(lang).iter().any(|marker| {
        let m: Vec<char> = marker.chars().collect();
        chars.len() >= i + m.len() && chars[i..i + m.len()] == m[..]
    })
}

fn comment_markers(lang: &str) -> &'static [&'static str] {
    match lang {
        "python" | "ruby" | "bash" | "yaml" | "elixir" | "perl" => &["#"],
        "sql" | "lua" | "haskell" => &["--"],
        _ => &["//", "/*"],
    }
}

fn backtick_strings(lang: &str) -> bool {
    matches!(lang, "javascript" | "typescript" | "go")
}

/// Shared keyword pool across the supported languages (deliberately loose —
/// a keyword colored in the "wrong" language is harmless in a terminal view).
const KEYWORDS: &[&str] = &[
    "as", "async", "await", "and", "abstract", "begin", "break", "case", "catch", "chan",
    "class", "const", "continue", "def", "default", "defer", "del", "delete", "do", "done",
    "dyn", "elif", "else", "elseif", "end", "enum", "except", "export", "extends", "extern",
    "false", "fi", "final", "finally", "fn", "for", "foreach", "func", "function", "go",
    "goto", "if", "impl", "import", "in", "interface", "is", "lambda", "let", "local",
    "loop", "match", "mod", "move", "mut", "new", "nil", "none", "not", "null", "or",
    "override", "package", "pass", "print", "private", "protected", "pub", "public",
    "raise", "range", "ref", "rescue", "return", "select", "self", "static", "struct",
    "super", "switch", "then", "this", "throw", "throws", "trait", "true", "try", "type",
    "typeof", "unless", "unsafe", "until", "use", "var", "virtual", "when", "where",
    "while", "with", "yield",
];

const TYPES: &[&str] = &[
    "any", "bigint", "bool", "boolean", "byte", "char", "dict", "double", "error", "f32",
    "f64", "float", "i8", "i16", "i32", "i64", "i128", "int", "isize", "list", "long",
    "number", "object", "short", "signed", "str", "string", "symbol", "tuple", "u8", "u16",
    "u32", "u64", "u128", "uint", "unsigned", "usize", "void",
];

fn is_keyword(word: &str) -> bool {
    KEYWORDS.contains(&word)
}

fn is_type(word: &str) -> bool {
    TYPES.contains(&word)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_lang_is_plain() {
        let spans = highlight_line("fn main() {}", None, Style::default());
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, "fn main() {}");
    }

    #[test]
    fn rust_line_gets_keyword_string_and_fn_colors() {
        let spans = highlight_line("let s = \"hi\"; // note", Some("rust"), Style::default());
        let joined = spans
            .iter()
            .map(|s| s.content.to_string())
            .collect::<String>();
        assert_eq!(joined, "let s = \"hi\"; // note");
        // keyword `let`
        assert!(spans.iter().any(|s| s.content == "let" && s.style.fg == Some(Color::Magenta)));
        // string literal
        assert!(spans.iter().any(|s| s.content == "\"hi\"" && s.style.fg == Some(Color::Green)));
        // trailing comment keeps its marker
        assert!(spans.iter().any(|s| s.content == "// note" && s.style.fg == Some(Color::DarkGray)));
    }

    #[test]
    fn python_uses_hash_comments() {
        let spans = highlight_line("x = 1  # one", Some("python"), Style::default());
        assert!(spans.iter().any(|s| s.content == "# one" && s.style.fg == Some(Color::DarkGray)));
    }

    #[test]
    fn numbers_and_calls_are_colored() {
        let spans = highlight_line("count(42)", Some("typescript"), Style::default());
        assert!(spans.iter().any(|s| s.content == "count" && s.style.fg == Some(Color::Blue)));
        assert!(spans.iter().any(|s| s.content == "42" && s.style.fg == Some(Color::Yellow)));
    }

    #[test]
    fn uppercase_identifiers_read_as_types() {
        let spans = highlight_line("let v: Vec<u8> = Vec::new();", Some("rust"), Style::default());
        assert!(spans.iter().any(|s| s.content == "Vec" && s.style.fg == Some(Color::Cyan)));
    }

    #[test]
    fn lang_for_path_maps_extensions() {
        assert_eq!(lang_for_path("src/lib.rs"), Some("rust"));
        assert_eq!(lang_for_path("notes/general.txt"), None);
    }
}
