# aldwin-cli

Binary crate — startup sequence, session bootstrap, impl wiring, slash-command dispatch.

**Status:** archived — implemented, tested, audited
**Scope:** crates/cli
**Owner:** Maximilian
**Last Updated:** 2026-06-10

**Completed:** 2026-08-29 — `20a8d39`, plus an audit fix (`f023d4a`) that
wired up context-file approval (was implemented in aldwin-permissions
and aldwin-tui but never actually invoked from here — the session
initializer now does exactly what this spec's Design section says: tests
each candidate file before composing additional-context). No known gaps
against this spec. Manually verified against the real binary: --help/
--version, first-launch init, refuse-to-start on a missing env var, and
the full startup path through TUI launch — but not yet a full live session
against the real Anthropic API (no API key in this environment).

**Post-archive addition (2026-08-29):** A live run's developer had no
discoverable way to end a session short of `Ctrl+C` (itself undiscoverable
until fixed in aldwin-tui the same day) and asked for a slash command.
Added `/exit` to the dispatch table — `Intercepted::Quit`
makes `run_interceptor` return instead of looping again, which drops its
`forward` and `events` sender clones; the core's command channel then
closes, the core drains and drops its own `events` sender, and the TUI's
event channel closes the same way it already does on `None` — no new
`Event` variant, no core changes. This was explicitly out of scope before
("Additional slash commands beyond /reload-config — V0 set is minimal"),
not a gap against the original spec.

Also added `/help`, listing all three commands (`/help`, `/exit`,
`/reload-config`) from a single `HELP_TEXT` constant kept in sync by hand
with the `match` in `intercept` — three commands doesn't earn a
data-driven dispatch table yet. The unknown-command Notice now points at
`/help` too.

**Post-archive addition (2026-08-29, `/clear`):** Part of aldwin-tui's
same-day live-feedback batch (see its own spec). Unlike every other known
command, `/clear` is translated and forwarded (`Intercepted::Forward(Command::ClearHistory)`)
rather than handled locally — core owns `ConversationLog`, so only core
can actually wipe it; `intercept` returns `Forward` instead of sending a
`Notice` and returning `Handled` the way `/help`/`/reload-config` do. This
doesn't reopen the Decisions section's "core has no slash-command
semantics" — `ClearHistory` is a generic core operation core would accept
from any caller, the same way `Submit`/`Cancel` are; the CLI layer still
owns 100% of the `/`-prefix parsing and dispatch table, it's just that this
one entry's action lives in core rather than in this crate. `HELP_TEXT`
updated to include it.

**Post-archive addition (2026-09-02, `/theme`):** Follow-up to aldwin-tui's
same-day light-theme addition (see its own spec) — the developer asked for
a way to switch themes from inside the harness rather than hand-editing
`tui.yaml`. `/theme light|dark` is closer to `/reload-config` than to
`/clear`: it never touches core at all. `handle_theme` reads/writes
`tui.yaml` directly (`Config::global_tui`/`set_tui`, already existed) and
sends `Event::ThemeChanged { theme }` straight into the same channel the
TUI reads from — this doesn't reopen "core has no slash-command semantics"
below any more than `PermissionsChanged` did; it's a config write plus a
UI-facing signal, not a core operation. `/theme` with no argument reports
the current value rather than erroring (mirrors `/help`'s "tell the
developer where they stand" instinct, since this process has no other way
to see what a *running* TUI is currently showing than what's already on
disk — the two should always agree, since this command is the only thing
that changes either). An invalid value (anything but `light`/`dark`,
case-insensitive) is rejected with a `Notice` and neither persists nor
emits `ThemeChanged` — confirmed by test, not just by the validation read.
`HELP_TEXT` updated to include it. See aldwin-tui.md's matching Progress
note for the `App`/`ui.rs` side (switches live, no restart, since `App::
theme` is read fresh on every draw) and aldwin-core.md's for the new
`Event` variant.

**Post-archive addition (2026-09-06, `/model`):** Follow-up to aldwin-tui's
same-day first-run rework, where screen `5d`'s first step became *provider*
rather than *model* and its prose promises "/model picks a model once the
session starts". That clause had been dropped from the frame twice for
naming a command that did not exist; this is the command.

