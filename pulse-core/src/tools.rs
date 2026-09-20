//! Tool definitions and execution for the agentic loop.
//!
//! Five core tools (`read_file`, `write_file`, `edit_file`, `grep`, `bash`)
//! plus one tool per discovered skill (`skill_<name>`). Skill tools load the
//! full `SKILL.md` on demand; only the frontmatter description is sent to the
//! model up front.

use crate::skills;
use serde::Serialize;
use serde_json::json;
use std::time::Duration;

/// One executed tool step, for display in the transcript.
#[derive(Serialize)]
pub struct ToolStep {
    pub tool: String,
    pub arguments: String,
    pub result: String,
    pub error: bool,
}

/// Build the OpenRouter `tools` array: core tools + one per skill.
pub fn definitions(skills: &[skills::SkillInfo]) -> Vec<serde_json::Value> {
    let mut tools = vec![
        json!({
            "type": "function",
            "function": {
                "name": "task_complete",
                "description": "Signal that the task is fully finished and no further work is needed. Call this — and only this — as your final action, once every part of the task is verified done. Include the complete final answer for the user. Do NOT call it while work remains (unverified changes, failing tests, unanswered parts of the request).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "summary": {"type": "string", "description": "The complete final answer / summary of the work for the user. Markdown allowed."}
                    },
                    "required": ["summary"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "read_file",
                "description": "Read the contents of a file at the given path. Optionally read a line range (1-indexed, inclusive).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "Path to the file."},
                        "offset": {"type": "integer", "description": "First line to read (1-indexed). Optional."},
                        "limit": {"type": "integer", "description": "Number of lines to read from offset. Optional."}
                    },
                    "required": ["path"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "write_file",
                "description": "Write content to a file, creating or overwriting it.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "Path to the file."},
                        "content": {"type": "string", "description": "Content to write."}
                    },
                    "required": ["path", "content"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "edit_file",
                "description": "Edit a file in one of two ways: (a) replace the first occurrence of old_string with new_string, or (b) replace the line range start_line..end_line (1-indexed, inclusive) with new_string.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "Path to the file."},
                        "old_string": {"type": "string", "description": "Exact text to find (string mode)."},
                        "new_string": {"type": "string", "description": "Replacement text."},
                        "start_line": {"type": "integer", "description": "First line to replace (1-indexed). Line mode when set; requires new_string, old_string not needed."},
                        "end_line": {"type": "integer", "description": "Last line to replace (1-indexed, inclusive). Optional; defaults to start_line."}
                    },
                    "required": ["path", "new_string"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "grep",
                "description": "Search files for a regex pattern. Returns matching lines with file paths and line numbers.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "pattern": {"type": "string", "description": "Regex pattern to search for."},
                        "path": {"type": "string", "description": "Directory or file to search in. Defaults to current directory."},
                        "glob": {"type": "string", "description": "Optional file glob filter, e.g. *.rs"}
                    },
                    "required": ["pattern"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "bash",
                "description": "Execute a shell command and return its output. Use for search, running scripts, or any command-line task.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "command": {"type": "string", "description": "Shell command to execute."}
                    },
                    "required": ["command"]
                }
            }
        }),
    ];
    for skill in skills {
        tools.push(json!({
            "type": "function",
            "function": {
                "name": format!("skill_{}", skill.name),
                "description": format!("Load the full instructions for the '{}' skill. Use when the task matches this skill's description.\n\n{}", skill.name, skill.description),
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        }));
    }
    tools
}

/// Execute a tool call. `cwd` is the beat's project directory (if any):
/// relative paths resolve against it and `bash`/`grep` run inside it.
/// Returns the output string (Err for failures).
pub async fn execute(name: &str, arguments: &str, cwd: Option<&str>) -> Result<String, String> {
    let args: serde_json::Value = serde_json::from_str(arguments).unwrap_or_default();
    match name {
        "read_file" => read_file(&args, cwd),
        "write_file" => write_file(&args, cwd),
        "edit_file" => edit_file(&args, cwd),
        "grep" => run_grep(&args, cwd),
        "bash" => run_bash(&args, cwd).await,
        other if other.starts_with("skill_") => {
            let skill_name = other.strip_prefix("skill_").unwrap_or(other);
            skills::load_content(skill_name)
        }
        _ => Err(format!("Unknown tool: {name}")),
    }
}

/// Resolve a tool path against the working directory when it's relative.
fn resolve(path: &str, cwd: Option<&str>) -> String {
    let p = std::path::Path::new(path);
    match (p.is_absolute(), cwd) {
        (true, _) | (false, None) => path.to_string(),
        (false, Some(dir)) => std::path::Path::new(dir)
            .join(p)
            .to_string_lossy()
            .into_owned(),
    }
}

