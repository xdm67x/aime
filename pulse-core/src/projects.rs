//! Projects: a local directory or a GitHub repo cloned via `gh`, that beats
//! (sessions) can be attached to. A beat attached to a project runs with the
//! project directory as its working directory — tool paths resolve relative to
//! it, `bash` runs inside it, and its AGENTS.md (if any) is injected into the
//! session prompt.

use crate::{db, tools};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;

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
    // a beat with a worktree runs in the worktree, not the project checkout
    let path: Option<String> = c
        .query_row(
            "SELECT COALESCE(b.worktree, p.path) FROM beats b \
             LEFT JOIN projects p ON p.id = b.project_id WHERE b.id = ?1",
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

/* ---- git worktrees: a beat spawned from a project works in a dedicated
worktree under ~/.pulse/worktrees so its edits never touch the main repo
checkout. Created with the beat, dropped when the archived beat is deleted. ---- */

fn worktrees_base() -> Result<PathBuf, String> {
    let home = std::env::var("HOME").map_err(|e| e.to_string())?;
    let base = Path::new(&home).join(".pulse").join("worktrees");
    std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    Ok(base)
}

/// Branch/worktree name from a task name: whitespace and `/` → hyphens, then
/// only branch-safe characters survive (alnum, `-`, `_`, `.`); repeats
/// collapse, `..` (illegal in git refs) flattens, and edges are trimmed so no
/// awkward characters can slip into the branch name.
pub fn slug(name: &str) -> String {
    let s: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_whitespace() || c == '/' {
                '-'
            } else {
                c
            }
        })
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .collect();
    let mut s = s
        .trim_matches(|c: char| c == '-' || c == '.')
        .replace("..", "-")
        .replace("--", "-");
    while s.contains("--") {
        s = s.replace("--", "-");
    }
    if s.is_empty() {
        "beat".into()
    } else {
        s
    }
}

fn branch_exists(project: &str, branch: &str) -> bool {
    Command::new("git")
        .args([
            "-C",
            project,
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Create a git worktree for a freshly created beat, branching off the
/// project's current HEAD. Returns a status message either way (the UI shows
/// it); a failure never blocks beat creation — the beat then just runs in the
/// project directory itself.
pub fn create_worktree(beat_id: i64, name: &str, project_path: &str) -> String {
    match worktrees_base() {
        Ok(b) => create_worktree_in(b, beat_id, name, project_path),
        Err(e) => format!("Worktree failed: {e}"),
    }
}

fn create_worktree_in(base: PathBuf, beat_id: i64, name: &str, project_path: &str) -> String {
    match create_worktree_git(base, beat_id, name, project_path) {
        Ok((branch, path)) => {
            if let Err(e) = db::open().and_then(|c| {
                c.execute(
                    "UPDATE beats SET worktree = ?1 WHERE id = ?2",
                    params![path, beat_id],
                )
                .map_err(|e| e.to_string())
            }) {
                let _ = std::fs::remove_dir_all(&path);
                return format!("Worktree failed: {e}");
            }
            format!("Worktree ready: {path} (branch {branch})")
        }
        Err(e) => e,
    }
}

/// The git side only: returns (branch, worktree path). No db touched.
fn create_worktree_git(
    base: PathBuf,
    beat_id: i64,
    name: &str,
    project_path: &str,
) -> Result<(String, String), String> {
    if !Path::new(project_path).join(".git").exists() {
        return Err(format!(
            "Worktree skipped: {project_path} is not a git repo"
        ));
    }
    let mut branch = slug(name);
    let mut dest = base.join(&branch);
    if branch_exists(project_path, &branch) || dest.exists() {
        branch = format!("{}-{beat_id}", slug(name));
        dest = base.join(&branch);
    }
    let out = Command::new("git")
        .args([
            "-C",
            project_path,
            "worktree",
            "add",
            "-b",
            &branch,
            &dest.to_string_lossy(),
        ])
        .output();
    let out = match out {
        Ok(o) => o,
        Err(e) => return Err(format!("Worktree failed: git: {e}")),
    };
    if !out.status.success() {
        return Err(format!(
            "Worktree failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok((branch, dest.to_string_lossy().to_string()))
}

/// Remove a beat's worktree directory from disk, cleanly unregistered from the
/// parent repo when possible. Returns a status message for the UI.
pub fn remove_worktree(path: &str, project: Option<&str>) -> String {
    if !Path::new(path).exists() {
        return format!("Worktree {path} already gone");
    }
    let repo = project.filter(|p| Path::new(p).join(".git").exists());
    if let Some(p) = repo {
        if let Ok(o) = Command::new("git")
            .args(["-C", p, "worktree", "remove", "--force", path])
            .output()
        {
            if o.status.success() {
                return format!("Worktree dropped: {path}");
            }
        }
    }
    // parent repo gone or `git worktree remove` refused — delete + prune
    match std::fs::remove_dir_all(path) {
        Ok(_) => {
            if let Some(p) = repo {
                let _ = Command::new("git")
                    .args(["-C", p, "worktree", "prune"])
                    .output();
            }
            format!("Worktree dropped: {path}")
        }
        Err(e) => format!("Worktree drop failed: {e}"),
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

    #[test]
    fn test_slug() {
        assert_eq!(slug("Fix Login Flow"), "Fix-Login-Flow");
        assert_eq!(slug("  weird  name!! @# "), "weird-name");
        assert_eq!(slug("--leading and trailing--"), "leading-and-trailing");
        assert_eq!(slug("a/b..c"), "a-b-c");
        assert_eq!(slug("   "), "beat");
    }

    #[test]
    fn test_worktree_lifecycle() {
        let tmp = std::env::temp_dir().join(format!("pulse-wt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let proj = tmp.join("proj");
        std::fs::create_dir_all(&proj).unwrap();
        let git = |args: &[&str]| {
            let out = Command::new("git")
                .arg("-C")
                .arg(&proj)
                .args(args)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {:?}: {}",
                args,
                String::from_utf8_lossy(&out.stderr)
            );
        };
        git(&["init", "-q"]);
        git(&[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "--allow-empty",
            "-m",
            "init",
        ]);

        let base = tmp.join("wts");
        let msg = create_worktree_in(base.clone(), 1, "Fix Login Flow", &proj.to_string_lossy());
        assert!(msg.contains("Worktree ready"), "{msg}");
        assert!(msg.contains("branch Fix-Login-Flow"), "{msg}");
        assert!(base.join("Fix-Login-Flow").is_dir());
        assert!(branch_exists(&proj.to_string_lossy(), "Fix-Login-Flow"));

        // drop it: dir gone, git no longer lists the worktree
        let msg = remove_worktree(
            &base.join("Fix-Login-Flow").to_string_lossy(),
            Some(&proj.to_string_lossy()),
        );
        assert!(msg.contains("Worktree dropped"), "{msg}");
        assert!(!base.join("Fix-Login-Flow").exists());
        let listed = Command::new("git")
            .arg("-C")
            .arg(&proj)
            .args(["worktree", "list"])
            .output()
            .unwrap();
        let listed = String::from_utf8_lossy(&listed.stdout);
        assert!(!listed.contains("Fix-Login-Flow"));

        // non-git project → skipped, no panic
        let msg = create_worktree_in(base, 2, "x", "/");
        assert!(msg.contains("skipped"), "{msg}");

        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
