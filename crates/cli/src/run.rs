//! Headless workflow runner: executes each step through the agentic loop
//! until its goal is reached, printing step progress to the terminal while
//! the full run (prompts, tool calls, streamed output, results) is written
//! to a markdown report in the current directory.

use pulse_core::harness::{TaggedEvent, TaskEvent};
use pulse_core::workflows::{RunHooks, Workflow, WorkflowStep, WorkflowStepResult};
use pulse_core::{beats, config, harness, projects, providers, workflows};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// Exit code returned when a run is interrupted with Ctrl-C.
pub const CANCELLED: i32 = 130;

/// Run `pulse <workflow>`: resolve the workflow, execute every step until
/// its goal is reached, write the markdown report next to the user and print
/// live step progress. Unless `use_worktree` is false the run works in a
/// fresh git worktree under `~/.pulse/worktrees` so the user's checkout
/// stays untouched. Returns the process exit code.
pub async fn run_workflow(name_or_path: &str, use_worktree: bool) -> i32 {
    match run_inner(name_or_path, use_worktree).await {
        Ok(code) => code,
        Err(e) => {
            pulse_core::log::error(format!("workflow run failed: {e}"));
            eprintln!("Error: {e}");
            1
        }
    }
}

async fn run_inner(name_or_path: &str, use_worktree: bool) -> Result<i32, String> {
    let (wf, wf_path) = workflows::find(name_or_path)?;
    wf.validate()?;
    let provider = provider_label()?;

    let cwd = std::env::current_dir()
        .map_err(|e| format!("Failed to read the current directory: {e}"))?;
    let ts = timestamp();
    let beat = beats::create_beat(
        &format!("{}-{ts}", slug(&wf.name)),
        &format!("workflow run: {}", wf_path.display()),
        None,
    )?;

    // By default the run executes in a fresh git worktree branched off the
    // current HEAD: `<run id>-<directory>` under ~/.pulse/worktrees, recorded
    // on the beat so every tool runs there. Not a git repo → run in place;
    // a repo whose worktree cannot be created is a hard error (the run would
    // otherwise silently edit the user's checkout).
    let mut worktree = None;
    let mut worktree_note = None;
    if use_worktree {
        let name = format!("{ts}-{}", dir_name(&cwd));
        match projects::ensure_worktree(beat.id, &name, &cwd.to_string_lossy()) {
            Ok(Some((branch, path))) => {
                worktree_note = Some(format!("  worktree: {path} (branch {branch})"));
                worktree = Some(format!("{path} (branch: {branch})"));
            }
            Ok(None) => {
                worktree_note =
                    Some("  not a git repository — running in this directory".to_string());
            }
            Err(e) => {
                return Err(format!(
                    "could not create a worktree for this run: {e} — \
                     retry, or pass --no-worktree to work in this directory directly"
                ));
            }
        }
    }

    let md_path = PathBuf::from(format!("{}-{ts}.md", slug(&wf.name)));
    let file = std::fs::File::create(&md_path)
        .map_err(|e| format!("Failed to create {}: {e}", md_path.display()))?;
    let mut rep = Reporter::new(file, &wf, &wf_path, &provider, &ts, worktree.as_deref());
    rep.header();

    println!(
        "▶ workflow '{}' — {} steps — {}",
        wf.name,
        wf.steps.len(),
        wf_path.display()
    );
    println!("  provider: {provider}");
    if let Some(note) = &worktree_note {
        println!("{note}");
    }
    println!("  report:   {}", md_path.display());

    // Ctrl-C: the first press asks the harness to stop after the current
    // model round; a second press force-exits.
    let beat_id = beat.id;
    let interrupted = Arc::new(AtomicBool::new(false));
    {
        let interrupted = interrupted.clone();
        tokio::spawn(async move {
            let mut first = true;
            while tokio::signal::ctrl_c().await.is_ok() {
                if !first {
                    std::process::exit(CANCELLED);
                }
                first = false;
                interrupted.store(true, Ordering::SeqCst);
                harness::cancel_current(beat_id);
                eprintln!("\nCancelling… press Ctrl-C again to force quit");
            }
        });
    }

    let total = wf.steps.len();
    let reporter = Arc::new(Mutex::new(rep));
    let mut on_step_start = |idx: usize, step: &WorkflowStep, prompt: &str| {
        reporter.lock().unwrap().step_start(idx, step, prompt);
    };
    let mut on_step_done = |r: &WorkflowStepResult| {
        reporter.lock().unwrap().step_done(r);
    };
    let mut on_goal_retry = |_step: &WorkflowStep, attempt: usize, reason: &str| {
        reporter.lock().unwrap().goal_retry(attempt, reason);
    };
    let mut on_event = |ev: TaggedEvent| {
        reporter.lock().unwrap().event(&ev.ev);
    };

    let result = workflows::run_hooked(
        beat.id,
        &wf,
        None,
        &[],
        RunHooks {
            on_step_start: &mut on_step_start,
            on_step_done: &mut on_step_done,
            on_goal_retry: &mut on_goal_retry,
            on_event: &mut on_event,
        },
    )
    .await;

    let mut rep = reporter.lock().unwrap();
    match result {
        Ok(r) => {
            rep.finished(r.total_cost_usd, true);
            println!(
                "\n✓ workflow complete — {} steps, ${:.4} total",
                total, r.total_cost_usd
            );
            println!("  report: {}", md_path.display());
            Ok(0)
        }
        Err(e) if e == harness::STOPPED || interrupted.load(Ordering::SeqCst) => {
            rep.finished(0.0, false);
            println!(
                "\n■ cancelled — partial output kept in {}",
                md_path.display()
            );
            Ok(CANCELLED)
        }
        Err(e) => {
            rep.failed(&e);
            Err(e)
        }
    }
}

