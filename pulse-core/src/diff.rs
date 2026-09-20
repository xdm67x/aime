//! Line diffs + language detection for file-edit tools.
//!
//! `write_file` / `edit_file` return a unified-diff snippet of what changed so
//! the UI can render a git-style view (additions in green, deletions in red)
//! with syntax highlighting. Pure std — no external diff crate needed.

/// One line of a unified diff, tagged as context / added / removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLine {
    Context,
    Added,
    Removed,
}

/// Produce a unified-diff body (no `---`/`+++` headers) between two texts.
/// LCS-based, so it finds real line moves instead of a full rewrite blowup.
/// Output is capped at `max_lines` (older hunks are dropped first, newest
/// changes matter most) with a `…` marker where content was elided.
pub fn unified_diff(before: &str, after: &str, max_lines: usize) -> String {
    let a: Vec<&str> = before.lines().collect();
    let b: Vec<&str> = after.lines().collect();

    // LCS table
    let n = a.len();
    let m = b.len();
    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if a[i] == b[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }

    // Walk it into tagged lines
    let mut out: Vec<(DiffLine, &str)> = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if a[i] == b[j] {
            out.push((DiffLine::Context, a[i]));
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            out.push((DiffLine::Removed, a[i]));
            i += 1;
        } else {
            out.push((DiffLine::Added, b[j]));
            j += 1;
        }
    }
    while i < n {
        out.push((DiffLine::Removed, a[i]));
        i += 1;
    }
    while j < m {
        out.push((DiffLine::Added, b[j]));
        j += 1;
    }

    if out.iter().all(|(k, _)| *k == DiffLine::Context) {
        return String::new(); // nothing actually changed
    }

    // Cap: keep the tail (most recent edits), collapse the head with a marker.
    let elided = out.len().saturating_sub(max_lines);
    if elided > 0 {
        out.drain(..elided);
    }
    let mut s = String::new();
    if elided > 0 {
        s.push_str(&format!("… ({elided} unchanged/earlier lines elided)\n"));
    }
    for (kind, line) in &out {
        let tag = match kind {
            DiffLine::Context => ' ',
            DiffLine::Added => '+',
            DiffLine::Removed => '-',
        };
        s.push(tag);
        s.push_str(line);
        s.push('\n');
    }
    s
}

/// Trailing-newline note for diff display: `write` to a file that ends
/// without a newline but the new content does (or vice versa) is invisible to
/// `lines()`, so callers pass raw strings and we surface it here.
pub fn newline_change(before: &str, after: &str) -> Option<&'static str> {
    match (before.ends_with('\n'), after.ends_with('\n')) {
        (false, true) => Some("\\ No newline at end of file → newline added"),
        (true, false) => Some("\\ No newline at end of file"),
        _ => None,
    }
}

/// Map a file extension to a highlight.js language name (subset covering the
/// languages agents most often edit). Unknown → `None` (plain diff).
pub fn lang_for_path(path: &str) -> Option<&'static str> {
    let ext = path.rsplit('.').next()?;
    if ext == path {
        return None; // no dot in the name (e.g. Makefile handled below)
    }
    Some(match ext {
        "rs" => "rust",
        "ts" | "tsx" | "mts" | "cts" => "typescript",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "py" => "python",
        "go" => "go",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" => "cpp",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "swift" => "swift",
        "rb" => "ruby",
        "php" => "php",
        "cs" => "csharp",
        "sh" | "bash" | "zsh" => "bash",
        "json" => "json",
        "yaml" | "yml" => "yaml",
        "toml" => "ini",
        "md" => "markdown",
        "html" | "htm" => "xml",
        "css" => "css",
        "scss" | "sass" => "scss",
        "sql" => "sql",
        "xml" | "svg" => "xml",
        "lua" => "lua",
        "zig" => "zig",
        "dart" => "dart",
        "ex" | "exs" => "elixir",
        "hs" => "haskell",
        "scala" => "scala",
        "pl" | "pm" => "perl",
        "vue" => "xml",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_marks_added_and_removed() {
        let d = unified_diff("a\nb\nc", "a\nB\nc", 100);
        assert!(d.contains("-b"));
        assert!(d.contains("+B"));
        assert!(d.contains(" a"));
        assert!(d.contains(" c"));
    }

    #[test]
    fn identical_files_give_empty_diff() {
        assert_eq!(unified_diff("same\nlines", "same\nlines", 100), "");
    }

    #[test]
    fn cap_keeps_tail_and_notes_elision() {
        let before = "1\n2\n3\n4\n5\n6\n7\n8\n9\n10";
        let after = "1\n2\n3\n4\n5\n6\n7\n8\n9\nX";
        let d = unified_diff(before, after, 4);
        assert!(d.contains("elided"));
        assert!(d.contains("-10"));
        assert!(d.contains("+X"));
        assert!(!d.contains("-1\n"));
    }

    #[test]
    fn write_of_new_file_is_all_additions() {
        let d = unified_diff("", "one\ntwo", 100);
        assert!(d.contains("+one"));
        assert!(d.contains("+two"));
        assert!(!d.contains('-'));
    }

    #[test]
    fn lang_detection() {
        assert_eq!(lang_for_path("src/main.rs"), Some("rust"));
        assert_eq!(lang_for_path("app.tsx"), Some("typescript"));
        assert_eq!(lang_for_path("Makefile"), None);
        assert_eq!(lang_for_path("unknown.xyz"), None);
    }
}
