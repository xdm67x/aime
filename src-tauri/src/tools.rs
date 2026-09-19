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
                "name": "read_file",
                "description": "Read the contents of a file at the given path.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "Path to the file."}
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
                "description": "Replace the first occurrence of old_string with new_string in a file.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "Path to the file."},
                        "old_string": {"type": "string", "description": "Exact text to find."},
                        "new_string": {"type": "string", "description": "Replacement text."}
                    },
                    "required": ["path", "old_string", "new_string"]
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

/// Execute a tool call. Returns the output string (Err for failures).
pub async fn execute(name: &str, arguments: &str) -> Result<String, String> {
    let args: serde_json::Value = serde_json::from_str(arguments).unwrap_or_default();
    match name {
        "read_file" => read_file(&args),
        "write_file" => write_file(&args),
        "edit_file" => edit_file(&args),
        "grep" => run_grep(&args),
        "bash" => run_bash(&args).await,
        other if other.starts_with("skill_") => {
            let skill_name = other.strip_prefix("skill_").unwrap_or(other);
            skills::load_content(skill_name)
        }
        _ => Err(format!("Unknown tool: {name}")),
    }
}

fn read_file(args: &serde_json::Value) -> Result<String, String> {
    let path = args["path"].as_str().ok_or("missing 'path'")?;
    let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    Ok(truncate(content, 50_000))
}

fn write_file(args: &serde_json::Value) -> Result<String, String> {
    let path = args["path"].as_str().ok_or("missing 'path'")?;
    let content = args["content"].as_str().ok_or("missing 'content'")?;
    if let Some(parent) = std::path::Path::new(path).parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, content).map_err(|e| e.to_string())?;
    Ok(format!("Wrote {} bytes to {path}", content.len()))
}

fn edit_file(args: &serde_json::Value) -> Result<String, String> {
    let path = args["path"].as_str().ok_or("missing 'path'")?;
    let old = args["old_string"].as_str().ok_or("missing 'old_string'")?;
    let new = args["new_string"].as_str().ok_or("missing 'new_string'")?;
    let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let count = content.matches(old).count();
    if count == 0 {
        return Err("old_string not found in file".into());
    }
    let new_content = content.replacen(old, new, 1);
    std::fs::write(path, new_content).map_err(|e| e.to_string())?;
    Ok(format!("Replaced 1 of {count} occurrence(s) in {path}"))
}

fn run_grep(args: &serde_json::Value) -> Result<String, String> {
    let pattern = args["pattern"].as_str().ok_or("missing 'pattern'")?;
    let path = args["path"].as_str().unwrap_or(".");
    let glob = args["glob"].as_str();
    // Prefer ripgrep (faster, better defaults); fall back to grep -rn.
    let rg = std::process::Command::new("rg")
        .arg("--line-number")
        .arg("--no-heading")
        .arg("--color")
        .arg("never")
        .args(glob.map(|g| vec!["--glob", g]).unwrap_or_default())
        .arg(pattern)
        .arg(path)
        .output();
    match rg {
        Ok(o) if !o.stdout.is_empty() => {
            return Ok(truncate(String::from_utf8_lossy(&o.stdout).to_string(), 10_000));
        }
        _ => {}
    }
    // Fallback: grep -rn
    let mut cmd = std::process::Command::new("grep");
    cmd.arg("-rn").arg("--color=never");
    if let Some(g) = glob {
        cmd.arg("--include").arg(g);
    }
    cmd.arg(pattern).arg(path);
    let o = cmd.output().map_err(|e| e.to_string())?;
    if !o.stdout.is_empty() {
        Ok(truncate(String::from_utf8_lossy(&o.stdout).to_string(), 10_000))
    } else if o.status.success() {
        Ok("(no matches)".into())
    } else {
        Err(String::from_utf8_lossy(&o.stderr).to_string())
    }
}

/// Execute a shell command with a 30-second timeout.
async fn run_bash(args: &serde_json::Value) -> Result<String, String> {
    let command = args["command"].as_str().ok_or("missing 'command'")?.to_string();
    let child = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(&command)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true) // dropped child on timeout → process killed
        .spawn()
        .map_err(|e| e.to_string())?;
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

fn truncate(s: String, max: usize) -> String {
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
        write_file(&json!({"path": file.to_str().unwrap(), "content": "hello world"})).unwrap();

        // read
        let content = read_file(&json!({"path": file.to_str().unwrap()})).unwrap();
        assert_eq!(content, "hello world");

        // edit
        edit_file(&json!({
            "path": file.to_str().unwrap(),
            "old_string": "hello",
            "new_string": "goodbye"
        }))
        .unwrap();
        let content = read_file(&json!({"path": file.to_str().unwrap()})).unwrap();
        assert_eq!(content, "goodbye world");

        // edit: not found
        let err = edit_file(&json!({
            "path": file.to_str().unwrap(),
            "old_string": "nonexistent",
            "new_string": "x"
        }));
        assert!(err.is_err());

        std::fs::remove_dir_all(&tmp).ok();
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