/// Everything written to the markdown report + terminal during one run.
struct Reporter {
    file: std::fs::File,
    workflow: String,
    path: PathBuf,
    provider: String,
    started: String,
    /// The worktree the run executes in, if any.
    worktree: Option<String>,
    step_num: usize,
    total: usize,
}

impl Reporter {
    fn new(
        file: std::fs::File,
        wf: &Workflow,
        path: &std::path::Path,
        provider: &str,
        started: &str,
        worktree: Option<&str>,
    ) -> Self {
        Self {
            file,
            workflow: wf.name.clone(),
            path: path.to_path_buf(),
            provider: provider.to_string(),
            started: started.to_string(),
            worktree: worktree.map(str::to_string),
            step_num: 0,
            total: wf.steps.len(),
        }
    }

    /// Append to the report file, flushing so a crash keeps everything
    /// written so far.
    fn md(&mut self, s: &str) {
        let _ = self.file.write_all(s.as_bytes());
        let _ = self.file.flush();
    }

    /// Report header section.
    fn header(&mut self) {
        let worktree = self
            .worktree
            .clone()
            .unwrap_or_else(|| "none — running in the launch directory".into());
        self.md(&format!(
            "# Workflow: {}\n\n\
             - File: {}\n\
             - Started: {} UTC\n\
             - Provider: {}\n\
             - Worktree: {}\n",
            self.workflow,
            self.path.display(),
            self.started,
            self.provider,
            worktree,
        ));
    }

    fn step_start(&mut self, idx: usize, step: &WorkflowStep, prompt: &str) {
        self.step_num = idx + 1;
        println!("\n[{}/{}] {}", self.step_num, self.total, step.name);
        if let Some(goal) = step
            .goal
            .as_deref()
            .map(str::trim)
            .filter(|g| !g.is_empty())
        {
            println!("  goal: {}", preview(goal, 72));
        }
        self.md(&format!(
            "\n\n## Step {}/{}: {}\n\n**Prompt:**\n\n~~~\n{prompt}\n~~~\n",
            self.step_num, self.total, step.name
        ));
        if let Some(goal) = step.goal.as_deref() {
            self.md(&format!("\n**Goal:**\n\n{goal}\n"));
        }
        self.md("\n### Live output\n");
    }

    fn event(&mut self, ev: &TaskEvent) {
        match ev {
            TaskEvent::Start { .. } => {}
            TaskEvent::Delta { text } => self.md(text),
            TaskEvent::Tool {
                tool,
                arguments,
                result,
                error,
            } => {
                println!(
                    "  ↳ {tool} {} [{}]",
                    preview(arguments, 48),
                    if *error { "error" } else { "ok" }
                );
                self.md(&format!(
                    "\n**Tool `{tool}`**\n\n~~~\n$ {arguments}\n~~~\n\n~~~\n{result}\n~~~\n"
                ));
            }
            TaskEvent::Step { text } => {
                self.md(&format!("\n{text}\n"));
            }
        }
    }

    fn goal_retry(&mut self, attempt: usize, reason: &str) {
        println!("  ↻ goal not reached — retrying (attempt {attempt}): {reason}");
        self.md(&format!(
            "\n\n**Goal not reached — running again (attempt {attempt})**\n\nReviewer: {reason}\n"
        ));
    }

    fn step_done(&mut self, r: &WorkflowStepResult) {
        match r.goal_met {
            None => println!("  ✓ step done — ${:.4}", r.cost_usd),
            Some(true) => println!(
                "  ✓ step done — goal reached in {} attempt(s), ${:.4}",
                r.attempts.unwrap_or(1),
                r.cost_usd
            ),
            Some(false) => println!(
                "  ⚠ step done — goal NOT reached after {} attempt(s), ${:.4}",
                r.attempts.unwrap_or(1),
                r.cost_usd
            ),
        }
        self.md(&format!(
            "\n\n### Result\n\n{}\n\n*Model: `{}` — cost: ${:.4}{}\n",
            r.answer,
            r.model,
            r.cost_usd,
            match r.goal_met {
                None => String::new(),
                Some(true) => format!(" — goal reached in {} attempt(s)", r.attempts.unwrap_or(1)),
                Some(false) => {
                    format!(
                        " — goal NOT reached after {} attempt(s)",
                        r.attempts.unwrap_or(1)
                    )
                }
            }
        ));
    }

