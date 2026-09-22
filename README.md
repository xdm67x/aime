# Pulse

Pulse is an AI agent harness with a terminal UI (plus a marketing website).
You send prompts to LLM providers; a classifier routes each prompt to a model
tier (`High`/`Base`/`Low`), an agentic tool loop executes the work, and results
persist as "beats". Beats born from a project get their own git worktree, and
a small workflow engine drives multi-step runs.

The project is a Rust workspace:

- `pulse-core/` — the agent harness as a pure library (no UI dependencies):
  providers (OpenRouter, OpenCode, LiteLLM), core tools (`read_file`,
  `write_file`, `edit_file`, `grep`, `bash`), skill discovery, tier routing,
  prompt templates, SQLite persistence, and the workflow engine.
- `pulse/` — the terminal app: a CLI + TUI hybrid on top of `pulse-core`.
- `web/` — the static marketing site (Vite + pnpm, deployed to GitHub Pages).

Development is driven by [mise](https://mise.jdx.dev); the tool versions and
tasks live in `mise.toml`.

## Development

Requirements: [mise](https://mise.jdx.dev/getting-started.html) — nothing else.
`mise install` provides everything, including the `node` runtime that the
`web/` build scripts (`tsc`, `vite`) require.

```sh
mise install        # rust 1.98.1 + node 24 + pnpm 12.4.1
mise run test       # cargo test
mise run build      # cargo build
mise run run        # cargo run -p pulse (the TUI)
mise run lint       # web/ lint + format check
mise run web-build  # build the static site
mise run release-build  # release binary for aarch64-apple-darwin
```

Headless CLI subcommands also work without the TUI:

```sh
cargo run -p pulse -- settings list
cargo run -p pulse -- workflow list
```

## Install (clients, macOS ARM only)

Releases are built by CI on tag push and publish a single asset per version:
`pulse-<tag>-aarch64-apple-darwin.tar.gz` containing the `pulse` binary. mise
installs it from GitHub releases and puts `pulse` on your PATH.

This repository is private, so clients need a GitHub token with read access to
it (fine-grained token with **Contents: read** is enough):

```sh
export MISE_GITHUB_TOKEN=<token with read access to the repo>
mise use -g "github:xdm67x/pulse[matching=aarch64-apple-darwin]@latest"
```

The `matching=aarch64-apple-darwin` option selects only the macOS ARM CLI
tarball; install attempts on any other platform or architecture fail with no
matching asset. To pin a version instead of tracking the latest release,
replace `@latest` with a version (`@0.5.1`, …).

Alternatively, build from source on any macOS ARM machine with the repo
checked out:

```sh
mise run release-build
# binary at target/aarch64-apple-darwin/release/pulse
```

## Releasing

Pushing a tag (`v*`) triggers `.github/workflows/release.yml`, which builds the
`pulse` binary for `aarch64-apple-darwin` and attaches
`pulse-<tag>-aarch64-apple-darwin.tar.gz` to the GitHub release — the exact
asset the client install above consumes, so keep the naming in sync.
