# AGENTS.md

Guidance for AI coding agents working in this repository.

Before making any changes or contributions, read
[CONTRIBUTING.md](CONTRIBUTING.md) and follow its contribution guidelines
(Conventional Commits: `type(scope): description`).

## What this repo is

**Pulse** — an AI agent harness with a terminal UI (plus a marketing website)
where users send prompts to LLM providers; a classifier routes each prompt to a
model tier, an agentic tool loop executes work, and results persist as "beats".

## Layout

- `pulse-core/` — the agent harness as a pure Rust library. **No UI
  dependencies.** Consumed today by the TUI app (`pulse-tui`); designed to also
  back other runtimes. Progress flows through a caller-supplied `harness::OnEvent`
  callback, so any runtime can drive it.
  - `harness.rs` — agentic loop, tier routing (`Tier::High/Base/Low`), task
    events (`TaskEvent`, `TaggedEvent`), cancellation, `run_task` entry point.
  - `providers/` — `Provider` trait with `openrouter`, `opencode`, `litellm`
    implementations; `chat_completion` / `chat_completion_stream` /
    `list_models`.
  - `tools.rs` — core tools: `read_file`, `write_file`, `edit_file`, `grep`,
    `bash`, plus one `skill_<name>` tool per discovered skill.
  - `skills.rs` — discovers skills from `~/.agents/skills/*/SKILL.md` (YAML
    frontmatter gives name/description; full file loads on demand).
  - `prompts.rs` + `prompts/` — prompt templates embedded at compile time via
    `include_str!`; `{{key}}` placeholders filled at runtime.
  - `config.rs` — API keys/base URLs and the four model slots
    (`classifier`, `high`, `base`, `low`) stored in the DB.
  - `db.rs` — SQLite at `~/.pulse/pulse.db` (`config`, `projects`, `beats`
    tables; rusqlite, bundled).
  - `beats.rs` / `projects.rs` — beat + project persistence. Beats born from a
    project get their own git worktree under `~/.pulse/worktrees/<beat>-<name>`.
  - `workflows.rs` — the workflow engine.
- `pulse-tui/` — the terminal app: ratatui + crossterm on top of `pulse-core`.
  - `src/main.rs` — terminal entry point, event loop, key dispatch.
  - `src/app.rs` — application state and modes; `src/task.rs` runs harness
    tasks; `src/event.rs` bridges input events.
  - `src/ui/` — views: chat, projects, sessions, settings, workflows.
- `web/` — static GitHub Pages site (project landing page + release
  downloads). Deployed under `/pulse/`, so `vite.config.ts` uses `base: './'`.

## Toolchains & commands

- Rust workspace (`Cargo.toml`): members are `pulse-core` and `pulse-tui`.
- JS: pnpm 12.4.1, only for `web/` (which has its own workspace + lockfile).
- Frontend lint/format: **oxlint** and **oxfmt** (not eslint/prettier).

```sh
cargo test                      # all workspace tests
cargo build                     # workspace
cargo run -p pulse-tui          # run the terminal app
pnpm --dir web lint && pnpm --dir web format:check
pnpm --dir web build            # static site (base: './')
```

Releases: `.github/workflows/release.yml` builds the `pulse-tui` binary for
`aarch64-apple-darwin` on tag push, packages it as
`pulse-tui-<tag>-aarch64-apple-darwin.tar.gz`, and attaches it to the GitHub
release. The website's download button links to the latest such asset — keep
the naming in sync when changing it.

## Conventions & gotchas

- **Keep `pulse-core` UI-agnostic.** New agent features (providers, tools,
  skills, routing, persistence, workflows) go in `pulse-core`; `pulse-tui`
  only adds terminal views and input handling.
- **Errors** are `Result<_, String>` throughout the core — follow that
  pattern; don't introduce a custom error type piecemeal.
- **Async**: core uses tokio (`rt`, `time`, `process`, `macros` features).
  Provider calls are async via `async_trait`.
- **Prompts are compile-time**: edit files in `pulse-core/src/prompts/`, not
  strings in code. Keep `{{placeholder}}` names in sync with `prompts::fill`.
- **DB migrations**: schema is created with `CREATE TABLE IF NOT EXISTS` in
  `db.rs::open()`. Add columns there; there is no migration framework.
- **API keys** live in the local DB (`~/.pulse/pulse.db`), never in code or
  the repo. Valid providers are hardcoded in `config.rs`.
- **Tests**: unit tests live inline in `pulse-core` modules plus an
  integration test in `pulse-core/tests/`. Add tests alongside the code you
  change; run `cargo test` before committing.
- **Website**: keep `web` self-contained (own package.json/lockfile); don't
  share code between it and the Rust workspace.
