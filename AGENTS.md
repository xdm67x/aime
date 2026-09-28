# AGENTS.md

Guidance for AI coding agents working in this repository.

Before making any changes or contributions, read
[CONTRIBUTING.md](CONTRIBUTING.md) and follow its contribution guidelines
(Conventional Commits: `type(scope): description`).

## What this repo is

**Pulse** — a headless workflow runner (plus a marketing website): users
provide a YAML workflow file, `pulse <workflow>` executes its steps through
an agentic tool loop until each step's goal is reached, everything the run
produces is written to a markdown report in the current directory, and the
terminal shows which step is running. The provider is configured with
`pulse provider use` — an OpenAI-compatible base URL + API key, or a known
provider name (`litellm`, `mistral`, `opencode`, `openrouter`) with just an
API key; `pulse provider` shows the current one.

## Layout

- `pulse-core/` — the agent harness as a pure Rust library. **No UI
  dependencies.** Consumed today by the `pulse` binary; designed to also
  back other runtimes. Progress flows through a caller-supplied `harness::OnEvent`
  callback, so any runtime can drive it.
  - `harness.rs` — agentic loop, task events (`TaskEvent`, `TaggedEvent`),
    cancellation, `run_task` entry point (resolves the workflow — plain
    prompts run `base`, `/workflow {name}` picks one — and runs its steps).
  - `providers/` — `Provider` trait with `custom` (the OpenAI-compatible
    endpoint configured by `pulse provider use <url> <key>`), plus
    `openrouter`, `opencode`, `litellm`, `mistral` implementations (any of
    which `pulse provider use <name> <key>` configures with a key alone);
    bare model ids route to the configured provider
    (`providers::configured_provider`); `chat_completion` /
    `chat_completion_stream` / `list_models` / `list_models_of` (per
    provider).
  - `tools.rs` — core tools: `read_file`, `write_file`, `edit_file`, `grep`,
    `bash`, plus one `skill_<name>` tool per discovered skill.
  - `skills.rs` — discovers skills from `~/.agents/skills/*/SKILL.md` (YAML
    frontmatter gives name/description; full file loads on demand).
  - `prompts.rs` + `prompts/` — prompt templates embedded at compile time via
    `include_str!`; `{{key}}` placeholders filled at runtime.
  - `config.rs` — API keys/base URLs stored in the DB (`provider_url` /
    `provider_api_key` / `provider_name` for the configured provider).
    There are no global model slots: models are named per workflow in the
    workflow files.
  - `db.rs` — SQLite at `~/.pulse/pulse.db` (`config`, `projects`, `beats`
    tables; rusqlite, bundled).
  - `beats.rs` / `projects.rs` — beat + project persistence. A workflow run
    creates a beat so context accumulates across its steps, and (by default)
    a git worktree under `~/.pulse/worktrees/<run id>-<dir>` via
    `projects::ensure_worktree`, recorded on the beat so every tool of the
    run executes there instead of the user's checkout.
  - `workflows.rs` — the workflow engine: YAML files (`./.pulse/workflows`
    first, then the current directory, then `~/.pulse/workflows/`),
    per-step/workflow `model:` (required
    somewhere), per-step optional `goal:` (the step re-runs with reviewer
    feedback until the model confirms the goal is reached, max
    `MAX_GOAL_ATTEMPTS` runs). `{{prompt}}` placeholders are filled with a
    runtime-supplied message (`harness::run_task`'s plain prompts use this;
    the workflow CLI passes none). `{{steps.<name>}}` placeholders in a
    step's prompt are filled with the named earlier step's final result,
    linking steps; whitespace inside the braces is tolerated
    (`{{ steps.<name> }}` is the same reference), and `Workflow::validate`
    rejects references that don't resolve before the run starts. `run_hooked` exposes progress
    hooks; `run` is the no-op-hooks wrapper.
- `pulse/` — the CLI on top of `pulse-core`. No TUI.
  - `src/main.rs` — entry point: clap dispatch + exit codes.
  - `src/cli.rs` — subcommands: `pulse create`, `pulse edit`, `pulse workflow
    list`, `pulse provider [use
    <url|litellm|mistral|opencode|openrouter> <key>]`
    (bare `pulse provider` shows the current one), `pulse models`,
    `pulse version`,
    `pulse update` (installs the latest release automatically when newer), and
    `pulse <workflow> [--no-worktree]` (an external subcommand — any unknown
    subcommand is treated as a workflow name/path to run; `split_run_args`
    pulls out the flags).
  - `src/run.rs` — the headless runner: executes steps via
    `workflows::run_hooked`, prints step progress (`[i/n] step` + tool
    lines) to the terminal, and writes everything (prompts, goals, tool
    calls, streamed output, results) to `<workflow>-<timestamp>.md` in the
    current directory, flushed as it happens. Unless `--no-worktree` is
    passed, the run first creates a worktree (`<timestamp>-<dir name>`
    under `~/.pulse/worktrees`) — a non-repo directory runs in place, a
    repo whose worktree cannot be created aborts. First Ctrl-C cancels the
    run (exit 130), second force-quits.
- `web/` — static GitHub Pages site (project landing page + release
  downloads). Deployed under `/pulse/`, so `vite.config.ts` uses `base: './'`.

## Toolchains & commands

- Rust workspace (`Cargo.toml`): members are `pulse-core` and `pulse`.
- JS: pnpm 12.4.1, only for `web/` (which has its own workspace + lockfile).
- Frontend lint/format: **oxlint** and **oxfmt** (not eslint/prettier).

```sh
cargo test                      # all workspace tests
cargo build                     # workspace
cargo run -p pulse -- create my-task   # create ./.pulse/workflows/my-task.yml
cargo run -p pulse -- create my-task --global  # create ~/.pulse/workflows/my-task.yml
cargo run -p pulse -- my-task     # run a workflow by name (or path)
cargo run -p pulse -- my-task --no-worktree   # run in the current directory, no git worktree
pnpm --dir web lint && pnpm --dir web format:check
pnpm --dir web build            # static site (base: './')
```

Releases: `.github/workflows/release.yml` builds the `pulse` binary for
`aarch64-apple-darwin` on tag push, packages it as
`pulse-<tag>-aarch64-apple-darwin.tar.gz`, and attaches it to the GitHub
release. The website's download button links to the latest such asset — keep
the naming in sync when changing it.

## Conventions & gotchas

- **Keep `pulse-core` UI-agnostic.** New agent features (providers, tools,
  skills, routing, persistence, workflows) go in `pulse-core`; `pulse`
  only adds the CLI, terminal output, and input handling.
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