One command, not two, because the two halves are not separable: a model id
means nothing without the provider whose catalogue it comes from, and
picking a provider with no model would leave `provider.yaml` incomplete.
`/model [provider/]model` splits its argument on the *first* `/` only. A
name the catalogue knows (`aldwin_llm::PROVIDERS`) is a provider whether or
not a slash follows it; anything else with no slash is a model id on the
provider already configured. Everything after that first slash is the model
— so `openrouter/qwen/qwen3-coder` reaches the right place. A slashed argument
whose first segment is *not* a provider is rejected rather than read as a
slashed model id: both readings are available, and the rejected one is what
a mistyped provider looks like (`gogle/gemini-2.5-pro` would otherwise be
written verbatim as a model on whatever provider was already set, and
reported as success).

The provider half is validated; the model half is not. A provider decides
an endpoint, a wire dialect and a key variable, none of which can be
guessed from a name. A model id is a plain string in `provider.yaml`, and a
provider's real catalogue is a network call away and changes without us —
so the notice lists what the harness *knows* rather than claiming to know
all of it.

Two things it does differently from `/theme`, both because the truth is
different:

1. **It writes the scope that actually supplies the setting** — project
   `provider.yaml` when one exists, global otherwise. Writing global while
   a project file shadows it would report a change the next start ignores.
2. **It does not take effect now, and says so.** The `LlmClient` is
   constructed in `bootstrap::run` and moved into the agent task, which
   owns it for the life of the process; there is no way to swap it under a
   running turn. `/theme` really does apply on the next redraw, so it
   promises that; this one persists the choice and states plainly that the
   session keeps what it started with. Emitting an event that redrew the
   top bar with the new model name would have been the easy lie.

   The model it names there is threaded in — `run_interceptor` takes a
   `session_model: String` captured in `bootstrap::run` beside the client it
   describes. Reading the current setting back off disk was the first cut
   and was wrong the moment the command was used twice in one session: the
   second call reported the *first* call's write as what the session was
   running. Caught by previewing the notices rather than by a test, and now
   pinned by `the_session_model_reported_is_the_one_the_process_started_with`.

`HELP_TEXT` updated to include it. The catalogue itself lives in
aldwin-llm (see its own post-archive note): first run and this command
read the same list, so a provider added there appears in both without
either being edited.

**Post-archive addition (2026-09-06, first run asks for a provider):**
`bootstrap::run`'s `needs_model` is now `needs_provider`, and the answer it
writes comes off a catalogue row rather than being hard-coded Anthropic —
`DEFAULT_API_KEY_ENV` is gone with it. The CLI is what joins the two
crates that must not depend on each other: it maps `aldwin_llm::PROVIDERS`
into `aldwin_tui::ProviderChoice` (id and purpose, nothing else) and hands
the display list to `run_first_run`, then maps the returned id back to the
catalogue row to build the `ProviderConfig`. `Answers::provider` is an
`Option`, so the access-only run — a new directory under an already
configured provider — cannot overwrite a provider it never asked about.


**Post-archive addition (2026-09-06, `/model` audit):** An audit of the
command as first written found two ways it wrote something the developer
did not ask for. Both are fixed and pinned.

1. **A bare provider name was written as a model id.** The slashed form was
   validated against the catalogue and the bare form was not, so `/model
   openai` wrote `model: openai` onto whatever provider was already
   configured and reported success — the failure only surfacing at the next
   start, as the host rejecting a model it had never heard of. A name the
   catalogue knows is now a provider in *either* form, `/model openai` and
   `/model openai/` being the same instruction. The provider half also folds
   case now, since `/theme` already accepts `LIGHT`; the model half is left
   exactly as typed, because a host compares it byte for byte.
2. **Naming the provider you were already on reset your model.** `/model
   anthropic` on `anthropic/claude-opus-5` took the catalogue default and
   dropped you to `claude-sonnet-5`. Naming where you already are is not a
   request to be moved, so the current model is kept and the command reports
   "already on …".

Also from the same pass: `bootstrap` writes first run's access answer only
when the access question was actually asked. See `docs/spec/aldwin-tui.
md`'s entry of the same date — `add_grant` only ever adds, so an unasked
answer could only widen an allow list the developer had already settled.


