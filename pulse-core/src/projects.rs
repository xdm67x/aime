//! Projects: a local directory or a GitHub repo cloned via `gh`, that beats
//! (sessions) can be attached to. A beat attached to a project runs with the
//! project directory as its working directory — tool paths resolve relative to
//! it, `bash` runs inside it, and its AGENTS.md (if any) is injected into the
//! session prompt.

use crate::{db, tools};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::Serialize;
use std::path::Path;

#[derive(Serialize)]
pub struct Project {
    pub id: i64,
    pub name: String,
    pub path: String,
    /// "local" or "github"
    pub source: String,
    pub created_at: String,
}

fn row_to_project(row: &Row) -> rusqlite::Result<Project> {
    Ok(Project {
        id: row.get(0)?,
        name: row.get(1)?,
        path: row.get(2)?,
        source: row.get(3)?,
        created_at: row.get(4)?,
    })
}

const SELECT: &str = "SELECT id, name, path, source, created_at FROM projects";

fn conn() -> Result<Connection, String> {
    let c = db::open()?;
    c.pragma_update(None, "foreign_keys", "ON")
        .map_err(|e| e.to_string())?;
    Ok(c)
}

/// Validate a project directory exists and looks like a project root.
fn validate_dir(path: &str) -> Result<(), String> {
    let p = Path::new(path);
    if !p.is_dir() {
        return Err(format!("Not a directory: {path}"));
    }
    Ok(())
}

pub fn list_projects() -> Result<Vec<Project>, String> {
    let c = conn()?;
    let mut stmt = c
        .prepare(&format!("{SELECT} ORDER BY id DESC"))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], row_to_project)
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

pub fn add_project(path: &str) -> Result<Project, String> {
    let path = path.trim().trim_end_matches('/').to_string();
    validate_dir(&path)?;
    let name = Path::new(&path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("project")
        .to_string();
    let c = conn()?;
    c.execute(
        "INSERT INTO projects (name, path, source) VALUES (?1, ?2, 'local')",
        params![name, path],
    )
    .map_err(|e| {
        if e.to_string().contains("UNIQUE") {
            format!("Project already added: {path}")
        } else {
            e.to_string()
        }
    })?;
    let id = c.last_insert_rowid();
    get_project(id)
}

fn get_project(id: i64) -> Result<Project, String> {
    let c = conn()?;
    c.query_row(
        &format!("{SELECT} WHERE id = ?1"),
        params![id],
        row_to_project,
    )
    .optional()
    .map_err(|e| e.to_string())?
    .ok_or_else(|| "Project not found".to_string())
}

/// Clone a GitHub repo (`owner/name` or a full URL) with `gh repo clone` into
/// `~/.pulse/repos/<name>` and register it. Returns the new project.
pub async fn clone_project(repo: &str) -> Result<Project, String> {
    let repo = repo.trim().to_string();
    if repo.is_empty() {
        return Err("Repository is empty".into());
    }
    // derive the folder name from owner/name or the URL's last segment
    let name = repo
        .trim_end_matches('/')
        .rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or("repo")
        .trim_end_matches(".git")
        .to_string();
    if name.is_empty() || name.contains("..") {
        return Err(format!("Cannot derive a project name from '{repo}'"));
    }
    let home = std::env::var("HOME").map_err(|e| e.to_string())?;
    let base = Path::new(&home).join(".pulse").join("repos");
    std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    let dest = base.join(&name);
    if dest.exists() {
        return Err(format!(
            "Directory already exists: {} — add it as a local project instead",
            dest.display()
        ));
    }
    let output = tokio::process::Command::new("gh")
        .args(["repo", "clone", &repo, &dest.to_string_lossy()])
        .output()
        .await
        .map_err(|e| format!("Failed to run `gh` (is the GitHub CLI installed?): {e}"))?;
    if !output.status.success() {
        let _ = std::fs::remove_dir_all(&dest);
        return Err(format!(
            "gh repo clone failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let path = dest.to_string_lossy().to_string();
    let c = conn()?;
    c.execute(
        "INSERT INTO projects (name, path, source) VALUES (?1, ?2, 'github')",
        params![name, path],
    )
    .map_err(|e| {
        let _ = std::fs::remove_dir_all(&dest);
        if e.to_string().contains("UNIQUE") {
            format!("Project already added: {path}")
        } else {
            e.to_string()
        }
    })?;
    let id = c.last_insert_rowid();
    get_project(id)
}

/// Remove a project from Pulse. The directory on disk is left untouched; beats
/// attached to it keep running but lose their working directory.
pub fn remove_project(id: i64) -> Result<(), String> {
    let c = conn()?;
    c.execute("DELETE FROM projects WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Resolve a beat's project directory, if it has one. Returns an error if the
/// project was deleted or its directory no longer exists.
pub fn working_dir(beat_id: i64) -> Result<Option<String>, String> {
    let c = conn()?;
    let path: Option<String> = c
        .query_row(
            "SELECT p.path FROM beats b JOIN projects p ON p.id = b.project_id WHERE b.id = ?1",
            params![beat_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    match path {
        Some(p) => {
            if !Path::new(&p).is_dir() {
                return Err(format!("Project directory no longer exists: {p}"));
            }
            Ok(Some(p))
        }
        None => Ok(None),
    }
}

/// The project's AGENTS.md content, if any — injected as session instructions.
pub fn agents_note(dir: &str) -> Option<String> {
    let p = Path::new(dir).join("AGENTS.md");
    std::fs::read_to_string(&p).ok().map(|s| {
        format!(
            "## Project instructions (AGENTS.md in {})\n\n{}",
            dir,
            tools::truncate(s, 8_000)
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agents_note() {
        // no AGENTS.md → None
        assert!(agents_note("/").is_none());
    }

    #[test]
    fn test_name_derivation() {
        // mirrors the rsplit logic in clone_project
        let name_of = |repo: &str| -> String {
            repo.trim_end_matches('/')
                .rsplit('/')
                .find(|s| !s.is_empty())
                .unwrap_or("repo")
                .trim_end_matches(".git")
                .to_string()
        };
        assert_eq!(name_of("vercel/next.js"), "next.js");
        assert_eq!(name_of("https://github.com/foo/bar.git"), "bar");
        assert_eq!(name_of("foo/bar/"), "bar");
    }
}
