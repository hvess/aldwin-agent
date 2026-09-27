# ADR 0012 — A connected account is tried before an API key

**Status:** accepted, 2026-09-26. Revised the same day, before it shipped:
the first cut had `/login`, a `login:` field in `provider.yaml` and a
key-or-account question in the model flow; the developer's direction
replaced all three with what is below.
**Supersedes:** nothing. Amends one sentence of the archived
`aldwin-config.md` — "there is deliberately no field a plaintext key could
go in" — which stays true of `provider.yaml` and gains a second file it is
not true of.
**Affects:** `aldwin-config` (`connections.yaml`), `aldwin-llm` (the xai
catalogue row, `Auth`, `Transport`), `aldwin-cli` (`/connect`,
`connect::reach`, `build_client`), `aldwin-tui` (the `/connect` list, the
failure sentence), `aldwin-login` (the crate this is for),
`crates/review/baseline.json`

## Context

Every provider so far is reached with an API key the developer exports,
and `provider.yaml` names the variable, never the key. That kept a secret
out of every file Aldwin writes, and it kept the model of "how a provider
is reached" to one shape.

x.ai now lets a SuperGrok or X Premium subscriber run inference on the
subscription by connecting an agent to their account at `auth.x.ai` — no
metered key. Connecting leaves behind a refresh token that has to outlive
the process, or every start is a new sign-in; and a refresh token is a
secret. So the constraint that Aldwin stores no secret cannot hold as
stated.

## Decision

**Connecting an account and choosing a model are separate acts, and the
model's provider is reached through a connected account if there is one,
through its key if not.**

1. **`/connect` connects an account.** Bare, it lists the providers that
   offer one — xai today — with the subscription each needs; picking one,
   or typing `/connect xai`, shows a URL and a code as a notice. When the
   account approves, it is stored, and if the session is already on that
   provider it moves onto the account at once.

2. **Choosing a model does not ask how.** `provider.yaml` is unchanged: it
   names the provider, the model and the key variable, as it always has.
   Whenever a client is built — at startup, and on every `/model` — a
   provider that offers an account is reached in this order:
   1. the connected account, when `connections.yaml` has one for it;
   2. the key, when the variable `provider.yaml` names is exported;
   3. neither: the model is still chosen, and every request answers with
      one sentence — the key is read when a client is built, and a variable
      exported in the shell afterwards never reaches the running process,
      so the sentence says to start Aldwin again — "No x.ai account is connected and XAI_API_KEY is not
      set. Connect one with /connect xai, or export the key and pick the
      model again." A provider that offers no account is reached through
      its key exactly as before, and a missing key still refuses the swap.

3. **The account's tokens live in one global file.**
   `~/.aldwin/connections.yaml`, one entry per provider, written by
   `/connect` and by every refresh that rotates the token, with owner-only
   permissions. Never at project scope: a token inside a repository is a
   leak waiting to be committed. Its header says what it holds and that
   deleting an entry disconnects it.

4. **The device-code flow is the only flow** (RFC 8628). Aldwin shows a
   URL and a code; the developer signs in wherever they like. Nothing
   listens on a port and no browser is opened from Aldwin, so ADR 0011's
   sandbox has nothing to make room for, and it works over SSH.

5. **The sign-in is a crate that knows nothing of Aldwin.** `aldwin-login`
   turns an account into a prompt, credentials and a session that hands
   out fresh request headers; nothing OAuth-shaped crosses its surface, the
   way nothing of a provider's wire crosses `LlmClient`.

6. **The client id is xAI's own Grok Build CLI's.** It is a public client
   with no secret, xAI offers no registration for a third party's own, and
   Zed, Warp and LiteLLM reuse it. It is the one thing here xAI could
   withdraw, and the crate says so beside the constant. If it is withdrawn,
   the key is the whole feature.

7. **`/connect` joins the shipped commands**, beside `/theme` and the rest
   the design does not name (`baseline.json`,
   `frame-command-list-is-not-the-products`). Its prompt and its outcome
   are notices and its list is a QuestionPanel; the design system has no
   connection component, and one line in `baseline.json` records that.

## Consequences

**Two files now hold what the developer would not want read.** The
transcripts under `history/` (ADR 0005) and `connections.yaml`. Both are
owner-only and both are global scope. The refresh token is the more
valuable: it acts as the developer at that provider until revoked. The
floor is the file's mode, its header and the redacted `Debug` on every
type that carries a token; there is no encryption at rest, and that is a
known gap rather than a judgement that none is needed.

**A connected account wins over an exported key.** A developer with both
runs on the subscription. Going back to the key means deleting the entry
from `connections.yaml`, which the file's header says. `/disconnect` is
not built until it is missed.

**A refresh that fails is not a disconnect.** The session keeps the old
credentials and the next request tries again; only a refusal in the
protocol's own shape ends the connection. Then the entry is removed from
`connections.yaml`, so the next client built on the provider falls back
to the key, and the running session answers "Connect it again with
/connect xai."

**Two Aldwin processes on one account do not share a refresh.** Each keeps
its own tokens in memory and writes its own rotations back. If x.ai
rotates the refresh token, the second process's next refresh is refused
and that session is disconnected until `/connect` runs again. Reading the
stored entry back before every refresh would fix it, and is not built
until it is seen.

**An error Aldwin raised without sending anything is its own sentence.**
The failure row led with "The provider kept failing" for every terminal
error, including the ones where no provider was asked. A zero-attempt
error's message is now the row's sentence, which also fixes the
unconfigured stand-in's. (Since 2026-09-27 the zero-attempt error is
`LlmError::NotSent`, chosen by type: aldwin-tui's Progress entry "a failed
turn carries its kind".)

**The ChatGPT case was checked and left.** Its device flow is not RFC
8628, its inference is the Responses API, and OpenAI has moved against
third-party use of subscription tokens before. The seams for it exist in
the crate and in `Transport`; building it is a separate decision on the
day, with that policy question answered first.
