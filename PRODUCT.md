# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

The product itself is a headless CLI (no TUI — the chat-style terminal app was
removed); `web` is the platform of its visual surfaces — today the static
marketing site in `web/`, deployed to GitHub Pages.

## Users

Developers who live in the terminal and want LLM-driven work to run as
defined, repeatable workflows instead of ad-hoc chat. The primary audience is
a public developer audience (owner-confirmed): the current private,
token-gated install is an interim state, not the intended audience. Their
job: turn recurring dev work (refactors, reviews, release prep, docs passes)
into named YAML workflows that run autonomously and leave a persisted,
inspectable, costed record.

## Product Purpose

Pulse is a headless workflow runner. The user writes a YAML workflow file
(created with `pulse workflow new <title>` in the current directory) that
names the model, the steps, and each step's optional `goal:`; `pulse
<workflow>` executes the steps through an agentic tool loop
(`read_file`, `write_file`, `edit_file`, `grep`, `bash`, plus discovered
skills) until each goal is confirmed reached; the terminal shows which step
is running while the full run — prompts, tool calls, streamed output,
results — is written to `<workflow>-<timestamp>.md` in the launch directory
as it happens. Success means daily work flows through workflows
autonomously and every run leaves a durable, inspectable report.

## Positioning

**Workflow-only agent runner — not a chat assistant.** Owner-confirmed
direction: Pulse moved away from chat entirely; the chat-style TUI was
replaced by the headless workflow runner. Where neighboring agent CLIs
(Claude Code, aider, opencode) are chat-first with the agent loop as the
interface, Pulse inverts it: the workflow file is the interface. It names
the model (per workflow or per step), the steps, and the `goal:` each step
runs until. Every run writes a markdown report in the launch directory —
the durable record is the position.

## Operating Context

- Terminal-first and headless: one binary, zero config files; the CLI is the
  whole product (`pulse workflow new|list|edit`, `pulse provider use <url>
  `pulse models`, `pulse version`, `pulse update`, `pulse <workflow>`).
- Provider: one OpenAI-compatible endpoint (base URL + API key) configured
  with `pulse provider use` — OpenAI, LiteLLM, Ollama's `/v1`, vLLM, …;
  `pulse models` lists what it offers. Workflows name the model per workflow
  or per step; there are no global model slots.
- Workflows: YAML files — the current directory first, then
  `~/.pulse/workflows/`; per-step optional `goal:` re-runs the step with
  reviewer feedback until confirmed reached (max 3 runs).
- Runs: by default the run executes in a fresh git worktree
  (`~/.pulse/worktrees/<run id>-<dir name>`, own branch) so the user's
  checkout is untouched; `--no-worktree` runs in the launch directory. The
  terminal shows step progress (`[i/n] step`, tool lines, worktree + report
  paths); the full run is written to `<workflow>-<timestamp>.md` in the
  launch directory, flushed as it happens. First Ctrl-C cancels (exit 130).
- Persistence: SQLite at `~/.pulse/pulse.db` (runs still create beats so
  context accumulates across steps; internal, not a user surface).
- Distribution: GitHub releases via mise (`mise use -g
  "github:xdm67x/pulse@latest"`); self-update via `pulse update`
  (token via `GITHUB_TOKEN`/`MISE_GITHUB_TOKEN`).
- Development: Rust workspace (`pulse-core` pure library + `pulse` CLI),
  mise-driven (`mise run test|build|run|lint|web-build`), pnpm for `web/`
  only.

## Capabilities and Constraints

Confirmed functionality:

- Provider: any OpenAI-compatible endpoint via `pulse provider use <url>
  <key>` (`chat_completion`, streaming, model listing); OpenRouter,
  OpenCode, LiteLLM, Mistral implementations also live in core.
- Core tools: `read_file`, `write_file`, `edit_file`, `grep`, `bash`, plus one
  `skill_<name>` tool per skill discovered in
  `~/.agents/skills/*/SKILL.md`.
- Workflow engine: per-step and per-workflow `model:` (required somewhere);
  per-step optional `goal:` (the step re-runs with reviewer feedback until
  the model confirms the goal is reached, max 3 runs). Workflows are
  self-contained — the run command takes only the workflow name.
