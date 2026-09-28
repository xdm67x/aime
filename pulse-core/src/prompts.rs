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

/// Verifies a workflow step's goal against its result (`{{goal}}`, `{{result}}`).
pub const GOAL_CHECK: &str = include_str!("prompts/goal-check.md");

/// Continuation prompt when a step's goal check fails (`{{goal}}`, `{{reason}}`).
pub const GOAL_RETRY: &str = include_str!("prompts/goal-retry.md");

/// Built-in session instructions, prepended to every model run's system message.
pub const SYSTEM: &str = include_str!("prompts/system.md");

/// Fill `{{key}}` placeholders in a prompt template with values.
///
/// Scans the template once, left to right, so inserted values are never
/// re-expanded even if they contain placeholder-like text. Whitespace
/// inside the braces is tolerated (`{{ key }}` fills like `{{key}}`).
/// Unknown `{{...}}` markers are left in place, which keeps template typos
/// visible in the final prompt instead of silently erasing them.
pub fn fill(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    'scan: while let Some(open) = rest.find("{{") {
        let after = &rest[open + 2..];
        let lead = after.len() - after.trim_start().len();
        for (key, value) in vars {
            // "{{key}}" or padded "{{ key }}" — nothing but whitespace may
            // sit between the braces and the key (or the key and "}}")
            if let Some(mid) = after[lead..].strip_prefix(key) {
                let trail = mid.len() - mid.trim_start().len();
                if mid[trail..].starts_with("}}") {
                    out.push_str(&rest[..open]);
                    out.push_str(value);
                    rest = &mid[trail + 2..];
                    continue 'scan;
                }
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
    fn test_fill_tolerates_placeholder_whitespace() {
        let vars = [("steps.a", "1"), ("prompt", "hi")];
        // padded and exact spellings fill the same
        assert_eq!(fill("{{ steps.a }}", &vars), "1");
        assert_eq!(fill("{{steps.a}}", &vars), "1");
        assert_eq!(fill("{{ prompt }}", &vars), "hi");
        // unknown markers stay visible, padding and all
        assert_eq!(fill("{{ nope }}", &vars), "{{ nope }}");
        // a shorter key must not eat into a longer placeholder
        let vars = [("steps.a", "1"), ("steps.ab", "2")];
        assert_eq!(fill("{{ steps.ab }} and {{ steps.a }}", &vars), "2 and 1");
        // text after a padded placeholder is kept
        assert_eq!(fill("{{ a }} trailing", &[("a", "x")]), "x trailing");
    }

    #[test]
    fn test_prompts_embedded() {
        // the embedded files are non-empty and the templated ones declare
        // every placeholder the code fills
        assert!(AGENT_NOTE.contains("Tools are available"));
        assert!(SUMMARIZE.contains("{{history}}"));
        assert!(SYSTEM.contains("Session instructions"));
        assert!(GOAL_CHECK.contains("{{goal}}") && GOAL_CHECK.contains("{{result}}"));
        assert!(GOAL_RETRY.contains("{{goal}}") && GOAL_RETRY.contains("{{reason}}"));
    }
}