**Post-archive addition (2026-09-06, the model selector):** Reported: the
selector "doesn't appear in the onboarding", and `/model` "says the model is
already selected when it isn't". Three changes here; the screens and the
picker are in `docs/spec/aldwin-tui.md`'s entry of the same date, and
the reason onboarding never appeared is in `aldwin-config.md`'s (init
seeded `provider.yaml`, so `needs_provider` was never true).

* `catalogue_choices()` is now the one place `aldwin_llm::PROVIDERS` is
  mapped into `aldwin_tui::ProviderChoice` — used by both first run and the
  session, and carrying each row's models, since first run now asks which
  model too. `first_run_provider_config` takes that answer;
  the catalogue default is the fallback for a row that offers no models, not
  the normal path.
* `aldwin_tui::run` takes a `SessionProvider` — the catalogue, plus the
  catalogue id of the row `provider.yaml` actually resolves to (from
  `identify(&effective_provider)`, the file that supplies the setting rather
  than the global one it may be shadowing). That is what the picker opens on
  and marks as current.
* Bare `/model` is read by the TUI and opens that picker, which answers by
  submitting `/model <provider>/<model>`. `handle_model` is unchanged and
  still does every write: the frontend owns how the question is asked and
  nothing about what the answer does, so there is no second implementation
  of the command to disagree with this one. The bare form still reaches
  `handle_model` when no catalogue was handed in, and reports as before —
  and its "already on …" now says where the list is rather than dead-ending,
  which is the message the report was about.


**Post-archive fix (2026-09-24, audit):** A panic in the agent or the
interceptor task was awaited with `let _ =` and the process exited 0; it is
`StartupError::TaskFailed`, printed after the terminal is restored, with a
non-zero exit. `/clear` and `/resume` no longer move the transcript writer
themselves — core does, through `RecordSink`, when it acts (aldwin-core.md's
same-day entry). `History` takes the project root rather than re-reading the
working directory, and `ModelSwitch` and `History::open` return typed errors.
`tests/binary.rs` runs the binary through `assert_cmd`: `--version`, a
refused argument, and a malformed `provider.yaml` refusing to start.

**Post-archive addition (2026-09-27, ADR 0013, the git shim):** Before the
startup sequence, `main` does two things, and so is no longer
`#[tokio::main]`. Started as `git` — its file name, through a symlink — the
binary is the git shim (`git_shim.rs`): it execs the real git from further
down `PATH`, with `--trailer 'Co-Authored-By: Aldwin <noreply@aldwin.codes>'`
after a `commit`, and exits without parsing a flag. Otherwise, after clap
and before the tokio runtime starts a thread, it installs the shim: a `0700`
temp directory holding a `git` symlink to itself, put first on the
process's `PATH` so every process Aldwin starts inherits it, and removed on
a clean exit. Unix only. `run` takes the install's error, if any, and says
once at the top of the session that commits will not name Aldwin.
`tests/git_shim.rs` commits through the shim against real git;
`tests/git_shim_sandboxed.rs` commits through the real `run` tool, confined.

## Design

- **Invocation:** Zero-arg binary. `aldwin` starts a session rooted at the current working directory. No runtime flags, subcommands, or environment overrides in V0 — everything driven by config files. (Since ADR 0013, 2026-09-27: started under the name `git`, the binary is the git shim instead, and a session puts that shim first on its own `PATH`.)
- **Startup Sequence:**
  1. init_global_if_empty — first launch writes annotated global config; PartiallyPresent → refuse to start.
  2. Load all config layers — refuse to start on any parse failure, schema error, unknown major, or missing env var (error to stderr, non-zero exit; TUI has not yet launched).
  3. Build additional-context string from cwd path and approved context file contents.
  4. Instantiate concrete impls: AnthropicClient (aldwin-llm), PermissionsEngine (aldwin-permissions), ToolDispatcher (aldwin-tools).
  5. Create the agent loop (aldwin-core) with LlmClient, ToolDispatcher, and additional-context.
  6. Launch TUI (aldwin-tui) with the core's event receiver and command sender.
  7. Block on TUI exit; drop channels; wait for core to drain cleanly.
