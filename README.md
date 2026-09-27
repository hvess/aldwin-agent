# Aldwin

*A tool for thought.*

Aldwin is a coding agent for your terminal that would rather talk the change
through with you than quietly rewrite half your repo.

It reads and runs whatever it needs, tells you what it's doing, and when it
wants to edit something, the edit waits for you. Every edit in a turn is
shown to you as one review, and nothing touches disk until you approve it.
The point isn't a bigger diff. The point is that you finish the session
actually understanding your code.

![Aldwin working through a plan](assets/plan.png)

Written in Rust on [ratatui](https://ratatui.rs). Linux and Apple Silicon.

> **Status:** 0.4.0, and moving fast. Linux is the best-tested platform.

## Install

Grab a binary from [Releases](../../releases), extract it, put `aldwin` on
your `PATH`. That's it. (The repo is private for now, so you'll need access.)

| archive | for |
| --- | --- |
| `…-x86_64-unknown-linux-gnu.tar.gz` | Intel/AMD Linux |
| `…-aarch64-apple-darwin.tar.gz` | Apple Silicon Macs |

No Intel Macs and no Windows, at least for now.

### Check what you downloaded

Every release ships a `SHA256SUMS` and a signature over it. Take
`allowed_signers` from **this repo**, not from the release page. A key sitting
next to its own signature proves nothing.

```sh
ssh-keygen -Y verify -f allowed_signers \
  -I release@aldwin -n aldwin-release \
  -s SHA256SUMS.sig < SHA256SUMS

sha256sum -c SHA256SUMS
```

`ssh-keygen` came with SSH, so there's nothing new to install. Releases from
before the rename were signed as `release@mjolnir` / `mjolnir-release`. Same
key, old name, and `allowed_signers` has both.

Why bother? Aldwin's whole deal is that nothing lands without your review.
That promise is worth nothing if the binary isn't the one built from this
source.

### Build it yourself

You'll need [rustup](https://rustup.rs). `rust-toolchain.toml` pins the
compiler to 1.98.1, and rustup is the only thing that reads that file.

```sh
cargo install --path crates/cli
```

Release builds are reproducible, so checking out a tag and running
`scripts/release.sh build <target>` then `scripts/release.sh package` gives you
the same bytes we shipped. One catch: the macOS archive only reproduces on a
Mac.

## Run it

```sh
aldwin
```

There are no flags or subcommands to learn, just `--help` and `--version`.
Everything else lives in config files you can read.

![The launch card](assets/launch.png)

The first time you run it, just type your question. Aldwin holds the message,
asks which provider and model you want, then sends it. You get a suggested
list: Anthropic, OpenAI, Google, xAI, Mistral, DeepSeek and Proton Lumo. You
can type any model id, and point `provider.yaml` at any OpenAI-compatible
endpoint:

```yaml
version: 1
provider: openai-compatible
model: mistral-small-latest
base_url: https://api.mistral.ai/v1/chat/completions
api_key_env: MISTRAL_API_KEY
```

Aldwin never stores your API key. `api_key_env` is the *name* of an
environment variable, and the key stays in your environment.

Got a SuperGrok subscription? `/connect` signs you in to your xAI account
instead, and an xAI model then uses the account before it tries a key. If
you have neither, you get one sentence telling you both ways to fix it.

If your project has a `CLAUDE.md` or `AGENTS.md`, the model reads it.

## How a change happens

You say what you want. Aldwin opens with a sentence about what it's doing,
shows its plan as a few plain steps, and gets on with it. The reads and runs
behind each step collapse into one line (`Read 3 files`), and `Space` opens
it up if you want to see the details.

**Reading and running don't need permission.** `run` is a plain shell
command, so pipes and `&&` work. Everything Aldwin starts (the command, the
language server, MCP servers) runs in a sandbox that can only write inside
your workspace, plus temp files and caches like `~/.cargo`. The kernel
enforces that (Landlock on Linux, Seatbelt on macOS). It isn't just trusted
to behave. If your system can't enforce it, Aldwin says so once when it
starts. It never pretends.

Your workspace is the project directory. Need a sibling checkout too? Add it
to `.aldwin/permissions.yaml`:

