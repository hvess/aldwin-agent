# Mjolnir

*A tool for thought.*

A coding agent harness where the developer's understanding is the product,
not the agent's throughput. Resting state is conversation — the agent reads,
explains, and proposes; it edits only on explicit signal ("apply this", "go
ahead"). Permissions are default-deny everywhere, and Edit is never
allowlistable: friction on Edit is structural, not a setting.

See `.claude/spec/mjolnir.md` for the full design rationale.

## Status

V0, under active development. All seven crates in `.claude/spec/` are
implemented; four (`mjolnir-core`, `mjolnir-config`, `mjolnir-llm`,
`mjolnir-cli`) are feature-complete against their specs and archived under
`.claude/spec/archive/`. Three (`mjolnir-permissions`, `mjolnir-tools`,
`mjolnir-tui`) are active with one or two disclosed, non-blocking gaps —
see the `Progress` note at the top of each spec file for specifics.

Not yet run against a real terminal or the live Anthropic API in this
project's own development environment — see those specs' notes before
relying on it for anything you can't afford to babysit closely.

## Install

The easiest path is a prebuilt binary from this repo's
[Releases](../../releases) page — no Rust toolchain needed. Grab the archive
for your platform (Linux or macOS, x86_64 or Apple Silicon; Windows isn't
supported yet — `run` relies on Unix process APIs), extract it, and put
`mjolnir` on your `PATH`. Since this repo is private, you'll need GitHub
access to it to download release assets.

**On macOS, reads cannot be enforced.** The sandbox that holds a
read-declared call to its word is Linux-only (see [Permissions](#permissions)),
so a macOS build asks about every call instead of running reads unattended.
That is the honest fallback rather than a silent downgrade, but it is a
materially different experience and worth knowing before you install.

### Verifying a release

Each release carries a `SHA256SUMS` covering every archive, and a detached
signature over that file. **Take `cosign.pub` from this repository, not from
the release page** — a key served from the same place as the signature
proves nothing.

```sh
cosign verify-blob \
  --key cosign.pub \
  --signature SHA256SUMS.sig \
  --insecure-ignore-tlog=true \
  SHA256SUMS

sha256sum -c SHA256SUMS
```

`--insecure-ignore-tlog` is expected here and is not a corner being cut. The
signature is deliberately kept out of Sigstore's public transparency log,
which would otherwise publish this private repository's name, its workflow
path and the timing of every release to a permanent public record. The flag
disables a check that does not apply.

Verification is worth doing on a tool like this specifically: Mjolnir's whole
claim is that an edit cannot land without a diff you accepted and that a read
is enforced rather than trusted. None of that survives running a binary that
is not the one built from the reviewed source.

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

The binary is `target/release/mjolnir`. To put `mjolnir` on your `PATH`
instead:

```sh
cargo install --path crates/cli
```

## Run

```sh
export ANTHROPIC_API_KEY=sk-...
mjolnir
```

Zero-arg binary — no flags or subcommands beyond `--help`/`--version`.
Everything else is driven by config files, not CLI arguments.

### First launch

The first run writes an annotated, fully-commented config to `~/.mjolnir/`
(`permissions.yaml`, `provider.yaml`, `mcp.yaml`, `tui.yaml`) and exits — read
it, then run `mjolnir` again. If `~/.mjolnir/` already exists but is
missing one of those four files, Mjolnir refuses to start rather than
silently filling the gap; restore the missing file or remove the directory
to reinitialize.

By default `provider.yaml` points `api_key_env` at `ANTHROPIC_API_KEY` — that
environment variable must be set before Mjolnir will start. Mjolnir never
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
`.mjolnir/context_files.yaml`, `[s]ession` approves for this run only,
`[n]o` declines). Nothing not explicitly approved is ever read into context.

### Permissions

Every call starts denied, and the allowlist builds by encounter rather than
by upfront configuration — so a first session in a new project asks about
nearly everything. See `.claude/adr/0004-…` for why the model has the shape
it does.

**A grant is a program and a class** — `git: read`, `cargo: write` — written
to `permissions.yaml` in the project's `.mjolnir/` or in `~/.mjolnir/`. The
class belongs to the *call*, not the program: `git status` is a read and
`git push` is a write, and they are the same binary.

**There is no shell.** A call names a program and an argument list, executed
directly, so `&&`, `|`, `;` and `$(…)` are ordinary characters with no power
to chain a second command onto an approved first one.

**A read declaration is enforced, not believed.** The agent declares what
each call does; a call it declares a read is executed with your source tree
read-only and the network unreachable. If it tries to write anyway, nothing
lands — you are asked whether to allow it as a write and it runs again. This
is why a wrong declaration costs a prompt rather than a tree. *Linux only*:
it needs Landlock, and where that is unavailable a `read` grant cannot be
honoured, so every call asks.

**A deny is a lock.** Nothing narrower overrides it — not the other file,
not a session, not a single turn — so a locked call is refused without a
prompt, because there is no answer that would lift it. Undoing one is a
deliberate edit to the file that holds it.

Each scope also carries a standing rung — `ask`, `read` or `write` — for
anything no entry covers, and the narrower file wins outright.

**Editing is outside all of it.** Not a grant, not a rung, not a row on any
prompt: every edit shows a diff and waits, under every setting, with no way
to turn it off.

### Log rendering

Each speaker gets its own color: user input is green, assistant text is
bold white with a leading `●`, a slash command is dim (it never reaches the
model), and tool/status text stays dim. Fenced code blocks (` ```lang `) in
assistant output render as a bordered block with real syntax highlighting
instead of raw backticks — an unrecognized or missing language tag falls
back to unhighlighted (but still bordered) text rather than refusing to
render.

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

- `/help` — lists the commands below.
- `/exit` — ends the session, same as `Ctrl+C` with no turn running.
- `/reload-config` — reloads all config layers from disk. A file that fails
  to parse keeps its previous in-memory snapshot (surfaced by path); files
  that parse fine still pick up the edit.

## Project layout

Cargo workspace, seven crates under `crates/`. Trait definitions live in the
crate that owns the boundary; concrete implementations live in siblings that
depend on it. Read the relevant file in `.claude/spec/` (or
`.claude/spec/archive/` for the finished ones) before changing any of them.

| Crate                  | Role                                                              |
|-------------------------|--------------------------------------------------------------------|
| `mjolnir-core`          | Agent loop, append-only log, event/command types, `LlmClient`/`ToolDispatcher` trait defs |
| `mjolnir-config`        | Per-domain YAML config, project/global scope, refuse-to-start validation |
| `mjolnir-permissions`   | Default-deny permission engine — three scopes, tiered prompts     |
| `mjolnir-tools`         | `ToolDispatcher` impl — Read, Edit, Run, Explain (LSP), the read-enforcing sandbox, MCP bridge (rmcp) |
| `mjolnir-llm`           | `LlmClient` impls for Anthropic and OpenAI-compatible (Mistral, self-hosted proxies) providers — reqwest + SSE, retry, prompt caching |
| `mjolnir-tui`           | ratatui frontend                                                   |
| `mjolnir-cli` (`crates/cli`) | Binary crate (`mjolnir`) — startup sequence, wiring, slash commands |

## Development

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

One crate (`mjolnir-tools`) has an `#[ignore]`d integration test that
exercises real rust-analyzer indexing (too slow for routine runs):

```sh
cargo test -p mjolnir-tools --lib tools::explain -- --ignored
```