- Workflow files: `pulse workflow new <title>` writes a blank template in the
  current directory; `pulse <workflow>` resolves by name (cwd, then
  `~/.pulse/workflows/`) or file path.
- Every run writes `<workflow>-<timestamp>.md` (prompts, goals, tool calls,
  streamed output, results, cost) in the launch directory as it happens.
- Worktree isolation by default: `~/.pulse/worktrees/<run id>-<dir name>`
  branched off the current HEAD (a non-repo directory runs in place;
  `--no-worktree` opts out).
- First Ctrl-C cancels the run (exit 130); a second force-quits.

Technical constraints (durable, from repo conventions):

- `pulse-core` stays UI-agnostic (no UI dependencies); `pulse` only adds CLI,
  terminal views, and input handling.
- Errors are `Result<_, String>` throughout core; no custom error type.
- Prompts are compile-time templates in `pulse-core/src/prompts/` with
  `{{placeholder}}` names kept in sync with `prompts::fill`.
- Schema via `CREATE TABLE IF NOT EXISTS` in `db.rs::open()`; no migration
  framework.
- API keys live in the local DB, never in code or repo.
- Releases: single `pulse-<tag>-aarch64-apple-darwin.tar.gz` asset per
  version, built on tag push; the site's download button links to it (keep
  naming in sync).
- `web/` stays self-contained (own package.json/lockfile; oxlint/oxfmt, not
  eslint/prettier; Vite `base: './'` under `/pulse/`).

Explicitly open decisions (owner did not answer; do not invent):

- Private repo + GitHub-token-gated install: interim step toward the public
  audience, or a durable constraint? Unconfirmed.
- macOS / Apple Silicon-only release asset: deliberate scope or interim?
  Unconfirmed.
- Licensing/open-source commitment: the site footer says "an open agent
  harness", but no license decision was confirmed.
- What the workflow-only direction means for the marketing site's beat
  vocabulary now that runs surface as reports (the ECG/heartbeat motif
  remains the visual brand; user-facing copy now centers on workflows,
  goals and reports).

## Brand Commitments

Existing brand vocabulary and assets (evidence from site and product, not
expanded beyond what exists):

- Name: **Pulse**.
- The beat/heartbeat metaphor is the core visual vocabulary: runs still
  create beats internally (SQLite), and the ECG/pulse-line motif runs through
  the marketing site (animated ECG strokes, status dots and costs). User-
  facing copy has shifted to workflows, goals and markdown reports —
  "beat" is no longer a user-visible concept in the CLI.
- Site voice: terse, imperative, terminal-flavored ("Every step runs to
  its goal.", "One command. Five moves.", "Pick a workflow. Watch it
  run.", "Built for the daily routine.").
- Footer claim: "an open agent harness".
- No binding palette, typography, or other visual constraints were stated by
  the owner.

## Evidence on Hand

- Working product: the Rust workspace itself (harness, providers, tools,
  workflows + goals, persistence, CLI).
- Marketing site (`web/`, deployed on GitHub Pages): hero with animated
  terminal demo and install command; "One command. Five moves." routine
  section with a run wall; interactive workflow playground (review / ship /
  docs tabs with simulated runs); four feature cards; install CTA; slide
  deck chrome (dots, counter, ticker).
- README with accurate install/update/release flows.
- No real testimonials, customers, benchmarks, or case studies exist —
  future work must not fabricate them. The site's demo terminal and
  playground runs are simulations, not recorded sessions.

## Product Principles

1. **Workflow-only, not chat.** The YAML workflow file is the interface;
   the chat TUI is gone.
2. **Autonomy with a record.** Runs execute tools autonomously until each
   step's goal is confirmed reached, and everything lands in an
   inspectable markdown report in the launch directory.
3. **Terminal-native.** One binary, zero config files, lives where the user
   already works; the marketing site speaks the terminal's visual language.
4. **Model freedom, named in the file.** Workflows choose the model per run
   and per step; the provider endpoint is the user's (any OpenAI-compatible
   URL + key).
5. **Core is UI-agnostic.** `pulse-core` remains a pure library so other
   runtimes can drive the same harness.
