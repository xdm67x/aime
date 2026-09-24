# Pulse

Pulse is a headless workflow runner (plus a marketing website). You write a
**workflow** — a YAML file that names the model, the steps, and each step's
optional `goal:` — and `pulse <workflow>` executes the steps through an
agentic tool loop (`read_file`, `write_file`, `edit_file`, `grep`, `bash`,
plus discovered skills) until each goal is reached. The terminal shows which
step is running; everything the run produces — prompts, tool calls, streamed
output, results — is written to a markdown report (`<workflow>-<timestamp>.md`)
in the directory where you launched it.

The project is a Rust workspace:

- `pulse-core/` — the agent harness as a pure library (no UI dependencies):
  providers (any OpenAI-compatible endpoint, plus OpenRouter, OpenCode,
  LiteLLM, Mistral), core tools, skill discovery, workflow + goal engine,
  prompt templates, SQLite persistence.
- `pulse/` — the CLI on top of `pulse-core`.
- `web/` — the static marketing site (Vite + pnpm, deployed to GitHub Pages).

Development is driven by [mise](https://mise.jdx.dev); the tool versions and
tasks live in `mise.toml`.

## Using Pulse

```sh
pulse provider use <url> <api_key>   # any OpenAI-compatible endpoint
pulse models                        # what the provider offers
pulse workflow new <title>          # blank workflow template in the current directory
pulse <workflow>                   # run it (by name or file path)
pulse <workflow> --no-worktree     # run it directly in the current directory
```

The provider URL points at an OpenAI-compatible API root (e.g.
`https://api.openai.com/v1`, a LiteLLM proxy, Ollama's `/v1`); `/chat/completions`
and `/models` are appended. Pass `""` as the API key for endpoints without
auth. The URL and key live in the local database (`~/.pulse/pulse.db`),
never in the repo.

A workflow file (`./<name>.yml`, or `~/.pulse/workflows/` for shared ones)
looks like this:

```yaml
name: ship
description: Tests green, tag it
model: gpt-4o            # required somewhere — per workflow or per step
steps:
  - name: test
    prompt: |
      Run the full test suite.
  - name: release
    goal: |              # optional: the step re-runs with reviewer feedback
      The version is bumped and tagged.   # until the model confirms the goal
    prompt: |            # is reached (max 3 runs)
      Bump the version, tag and build.
```

A run works in a **fresh git worktree** by default —
`~/.pulse/worktrees/<run id>-<directory name>` on its own branch, so your
checkout is never touched mid-run — and lands its changes there for you to
merge or discard. A non-repo directory runs in place; pass `--no-worktree`
to work in the current directory directly. During a run the terminal shows
the current step (`[2/2] release`), tool calls and results, plus the worktree
and report paths; the full transcript is written to `ship-<timestamp>.md` as
it happens. First Ctrl-C cancels the run (exit 130); a second force-quits.

## Development

Requirements: [mise](https://mise.jdx.dev/getting-started.html) — nothing else.
`mise install` provides everything, including the `node` runtime that the
`web/` build scripts (`tsc`, `vite`) require.

```sh
mise install        # rust 1.98.1 + node 24 + pnpm 12.4.1
mise run test       # cargo test
mise run build      # cargo build
mise run run        # cargo run -p pulse (the CLI)
mise run lint       # web/ lint + format check
mise run web-build  # build the static site
mise run release-build  # release binary for aarch64-apple-darwin
```

## Updating

`pulse update` checks GitHub releases for a newer version; `pulse update
--apply` downloads the latest release asset and replaces the running binary
in place (restart pulse afterwards to run the new version):

```sh
pulse update          # show the latest release when one is newer
pulse update --apply  # download and install it
```

The repository is private, so the updater needs a token with read access to
it — export `GITHUB_TOKEN` or `MISE_GITHUB_TOKEN` (mise clients already have
the latter).

## Install (clients, macOS ARM only)

Releases are built by CI on tag push and publish a single asset per version:
`pulse-<tag>-aarch64-apple-darwin.tar.gz` containing the `pulse` binary. mise
installs it from GitHub releases and puts `pulse` on your PATH.

This repository is private, so clients need a GitHub token with read access to
it (fine-grained token with **Contents: read** is enough):

```sh
export MISE_GITHUB_TOKEN=<token with read access to the repo>
mise use -g "github:xdm67x/pulse@latest"
```

Each release publishes a single `aarch64-apple-darwin` asset, which mise
auto-detects on macOS ARM; install attempts on any other platform or
architecture fail with no matching asset. To pin a version instead of
tracking the latest release, replace `@latest` with a version (`@0.6.0`, …).

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
