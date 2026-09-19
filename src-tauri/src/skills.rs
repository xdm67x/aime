//! Skill discovery: scans `~/.agents/skills/*/SKILL.md`, parses YAML
//! frontmatter to extract `name` + `description`. Only the frontmatter is sent
//! to the model (as tool descriptions); the full file is loaded on demand when
//! the model invokes a skill tool.

use serde::Serialize;

/// One skill, discovered from `~/.agents/skills/<name>/SKILL.md`.
#[derive(Clone, Serialize)]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    /// Absolute path to the SKILL.md file.
    pub path: String,
}

/// Discover all skills in `~/.agents/skills/`. Each subdirectory containing a
/// `SKILL.md` is a skill; its YAML frontmatter (between `---` markers)
/// provides the name and description shown to the model.
pub fn discover() -> Vec<SkillInfo> {
    let home = match std::env::var("HOME") {
        Ok(h) => h,
        Err(_) => return vec![],
    };
    let dir = std::path::Path::new(&home).join(".agents/skills");
    let mut skills = vec![];
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let md = entry.path().join("SKILL.md");
            if md.is_file() {
                if let Some(s) = parse(&md) {
                    skills.push(s);
                }
            }
        }
    }
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}

/// Read the full SKILL.md content for a skill by name.
pub fn load_content(name: &str) -> Result<String, String> {
    let home = std::env::var("HOME").map_err(|e| e.to_string())?;
    let path = std::path::Path::new(&home)
        .join(".agents/skills")
        .join(name)
        .join("SKILL.md");
    std::fs::read_to_string(&path).map_err(|e| format!("skill {name}: {e}"))
}

/// Parse the YAML frontmatter of a SKILL.md file. Handles simple scalars
/// (`name: foo`) and folded block scalars (`description: >-` followed by
/// indented lines). Not a full YAML parser — just enough for skill files.
fn parse(path: &std::path::Path) -> Option<SkillInfo> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut lines = content.lines();
    let first = lines.next()?;
    if first.trim() != "---" {
        return None;
    }
    let mut fm = String::new();
    for line in lines {
        if line.trim() == "---" {
            break;
        }
        fm.push_str(line);
        fm.push('\n');
    }
    let name = yaml_field(&fm, "name")?;
    let description = yaml_field(&fm, "description").unwrap_or_default();
    Some(SkillInfo {
        name,
        description,
        path: path.to_string_lossy().to_string(),
    })
}

/// Extract a field from a YAML frontmatter string. Handles simple scalars and
/// folded (`>-`) / literal (`|-`) block scalars.
fn yaml_field(fm: &str, field: &str) -> Option<String> {
    let lines: Vec<&str> = fm.lines().collect();
    let prefix = format!("{}:", field);
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with(&prefix) {
            continue;
        }
        let value = trimmed[prefix.len()..].trim();
        // simple scalar: `name: find-docs`
        if !value.is_empty() && !value.starts_with('>') && !value.starts_with('|') {
            return Some(value.trim_matches('"').trim_matches('\'').to_string());
        }
        // block scalar: `description: >-` or `description: |-`
        if value.starts_with('>') || value.starts_with('|') {
            let literal = value.starts_with('|');
            let mut parts: Vec<&str> = vec![];
            for next in &lines[i + 1..] {
                if next.trim().is_empty() {
                    parts.push("");
                    continue;
                }
                if next.starts_with(' ') || next.starts_with('\t') {
                    parts.push(next.trim());
                } else {
                    break;
                }
            }
            while parts.last().is_some_and(|s| s.is_empty()) {
                parts.pop();
            }
            let joiner = if literal { "\n" } else { " " };
            return Some(parts.join(joiner));
        }
        return Some(value.trim_matches('"').trim_matches('\'').to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_frontmatter() {
        let tmp = std::env::temp_dir().join(format!("skill-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let path = tmp.join("SKILL.md");
        std::fs::write(
            &path,
            "---\nname: my-skill\ndescription: A simple skill\n---\n\n# Body\nContent here.",
        )
        .unwrap();
        let s = parse(&path).unwrap();
        assert_eq!(s.name, "my-skill");
        assert_eq!(s.description, "A simple skill");
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn test_parse_folded_description() {
        let tmp = std::env::temp_dir().join(format!("skill-fold-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let path = tmp.join("SKILL.md");
        std::fs::write(
            &path,
            "---\nname: find-docs\ndescription: >-\n  Retrieves up-to-date\n  documentation for libraries.\n  Always use for API questions.\n---\n\nBody.",
        )
        .unwrap();
        let s = parse(&path).unwrap();
        assert_eq!(s.name, "find-docs");
        assert_eq!(
            s.description,
            "Retrieves up-to-date documentation for libraries. Always use for API questions."
        );
        std::fs::remove_dir_all(&tmp).ok();
    }
}