- **Additional Context:** Opaque string handed to aldwin-core. Contains: absolute cwd path, then the full text of each approved context file (CLAUDE.md / AGENTS.md) from the project_context_files() snapshot, in path order. Files not in the approved list are excluded regardless of existence on disk. The core composes `<base_system_prompt>\n\n<additional_context>` and sends it verbatim.
- **Slash Commands:** Input that begins with `/` is intercepted at the CLI layer before the Submit command reaches the core. The CLI maintains a dispatch table of known slash commands. Unknown slash commands are rejected with an error message in the TUI; they do not reach the core. Known V0 commands: /reload-config.
- **Reload Config:** `/reload-config` calls config.reload_all(). On success, re-initializes the PermissionsEngine from the new snapshot and notifies the TUI. On failure, previous snapshot is retained and the failing file path is surfaced to the TUI verbatim.

## Decisions

- **Zero-arg binary in V0 — no flags, no subcommands.** — All runtime configuration belongs in config files. CLI flags duplicate the config surface and create implicit override precedence rules. V0 has one provider and one mode of operation.

- **Refuse-to-start errors go to stderr before the TUI launches.** — The TUI is not available yet. A clean stderr message with the exact failing path is more actionable than launching a partially-initialised TUI to show the same error.

- **Slash commands intercepted at the CLI layer, not forwarded to the core.** — The core's only input is Submit, Cancel, ApproveTool — it has no slash-command semantics. CLI owns the dispatch table so slash commands can trigger config, TUI, or process operations that the core has no visibility into.

- **Additional-context string includes only approved context files.** — Reading arbitrary project files without a permission grant would silently bypass the default-deny model. The project_context_files() snapshot is the authorised list.

## Steps

1. Add crates/cli — Cargo.toml depending on all six sibling crates plus clap (--help/--version only).

2. Implement startup sequence — init_global_if_empty → load config → refuse-to-start path.
   - Verify: Missing env var, malformed YAML, and PartiallyPresent each produce a distinct stderr message quoting the exact path or var name.

3. Build additional-context string — cwd path + approved context file contents from project_context_files() snapshot.
   - Why: Files present on disk but absent from the approved list must be excluded — permission check is not optional.

4. Instantiate AnthropicClient, PermissionsEngine, ToolDispatcher; wire into a new agent loop.

5. Launch TUI with event/command channels; block until TUI exits.

6. Implement slash-command interceptor — parse `/`-prefixed input before channel send; reject unknowns to TUI.

7. Implement /reload-config handler — reload_all(), re-init PermissionsEngine, notify TUI; surface failing path on error.

8. Implement clean exit — TUI exit drops command sender; core drains and shuts down; process exits 0.

## Pitfalls

- Context file read bypassing the approved list — always gate on project_context_files() snapshot, not a raw filesystem walk.
- Slash commands reaching the core as Submit commands — interceptor must run synchronously before the channel send, not as a post-send hook.
- Refuse-to-start error messages that paraphrase the failing field rather than quoting it — quote the exact path, env var name, or domain file verbatim.
- Reload re-instantiating the PermissionsEngine from stale config on failure — retain previous PermissionsEngine snapshot when reload_all() returns an error.

## Out of Scope

- CLI flags, subcommands, or env-var overrides — zero-arg in V0.
- Shell completions.
- Multiple concurrent sessions or session multiplexing.
- Additional slash commands beyond /reload-config — V0 set is minimal.
- Config editing via CLI (writing provider, MCP, or permission entries from flags).
- Daemonisation or background operation.

## References

- docs/spec/aldwin.md — parent spec; binary crate role and dependency list.
- docs/spec/aldwin-core.md — agent loop, additional-context contract, event/command channels.
- docs/spec/aldwin-config.md — startup sequence, init_global_if_empty, refuse-to-start rules.
- docs/spec/aldwin-permissions.md — PermissionsEngine init from config snapshot.
- docs/spec/aldwin-tools.md — ToolDispatcher instantiation.
- docs/spec/aldwin-tui.md — TUI launch, channel wiring, /reload-config surface.