```yaml
roots:
  - ../proton-libs
```

That's the only knob. Reads and the network are open, on purpose. The
boundary is there to keep your files from changing behind your back, not to
hide them.

**Edits are staged, then reviewed.** The `edit` tool doesn't write anything.
Every edit in a turn lands in one changeset, and the review opens at the
turn's end, or before a `run` that would see the changes, so your tests run
on code you've approved.

![The review, with a comment on a line](assets/review.png)

The review takes over the whole window: files on the left, the diff on the
right with unchanged code folded away. Click or drag to select lines, `↩` to
comment, and `⌃↩` to send your comments back or to approve once you've read
every file. You can't switch the review off, and there's no "just this once".

MCP servers get no write access to the workspace at all. If one wants to
change a file, the agent does it with `edit`, and it goes through the review
like everything else. Two cases get around that: a system that can't
confine anything (you're told at startup), and a workspace kept under `/tmp`
or `~/.cache`, which every process can write. So maybe don't keep your
project in `/tmp`.

**When Aldwin needs you**, it asks one question with a short list of answers,
and the last one is always *Chat about this*.

**Commits it makes carry its name.** Any `git commit` Aldwin runs gets
`Co-Authored-By: Aldwin <noreply@aldwin.codes>`, so it's always clear who
helped.

## Sessions

Every conversation is saved as it happens, one file per session under
`~/.aldwin/history/`, readable only by you. `/resume` lists this project's
past sessions and picks one back up, both what you see and what the model
remembers.

Nothing carries over between sessions unless you ask for it. There's no
hidden memory and nothing summarized behind your back. Old sessions aren't
cleaned up automatically, because they're your files.

## Keys and commands

| key | does |
| --- | --- |
| `↩` / `⇧↩` | send / new line (`⌃J` if your terminal can't tell them apart) |
| `⎋` | stop the running turn |
| `⌃C` | stop the turn, or quit if nothing's running |
| `Space` | show or hide the current turn's work (on an empty field) |
| `1`–`9` | answer a question |

In the review, `?` shows every key.

Type `/` on an empty field to get the commands: `/resume`, `/model`,
`/connect`, `/clear`, `/theme`, `/quit` (or `/exit`). `/reload-config` and
`/help` work too, they just aren't in the menu.

## Config

All of it is commented YAML. Global settings live in `~/.aldwin/` and
project settings in `.aldwin/`.

| file | holds |
| --- | --- |
| `provider.yaml` | provider, model, which env var holds the key |
| `permissions.yaml` | extra workspace `roots:` (project only) |
| `mcp.yaml` | MCP servers, stdio or http |
| `tui.yaml` | theme (global only) |
| `connections.yaml` | accounts from `/connect` (global only, keep it private) |

Edited something while Aldwin is running? `/reload-config`.

## Hacking on it

It's a Cargo workspace of eight crates. Each boundary's trait lives in the
crate that owns it, and the implementations live in the crates that depend
on it.

| crate | does |
| --- | --- |
| `aldwin-core` | the agent loop, the conversation, the `LlmClient` and `ToolDispatcher` traits |
| `aldwin-llm` | Anthropic and OpenAI-compatible clients: streaming, retries, prompt caching |
| `aldwin-tools` | read, edit, run, explain (LSP), plan, ask, the staged changeset, the sandbox, MCP |
| `aldwin-tui` | the ratatui frontend, review included |
| `aldwin-config` | the YAML files, global and project |
| `aldwin-login` | the `/connect` device-code sign-in |
| `aldwin-cli` | the `aldwin` binary: startup, wiring, slash commands |
| `aldwin-review` | dev only, never ships: the review loop and the screenshot harness |

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

A few tests are `#[ignore]`d because they need a real `rust-analyzer` or a
live Lumo key. `cargo test -- --ignored` runs them if you have those.

Design notes are in `.claude/spec/`, and the decisions behind them are in
`.claude/adr/`. Read the relevant one before you change something. There's
usually a reason, and it's usually written down.

If you're an agent: every commit goes through `/review`. It runs lint, tests,
the design tokens, screenshots of the real binary, and then three blind
judges. The pre-commit hook won't let a commit in without a passing review,
so don't go looking for a way around it.