    fn finished(&mut self, total_cost: f64, complete: bool) {
        self.md(&format!(
            "\n\n---\n\n## Summary\n\n- Steps completed: {}/{}\n- Total cost: ${total_cost:.4}\n- Finished: {} UTC\n- Status: {}\n",
            self.step_num,
            self.total,
            timestamp(),
            if complete { "complete" } else { "stopped" },
        ));
    }

    fn failed(&mut self, error: &str) {
        self.md(&format!("\n\n## Run failed\n\n~~~\n{error}\n~~~\n"));
    }
}

/// First line of `s`, cut to `width` chars with an ellipsis.
fn preview(s: &str, width: usize) -> String {
    let first = s.lines().next().unwrap_or("").trim();
    let mut out: String = first.chars().take(width).collect();
    if first.chars().count() > width {
        out.push('…');
    }
    out
}

/// The final path segment of `dir` — names the run's worktree after the
/// directory the workflow was launched from.
fn dir_name(dir: &std::path::Path) -> String {
    dir.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("repo")
        .to_string()
}

/// Turn a workflow title into a safe file stem: lowercase, runs of
/// non-alphanumeric characters collapsed to `-`, edges trimmed. A trailing
/// `.yml`/`.yaml` (the user passing a filename as the title) is dropped.
pub fn slug(title: &str) -> String {
    let mut base = title.trim().to_lowercase();
    for ext in [".yml", ".yaml"] {
        if let Some(s) = base.strip_suffix(ext) {
            base = s.to_string();
            break;
        }
    }
    let mut out = String::with_capacity(base.len());
    let mut dash = false;
    for c in base.chars() {
        if c.is_ascii_alphanumeric() || c == '-' {
            out.push(c);
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// `YYYYMMDD-HHMMSS` in UTC from unix seconds — used for beat names, report
/// file names and timestamps inside the report (no chrono dependency).
fn timestamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    timestamp_from(secs)
}

fn timestamp_from(secs: u64) -> String {
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days as i64);
    format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}",
        y,
        m,
        d,
        rem / 3_600,
        (rem % 3_600) / 60,
        rem % 60
    )
}

/// Civil date from days since the unix epoch (Howard Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Shown by every provider-needing CLI path when none is configured.
pub const NO_PROVIDER: &str =
    "No provider configured — set one first: pulse provider use <url|litellm|mistral|opencode|openrouter> <api_key>";

/// The provider configured with `pulse provider use`, or the error every
/// provider-needing command shows.
pub fn require_provider() -> Result<&'static dyn providers::Provider, String> {
    providers::configured_provider().ok_or(NO_PROVIDER.to_string())
}

/// Human label for the configured provider: `Custom (<url>)` for the
/// URL-configured one, else its display name (e.g. `OpenRouter`).
pub fn provider_label() -> Result<String, String> {
    let p = require_provider()?;
    if p.name() == "Custom" {
        if let Some(url) = config::provider_url()? {
            return Ok(format!("Custom ({url})"));
        }
    }
    Ok(p.name().to_string())
}

/// Bare model id for display: strip the provider prefix the registry adds.
pub fn bare_id(id: &str) -> &str {
    id.split_once(" - ").map(|(_, rest)| rest).unwrap_or(id)
}

/// Price per million tokens: `in`/`out` per-token strings → `$x / $y`.
pub fn price_per_m(pricing: &providers::Pricing) -> String {
    let fmt = |per_token: &str| -> String {
        per_token
            .parse::<f64>()
            .ok()
            .filter(|v| *v > 0.0)
            .map(|v| format!("${:.2}", v * 1_000_000.0))
            .unwrap_or_else(|| "-".into())
    };
    format!("{} / {}", fmt(&pricing.prompt), fmt(&pricing.completion))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slug() {
        assert_eq!(slug("Code Review"), "code-review");
        assert_eq!(slug("  My  Fancy: workflow!! "), "my-fancy-workflow");
        assert_eq!(slug("already-slug.yml"), "already-slug");
        assert_eq!(slug("..."), "");
        assert_eq!(slug("a_b"), "a-b");
    }

    #[test]
    fn test_timestamp() {
        assert_eq!(timestamp_from(0), "19700101-000000");
        // leap day
        assert_eq!(timestamp_from(1_709_164_800), "20240229-000000");
        assert_eq!(timestamp_from(1_790_253_296), "20260924-123456");
    }

    #[test]
    fn test_preview() {
        assert_eq!(preview("hello world", 20), "hello world");
        assert_eq!(preview("hello world", 5), "hello…");
        assert_eq!(preview("line1\nline2", 20), "line1");
    }

    #[test]
    fn test_bare_id() {
        assert_eq!(bare_id("Custom - gpt-4o"), "gpt-4o");
        assert_eq!(bare_id("gpt-4o"), "gpt-4o");
    }

    #[test]
    fn test_price_per_m() {
        let p = providers::Pricing {
            prompt: "0.000003".into(),
            completion: "0.000015".into(),
        };
        assert_eq!(price_per_m(&p), "$3.00 / $15.00");
        let empty = providers::Pricing::default();
        assert_eq!(price_per_m(&empty), "- / -");
    }
}