fn read_file(args: &serde_json::Value, cwd: Option<&str>) -> Result<String, String> {
    let path = resolve(args["path"].as_str().ok_or("missing 'path'")?, cwd);
    let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let offset = args["offset"].as_u64().unwrap_or(1).max(1) as usize;
    let limit = args["limit"].as_u64().map(|l| l as usize);
    if offset == 1 && limit.is_none() {
        return Ok(truncate(content, 50_000));
    }
    let lines: Vec<&str> = content.lines().collect();
    let start = (offset - 1).min(lines.len());
    let end = limit
        .map(|l| (start + l).min(lines.len()))
        .unwrap_or(lines.len());
    if start >= lines.len() {
        return Err(format!(
            "offset {} is past end of file ({} lines)",
            offset,
            lines.len()
        ));
    }
    Ok(lines[start..end].join("\n"))
}

fn write_file(args: &serde_json::Value, cwd: Option<&str>) -> Result<String, String> {
    let path = resolve(args["path"].as_str().ok_or("missing 'path'")?, cwd);
    let content = args["content"].as_str().ok_or("missing 'content'")?;
    if let Some(parent) = std::path::Path::new(&path).parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, content).map_err(|e| e.to_string())?;
    Ok(format!("Wrote {} bytes to {path}", content.len()))
}

fn edit_file(args: &serde_json::Value, cwd: Option<&str>) -> Result<String, String> {
    let path = resolve(args["path"].as_str().ok_or("missing 'path'")?, cwd);
    let new = args["new_string"].as_str().ok_or("missing 'new_string'")?;
    let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;

    // Line mode: replace lines start..=end (1-indexed, inclusive) with new_string.
    if let Some(start) = args["start_line"].as_u64() {
        let start = start.max(1) as usize;
        let end = args["end_line"].as_u64().unwrap_or(start as u64).max(1) as usize;
        if end < start {
            return Err(format!("end_line {} is before start_line {}", end, start));
        }
        let mut lines: Vec<&str> = content.lines().collect();
        if start > lines.len() {
            return Err(format!(
                "start_line {} is past end of file ({} lines)",
                start,
                lines.len()
            ));
        }
        let s = start - 1;
        let e = end.min(lines.len());
        lines.splice(s..e, new.lines());
        let mut out = lines.join("\n");
        if content.ends_with('\n') {
            out.push('\n');
        }
        std::fs::write(&path, out).map_err(|e| e.to_string())?;
        return Ok(format!("Replaced lines {}-{} in {path}", start, e));
    }

    // String mode: replace first occurrence of old_string.
    let old = args["old_string"]
        .as_str()
        .ok_or("missing 'old_string' (or set start_line)")?;
    let count = content.matches(old).count();
    if count == 0 {
        return Err("old_string not found in file".into());
    }
    let new_content = content.replacen(old, new, 1);
    std::fs::write(&path, new_content).map_err(|e| e.to_string())?;
    Ok(format!("Replaced 1 of {count} occurrence(s) in {path}"))
}

fn run_grep(args: &serde_json::Value, cwd: Option<&str>) -> Result<String, String> {
    let pattern = args["pattern"].as_str().ok_or("missing 'pattern'")?;
    let path = resolve(args["path"].as_str().unwrap_or("."), cwd);
    let glob = args["glob"].as_str();
    // Prefer ripgrep (faster, better defaults); fall back to grep -rn.
    let rg = std::process::Command::new("rg")
        .arg("--line-number")
        .arg("--no-heading")
        .arg("--color")
        .arg("never")
        .args(glob.map(|g| vec!["--glob", g]).unwrap_or_default())
        .arg(pattern)
        .arg(&path)
        .output();
    match rg {
        Ok(o) if !o.stdout.is_empty() => {
            return Ok(truncate(
                String::from_utf8_lossy(&o.stdout).to_string(),
                10_000,
            ));
        }
        _ => {}
    }
    // Fallback: grep -rn
    let mut cmd = std::process::Command::new("grep");
    cmd.arg("-rn").arg("--color=never");
    if let Some(g) = glob {
        cmd.arg("--include").arg(g);
    }
    cmd.arg(pattern).arg(&path);
    let o = cmd.output().map_err(|e| e.to_string())?;
    if !o.stdout.is_empty() {
        Ok(truncate(
            String::from_utf8_lossy(&o.stdout).to_string(),
            10_000,
        ))
    } else if o.status.success() {
        Ok("(no matches)".into())
    } else {
        Err(String::from_utf8_lossy(&o.stderr).to_string())
    }
}

/// The user's default shell, e.g. from `$SHELL` or `/etc/passwd`; falls back
/// to `sh`. Used so spawned commands see the user's normal (login) PATH.
fn default_shell() -> String {
    if let Ok(shell) = std::env::var("SHELL") {
        if !shell.is_empty() {
            return shell;
        }
    }
    // Fall back to the passwd entry for the current user (Unix).
    #[cfg(unix)]
    {
        if let Some(passwd) = std::env::var("USER")
            .ok()
            .and_then(|u| passwd_shell(&u))
        {
            return passwd;
        }
    }
    "sh".into()
}

