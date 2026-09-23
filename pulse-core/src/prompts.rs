//! Prompt templates, embedded into the binary at compile time.
//!
//! The prompt text lives in the `prompts/` directory next to this module and
//! is pulled in with `include_str!` — the packaged app ships every prompt,
//! and no runtime file lookups can fail. Templates that carry runtime values
//! use `{{key}}` placeholders, filled with [`fill`].

/// Tool-use note appended to the system message of agentic runs.
pub const AGENT_NOTE: &str = include_str!("prompts/agent-note.md");

/// Summarizes prior beat messages into a context brief (`{{history}}`).
pub const SUMMARIZE: &str = include_str!("prompts/summarize.md");

/// Built-in session instructions, prepended to every model run's system message.
pub const SYSTEM: &str = include_str!("prompts/system.md");

/// Fill `{{key}}` placeholders in a prompt template with values.
///
/// Scans the template once, left to right, so inserted values are never
/// re-expanded even if they contain placeholder-like text. Unknown
/// `{{...}}` markers are left in place, which keeps template typos visible
/// in the final prompt instead of silently erasing them.
pub fn fill(template: &str, vars: &[(&str, &str)]) -> String {
    let tokens: Vec<(String, &str)> = vars
        .iter()
        .map(|(key, value)| {
            let mut token = String::with_capacity(key.len() + 4);
            token.push_str("{{");
            token.push_str(key);
            token.push_str("}}");
            (token, *value)
        })
        .collect();
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    'scan: while let Some(open) = rest.find("{{") {
        for (token, value) in &tokens {
            if rest[open..].starts_with(token.as_str()) {
                out.push_str(&rest[..open]);
                out.push_str(value);
                rest = &rest[open + token.len()..];
                continue 'scan;
            }
        }
        // no known placeholder starts here — keep the literal text
        out.push_str(&rest[..open + 2]);
        rest = &rest[open + 2..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fill() {
        let t = "Task:\n{{prompt}}\nDraft: {{draft}}";
        let out = fill(t, &[("prompt", "fix the bug"), ("draft", "done")]);
        assert_eq!(out, "Task:\nfix the bug\nDraft: done");

        // same placeholder twice
        assert_eq!(fill("{{a}}-{{a}}", &[("a", "x")]), "x-x");

        // values are not re-scanned, so nested placeholders survive
        let out = fill("{{a}}", &[("a", "{{b}}"), ("b", "boom")]);
        assert_eq!(out, "{{b}}");

        // unknown markers stay visible
        assert_eq!(fill("{{nope}} {{a}}", &[("a", "1")]), "{{nope}} 1");

        // no vars — template passes through
        assert_eq!(fill("plain {{a}}", &[]), "plain {{a}}");
    }

    #[test]
    fn test_prompts_embedded() {
        // the embedded files are non-empty and the templated ones declare
        // every placeholder the code fills
        assert!(AGENT_NOTE.contains("Tools are available"));
        assert!(SUMMARIZE.contains("{{history}}"));
        assert!(SYSTEM.contains("Session instructions"));
    }
}
