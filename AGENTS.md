# AGENTS.md

Guidance for AI coding agents working in this repository.

Before making any changes or contributions, read
[CONTRIBUTING.md](CONTRIBUTING.md) and follow its contribution guidelines
(Conventional Commits: `type(scope): description`).

## What this repo is

**Pulse** — an AI agent harness. A Tauri desktop app (plus a marketing website)
where users send prompts to LLM providers; a classifier routes each prompt to a
model tier, an agentic tool loop executes work, and results persist as "beats".

## Layout

- `pulse-core/` — the agent harness as a pure Rust library. **No UI
  dependencies.** Consumed today by the Tauri app; designed to also back a
  future CLI. Progress flows through a caller-supplied `harness::OnEvent`
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
- `src-tauri/` — thin Tauri 2 shell. `src/main.rs` (≈160 lines) only wraps
  `pulse-core` functions as `#[tauri::command]`s and registers them in
  `invoke_handler`. Keep business logic out of here — it belongs in
  `pulse-core`.
- `ui/` — Tauri frontend: vanilla TypeScript + Vite, no framework.
  `src/main.ts` (~840 lines) renders beats, transcripts, and settings; calls
  the backend via `window.__TAURI__.core.invoke`. Markdown rendered with
  `marked` + sanitized with `dompurify`.
- `web/` — static GitHub Pages site (project landing page + release
  downloads). Deployed under `/pulse/`, so `vite.config.ts` uses `base: './'`.
- `pulse-node/` — napi-rs addon exposing `pulse-core` to Node.js. The VS Code
  extension runs the same Rust harness as the Tauri app through it (no TS
  port to keep in sync). `src/lib.rs` holds the `#[napi]` bindings: sync
  CRUD for beats/projects/config, async `run_task` streaming events through
  a `ThreadsafeFunction` callback, `start_models_refresh` for the background
  model-list refresh. Build with `pnpm --dir pulse-node run build`
  (requires Rust + the platform target; outputs `pulse-node.<triple>.node`).
- `vscode/` — the harness as a VS Code extension (TypeScript + React).
  Shares the same `~/.pulse/pulse.db` as the Tauri app.
  - `src/native.ts` — typed facade over the `pulse-node` addon; the single
    import surface for the extension host. napi objects are camelCase.
  - `src/extension.ts` + `src/panel.ts` — extension host: webview panel with
    CSP + nonce, message protocol in `src/protocol.ts`.
  - `src/webview/` — React 19 UI (sidebar beats, transcript with streaming
    bubbles and tool/diff cards, settings). Built by Vite into
    `webview-dist/`, loaded via `webview.asWebviewUri`.
  - React pattern quality is enforced by **react-doctor** (`pnpm --dir vscode
    doctor`), alongside oxlint/oxfmt.

## Toolchains & commands

- Rust workspace (`Cargo.toml`): members are `pulse-core`, `pulse-node` and `src-tauri`.
- JS: pnpm workspaces (`pnpm-workspace.yaml`), pnpm 12.4.1.
- Frontend lint/format: **oxlint** and **oxfmt** (not eslint/prettier).

```sh
cargo test                      # all workspace tests (19 currently pass)
cargo build                     # workspace
pnpm --dir ui dev               # dev server (Vite, port 5173) for the Tauri app
pnpm --dir ui lint && pnpm --dir ui format:check
pnpm --dir web build            # static site (base: './')
pnpm tauri dev                  # full desktop app (from src-tauri)
pnpm install                     # workspace install (incl. vscode extension)
pnpm --dir pulse-node run build  # build the native addon (needs Rust)
pnpm --dir vscode build         # compile extension + build webview (Vite)
pnpm --dir vscode test          # vitest tests (run the native addon in Node)
pnpm --dir vscode doctor        # react-doctor scan of the React webview
pnpm --dir vscode package       # build .vsix (via vsce)
```

## Conventions & gotchas

- **Keep `pulse-core` UI-agnostic.** New agent features (providers, tools,
  skills, routing, persistence) go in `pulse-core`; `src-tauri` only adds
  command wrappers; `ui` only adds views/invocations.
- **Every new Tauri command** must be added to
  `tauri::generate_handler![...]` in `src-tauri/src/main.rs` or it silently
  fails at runtime.
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
- **Tests**: unit tests live inline in `pulse-core` modules (`cargo test`
  runs 18 of them) plus an integration test in `pulse-core/tests/`. Add tests
  alongside the code you change; run `cargo test` before committing.
- **Frontend**: no framework, no build-time deps beyond what's in
  `package.json`. Keep `ui` (app) and `web` (site) separate — don't share code
  between them.
- **Tauri capabilities**: commands used from JS must be allowed in
  `src-tauri/capabilities/default.json`.
