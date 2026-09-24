# Aldwin

*A tool for thought.*

A terminal coding agent that would rather explain the code than rewrite it
behind your back. You describe a change in plain words; it reads and runs
what it needs, says what it is doing, and never edits a file directly. Every
edit opens as a full-window review, and nothing is saved until you approve.
The point is that **you** finish the session understanding the code, not
just holding a larger diff than when you started.

Built in Rust on [ratatui](https://ratatui.rs). Works with Anthropic or any
OpenAI-compatible endpoint.

> **Status:** 0.3.0 and actively developed. Linux is the best-supported
> platform — see the macOS note under Install.

## Install

Grab a binary from [Releases](../../releases), extract, put `aldwin` on your
`PATH`. No toolchain required. (The repo is private, so you'll need access.)

| archive | for |
| --- | --- |
| `…-x86_64-unknown-linux-gnu.tar.gz` | Intel/AMD Linux |
| `…-aarch64-apple-darwin.tar.gz` | Apple Silicon Macs |

No Intel Mac build. No Windows — `run` leans on Unix process APIs.

**macOS caveat worth knowing before you install:** Linux enforces a read-only
call with Landlock; macOS does it with Seatbelt, through `sandbox-exec`. The
macOS half is newer and less exercised than the Linux one. If it can't be set
up — or on a platform with no such primitive at all — the call runs with the
tree writable, and Aldwin tells you so once, at the first such call.

### Verify what you downloaded

Each release ships `SHA256SUMS` and a signature over it. Take
`allowed_signers` from **this repo**, not the release page — a key served
next to its own signature proves nothing.

```sh
ssh-keygen -Y verify -f allowed_signers \
  -I release@aldwin -n aldwin-release \
  -s SHA256SUMS.sig < SHA256SUMS

sha256sum -c SHA256SUMS
```

No new tools: `ssh-keygen` came with SSH.

For a release published before the project was renamed, swap in the old pair
— `-I release@mjolnir -n mjolnir-release`. It is the same key; only the
labels changed, and `allowed_signers` carries both.

This matters more here than for most downloads. Aldwin's whole pitch is that
nothing reaches your files without a review you approved and that a read is
enforced rather than trusted — none of which survives running a binary that
isn't the one built from the source you can read.

### Build from source

Needs [rustup](https://rustup.rs) — `rust-toolchain.toml` pins the compiler
to 1.98.1, and only rustup honours it.

```sh
cargo install --path crates/cli   # or: cargo build --release
```

Release builds are reproducible: pinned compiler, `Cargo.lock`, source paths
remapped out of the binary, no timestamps in the archive. Rebuilding a tag
gives byte-identical archives.

```sh
git checkout vX.Y.Z
scripts/release.sh build x86_64-unknown-linux-gnu
scripts/release.sh package
sha256sum -c SHA256SUMS      # the one from the release
```

Two honest limits: the macOS archive only reproduces *on a Mac* (Apple's SDK
licence), and without rustup the compiler pin does nothing — the script says
so rather than letting you wonder why the hashes differ.

## Run

```sh
export ANTHROPIC_API_KEY=sk-...
aldwin
```

That's the whole CLI. No flags, no subcommands, nothing but `--help` and
`--version`. Everything else lives in config files.

**Every launch opens straight to the field**, under a short card of what
Aldwin is working with: version, project, branch, model. There is no setup.
With nothing configured the card reads `Model  not set`, and your first
message is held while two questions ask which provider and which model —
then it goes. `/model` changes either later. Answers go to `~/.aldwin/`
(global) and `.aldwin/` (this project), both commented, both meant to be read
and edited.

Aldwin never stores your API key. `provider.yaml` holds the *name* of an
environment variable, and reads it at startup. For an OpenAI-compatible
endpoint:

```yaml
version: 1
provider: openai-compatible
model: mistral-small-latest
base_url: https://api.mistral.ai/v1/chat/completions
api_key_env: MISTRAL_API_KEY
```

(`base_url` is ignored for `provider: anthropic`, which knows its own address.)

If the project has a `CLAUDE.md` or `AGENTS.md`, it goes in the model's
context. Reading is a read.

## How a change happens

Say what you want. Aldwin leads with a sentence about what it's doing, reads
and runs what it needs without asking, and shows the plan as three plain
lines — *Count requests per key*, *Turn away requests over the limit*,
*Check that it works* — each marked done, running or pending. The work
behind them collapses to a line (`Read 3 files · Ran 1 program`) that
`Space` opens into exact paths and counts.

**Reads and runs need no permission.** A `run` is a program plus an argument
list, executed directly — `&&`, `|`, `;` and `$(…)` are just characters.
When the agent declares a call a read, it runs with your tree read-only and
the network unreachable; declare wrong and nothing lands, and the agent is
told to declare it again as a write. *(Landlock on Linux, Seatbelt on macOS;
see the note above.)*

**Every tool stays inside your workspace — `run` too.** A path argument that
points outside the project is refused before the program starts. Working
across sibling checkouts is a line in the project's
`.aldwin/permissions.yaml`:

```yaml
roots:
  - ../proton-libs
```

**A deny is a lock.** The other thing that file holds. A `deny:` entry —
`curl`, or `npm: write` — refuses the call outright, the refusal names the
file, and nothing narrower overrides it. Unlocking is a deliberate edit to
the file.

**Every edit is reviewed.** The `edit` tool writes nothing: it stages the
change, and everything staged in a turn is shown to you as one review — at
the end of the turn, or before any run that would see it, so tests run on
approved code. The review takes the whole window: a file tree with reading
progress, the diff with unchanged code folded away, and your comments riding
at the end of their lines. `⌃↩` approves once you've read every file, or
sends your comments back to the agent, which addresses them and stages the
edits again. `⎋` asks before discarding. Nothing is written until you
approve, under every setting, with no way to switch it off.

The one gap, stated plainly: this covers Aldwin's own `edit` tool. An MCP
server's tools are its own code, and what one of them writes is outside the
review.

**When the agent needs you**, it asks one question with a short list —
always a yes, a no, and *Chat about this*. `↑↓` and `↩`, or press the number.

## Sessions

Conversations are written to disk as they happen, one JSONL transcript per
session under `~/.aldwin/history/`, mode `0600`. `/resume` lists past
sessions in this project and picks one back up — into the transcript you see
*and* the context the model has.

Nothing crosses between sessions on its own. Resume is something you ask for,
by name; there's no cross-session memory and nothing gets summarised behind
your back.

Nothing prunes old transcripts yet. They're your files, in a directory you
own.

## Keys and commands

| key | does |
| --- | --- |
| `↩` / `⇧↩` | send / newline (`⌃J` where the terminal can't tell them apart) |
| `⎋` | stop the turn that's running |
| `⌃C` | stop the turn, or leave if nothing is running |
| `Space` | show or hide the details of the current turn's work (on an empty field) |
| `↑` `↓` | move within a multi-line draft, then scroll the transcript |
| `1`–`9`, `↩` | pick and confirm in any question |

In the review:

| key | does |
| --- | --- |
| click, drag | select a line, or a run of lines, to comment on; a click on `⋯  N lines` opens it |
| `⇧↑` `⇧↓` | select from the keyboard: the first line shown, then extend |
| `Space` | open every folded run in the file |
| `↑` `↓`, `PgUp` `PgDn` | scroll the diff, or move a selection |
| `↩` | comment on the selection |
| `⌃↩` | approve, or send the comments (a bare `↩` with nothing selected or typed does the same, for terminals that cannot tell them apart) |
| `⇥` `⇧⇥`, `←` `→` | next and previous file |
| `⎋` | clear the selection, then discard |
| `?` | show the keys |

The mouse wheel scrolls too. In the conversation the terminal keeps the
mouse, so selecting and copying text works the way it does anywhere else;
while a review is open Aldwin takes it, to select lines with (most
terminals still copy with a Shift-drag).

`/` in an empty field opens the commands: `/resume`, `/model`, `/quit`,
`/clear`. Also reachable by typing them: `/theme light|dark`,
`/reload-config`, `/help`.

## Layout

Cargo workspace, eight crates. `aldwin-review` is a dev-only harness that
lints, tests and screenshots the TUI, and never reaches a release build —
the release workflow builds `-p aldwin-cli` and nothing else. Traits live in
the crate owning the boundary, impls in the siblings that depend on it.

| crate | role |
| --- | --- |
| `aldwin-core` | agent loop, conversation log, event/command types, `LlmClient` and `ToolDispatcher` traits |
| `aldwin-config` | YAML config per domain, project and global scope, refuses to start on a half-deleted one |
| `aldwin-permissions` | the deny lock |
| `aldwin-tools` | read, edit, run, explain (LSP), plan, ask, the staged changeset, the read-enforcing sandbox, MCP bridge |
| `aldwin-llm` | Anthropic and OpenAI-compatible clients — reqwest, SSE, retry, prompt caching |
| `aldwin-tui` | the ratatui frontend, the review included |
| `aldwin-cli` | the `aldwin` binary — startup, wiring, slash commands |
| `aldwin-review` | dev-only: the review loop and screenshot harness |

Design notes live in `.claude/spec/`, and decisions that changed a stated
constraint in `.claude/adr/`. Read the relevant one before changing a crate.

## Development

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Two tests are `#[ignore]`d because they spawn a real `rust-analyzer`:

```sh
cargo test -p aldwin-tools -- --ignored
```

`cargo run -p aldwin-review -- review --goal "…" --focus "…"` runs the full
loop — lint, tests, design tokens, frame snapshots, and screenshots of the
real binary driven through a real terminal.