#[cfg(unix)]
fn passwd_shell(user: &str) -> Option<String> {
    let passwd = std::fs::read_to_string("/etc/passwd").ok()?;
    passwd.lines().find_map(|line| {
        let mut fields = line.split(':');
        (fields.next()? == user).then(|| fields.nth(5).map(|s| s.to_string())).flatten()
    })
}

/// Execute a shell command with a 30-second timeout, inside `cwd` when set.
async fn run_bash(args: &serde_json::Value, cwd: Option<&str>) -> Result<String, String> {
    let command = args["command"]
        .as_str()
        .ok_or("missing 'command'")?
        .to_string();
    let mut cmd = tokio::process::Command::new(default_shell());
    // Login shell so the user's PATH (e.g. Homebrew) applies.
    cmd.arg("-l")
        .arg("-c")
        .arg(&command)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true); // dropped child on timeout → process killed
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let child = cmd.spawn().map_err(|e| e.to_string())?;
    match tokio::time::timeout(Duration::from_secs(30), child.wait_with_output()).await {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            if output.status.success() {
                Ok(truncate(stdout, 10_000))
            } else {
                Err(format!(
                    "exit {:?}: {}",
                    output.status.code(),
                    truncate(stderr, 5_000)
                ))
            }
        }
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("Command timed out (30s)".into()),
    }
}

pub(crate) fn truncate(s: String, max: usize) -> String {
    if s.len() <= max {
        s
    } else {
        format!("{}\n… (truncated, {} total chars)", &s[..max], s.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_write_edit() {
        let tmp = std::env::temp_dir().join(format!("tools-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let file = tmp.join("test.txt");

        // write
        write_file(
            &json!({"path": file.to_str().unwrap(), "content": "hello world"}),
            None,
        )
        .unwrap();

        // read
        let content = read_file(&json!({"path": file.to_str().unwrap()}), None).unwrap();
        assert_eq!(content, "hello world");

        // edit
        edit_file(
            &json!({
                "path": file.to_str().unwrap(),
                "old_string": "hello",
                "new_string": "goodbye"
            }),
            None,
        )
        .unwrap();
        let content = read_file(&json!({"path": file.to_str().unwrap()}), None).unwrap();
        assert_eq!(content, "goodbye world");

        // edit: not found
        let err = edit_file(
            &json!({
                "path": file.to_str().unwrap(),
                "old_string": "nonexistent",
                "new_string": "x"
            }),
            None,
        );
        assert!(err.is_err());

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn test_line_numbers() {
        let tmp = std::env::temp_dir().join(format!("tools-linenum-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let file = tmp.join("lines.txt");
        write_file(
            &json!({"path": file.to_str().unwrap(), "content": "a\nb\nc\nd\ne\n"}),
            None,
        )
        .unwrap();

        // read with offset/limit
        let content = read_file(
            &json!({"path": file.to_str().unwrap(), "offset": 2, "limit": 2}),
            None,
        )
        .unwrap();
        assert_eq!(content, "b\nc");

        // read: offset past EOF errors
        assert!(read_file(&json!({"path": file.to_str().unwrap(), "offset": 99}), None).is_err());

        // edit: single line replace
        edit_file(
            &json!({"path": file.to_str().unwrap(), "start_line": 3, "new_string": "C!"}),
            None,
        )
        .unwrap();
        let content = read_file(&json!({"path": file.to_str().unwrap()}), None).unwrap();
        assert_eq!(content, "a\nb\nC!\nd\ne\n");

        // edit: range replace with fewer lines
        edit_file(
            &json!({"path": file.to_str().unwrap(), "start_line": 4, "end_line": 5, "new_string": "x\ny\nz"}),
            None,
        )
        .unwrap();
        let content = read_file(&json!({"path": file.to_str().unwrap()}), None).unwrap();
        assert_eq!(content, "a\nb\nC!\nx\ny\nz\n");

        // edit: end_line before start_line errors
        let err = edit_file(
            &json!({"path": file.to_str().unwrap(), "start_line": 5, "end_line": 2, "new_string": "q"}),
            None,
        );
        assert!(err.is_err());

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn test_resolve() {
        // absolute paths pass through, relative ones join the cwd
        assert_eq!(resolve("/tmp/x", Some("/proj")), "/tmp/x");
        assert_eq!(resolve("src/main.rs", Some("/proj")), "/proj/src/main.rs");
        assert_eq!(resolve("src/main.rs", None), "src/main.rs");
    }

    #[test]
    fn test_truncate() {
        assert_eq!(truncate("short".into(), 100), "short");
        let long = "x".repeat(200);
        let t = truncate(long.clone(), 50);
        assert!(t.contains("truncated"));
        assert!(t.contains("200 total chars"));
    }
}
