# Amundsen

*A tool for thought.*

A coding agent harness where the developer's understanding is the product,
not the agent's throughput. Resting state is conversation — the agent reads,
explains, and proposes; it edits only on explicit signal ("apply this", "go
ahead"). Permissions are default-deny everywhere, and Edit is never
allowlistable: friction on Edit is structural, not a setting.

See `.claude/spec/amundsen.md` for the full design rationale.

## Status

V0, under active development. All seven crates in `.claude/spec/` are
implemented; four (`amundsen-core`, `amundsen-config`, `amundsen-llm`,
`amundsen-cli`) are feature-complete against their specs and archived under
`.claude/spec/archive/`. Three (`amundsen-permissions`, `amundsen-tools`,
`amundsen-tui`) are active with one or two disclosed, non-blocking gaps —
see the `Progress` note at the top of each spec file for specifics.

Not yet run against a real terminal or the live Anthropic API in this
project's own development environment — see those specs' notes before
relying on it for anything you can't afford to babysit closely.

## Install

The easiest path is a prebuilt binary from this repo's
[Releases](../../releases) page — no Rust toolchain needed. Grab the archive
for your platform (Linux or macOS, x86_64 or Apple Silicon; Windows isn't
supported yet — the shell tool relies on Unix process APIs), extract it,
and put `amundsen` on your `PATH`. Since this repo is private, you'll need
GitHub access to it to download release assets.

Building from source is the alternative — see [Prerequisites](#prerequisites)
and [Build](#build) below. It's also how new releases get made: pushing a
`vX.Y.Z` tag triggers `.github/workflows/release.yml`, which cross-builds all
platform binaries and attaches them to a GitHub Release automatically.

## Prerequisites

Only needed if you're building from source rather than using a release
binary above.

- **A Rust toolchain.** Once built, the resulting binary needs nothing
  Rust-specific to run.

  Use [rustup](https://rustup.rs/), not your OS package manager's `cargo` —
  this repo's lockfile needs a reasonably recent cargo (1.75 is too old to
  read it; 1.98 works) and rustup is the reliable way to get one:

  ```sh
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable --profile minimal
  source "$HOME/.cargo/env"
  ```

- **An API key for your provider.** Anthropic, or any OpenAI-compatible
  endpoint (Mistral, a self-hosted proxy, etc.) — see [Run](#run) below for
  the `provider.yaml` config for each.

- **[rust-analyzer](https://rust-analyzer.github.io/)** on `PATH`, optional —
  only needed for the `explain` tool's code-intelligence operations
  (definition, references, hover, implementations, workspace symbols), and
  only when working in a Rust project (the only language `explain` supports
  in V0). Everything else works without it. If you installed Rust via
  rustup: `rustup component add rust-analyzer`.

## Build

From the repo root:

```sh
cargo build --release
```

The binary is `target/release/amundsen`. To put `amundsen` on your `PATH`
instead:

```sh
cargo install --path crates/cli
```

## Run

```sh
export ANTHROPIC_API_KEY=sk-...
amundsen
```

Zero-arg binary — no flags or subcommands beyond `--help`/`--version`.
Everything else is driven by config files, not CLI arguments.

### First launch

The first run writes an annotated, fully-commented config to `~/.amundsen/`
(`permissions.yaml`, `provider.yaml`, `mcp.yaml`, `tui.yaml`) and exits — read
it, then run `amundsen` again. If `~/.amundsen/` already exists but is
missing one of those four files, Amundsen refuses to start rather than
silently filling the gap; restore the missing file or remove the directory
to reinitialize.

By default `provider.yaml` points `api_key_env` at `ANTHROPIC_API_KEY` — that
environment variable must be set before Amundsen will start. Amundsen never
reads or stores the key itself in config, only the variable's name.

To use an OpenAI-compatible provider instead (Mistral, a self-hosted proxy,
etc.), edit `provider.yaml`:

```yaml
version: 1
provider: openai-compatible
model: mistral-small-latest
base_url: https://api.mistral.ai/v1/chat/completions
api_key_env: MISTRAL_API_KEY
```

`base_url` only takes effect under `provider: openai-compatible`; Anthropic
always uses its own fixed endpoint regardless of what's set there.

If your project has a `CLAUDE.md` or `AGENTS.md`, you'll be asked — once,
synchronously, before the TUI launches — whether to include it in the
model's context, and at what scope (`[p]roject` persists the approval to
`.amundsen/context_files.yaml`, `[s]ession` approves for this run only,
`[n]o` declines). Nothing not explicitly approved is ever read into context.

### Permissions

Every tool call starts denied. The first session in a new project will
prompt for nearly everything it tries to do — that's deliberate; the
allowlist builds by encounter, not by upfront configuration. Each prompt
offers allow/deny at four tiers: once, this session, this project, always.
Edit is the one exception: it's never allowlistable at any tier, and always
shows a diff for per-call approval.

### Keybindings

- **Input:** `Enter` submits, `Shift+Enter` inserts a newline (`Ctrl+J` as a
  fallback on terminals that can't distinguish `Shift+Enter` from plain
  `Enter`). `Ctrl+C` cancels the active turn, or exits if none is running.
- **Scroll:** arrow keys or `j`/`k` (only when the input box is empty),
  `PageUp`/`PageDown`, `G`/`End` to jump to the bottom.
- **Edit approval card:** `y` approve, `n` deny.
- **Permission prompt card:** `o`/`s`/`p`/`a` = allow once/session/
  project/always; `O`/`S`/`P`/`A` = deny at the same tiers.

### Slash commands

- `/reload-config` — reloads all config layers from disk. A file that fails
  to parse keeps its previous in-memory snapshot (surfaced by path); files
  that parse fine still pick up the edit.
- `/exit`, `/quit` — ends the session, same as `Ctrl+C` with no turn running.

## Project layout

Cargo workspace, seven crates under `crates/`. Trait definitions live in the
crate that owns the boundary; concrete implementations live in siblings that
depend on it. Read the relevant file in `.claude/spec/` (or
`.claude/spec/archive/` for the finished ones) before changing any of them.

| Crate                  | Role                                                              |
|-------------------------|--------------------------------------------------------------------|
| `amundsen-core`          | Agent loop, append-only log, event/command types, `LlmClient`/`ToolDispatcher` trait defs |
| `amundsen-config`        | Per-domain YAML config, project/global scope, refuse-to-start validation |
| `amundsen-permissions`   | Default-deny permission engine — three scopes, tiered prompts     |
| `amundsen-tools`         | `ToolDispatcher` impl — Read, Edit, shell, Explain (LSP), MCP bridge (rmcp) |
| `amundsen-llm`           | `LlmClient` impls for Anthropic and OpenAI-compatible (Mistral, self-hosted proxies) providers — reqwest + SSE, retry, prompt caching |
| `amundsen-tui`           | ratatui frontend                                                   |
| `amundsen-cli` (`crates/cli`) | Binary crate (`amundsen`) — startup sequence, wiring, slash commands |

## Development

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

One crate (`amundsen-tools`) has an `#[ignore]`d integration test that
exercises real rust-analyzer indexing (too slow for routine runs):

```sh
cargo test -p amundsen-tools --lib tools::explain -- --ignored
```
