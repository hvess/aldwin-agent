# aldwin-login

Connecting a provider's account, so a subscription can stand in for an API key.

**Status:** active — built and wired end to end; the live run and the review scenes are open
**Scope:** aldwin-login crate, and the four wiring steps below that reach it. Excludes any second account (see *What a second account adds*).
**Owner:** Maximilian
**Last Updated:** 2026-09-26 — revised to `/connect`; see Progress

## Why

x.ai lets a SuperGrok or X Premium subscriber run inference on the
subscription, with no metered key, by connecting an agent to their account
at `auth.x.ai`. The developer runs `/connect`, picks x.ai, and signs in in
their browser; from then on a model on xai runs on the account rather than
a key. The sign-in and the session it leaves are what this crate does.

## The crate

A leaf: it depends on no other workspace crate, and aldwin-llm and
aldwin-cli depend on it. Its surface is four types and nothing OAuth-shaped
crosses it — the same rule aldwin-llm keeps for wire types.

| Type | What it is |
|---|---|
| `Account` | The closed list of accounts that can be logged in to. `Xai` today. |
| `Login` + `Prompt` | One login: `Login::start(account)` asks the server for a code and returns the prompt to show (URL, code, how long it is good); `Login::wait(self)` polls until the account approves. |
| `Credentials` | Access token, refresh token, expiry. Plain data, for whoever keeps it on disk. |
| `Session` | A logged-in account. `headers()` yields the request headers on a token that will still be good when the request lands, refreshing under one lock so concurrent requests refresh once, and calling `persist` with every rotated set. `invalidate()` marks the token refused, for the one 401 the inference endpoint may answer before the stated lifetime is up. |

Two error enums, each variant something the caller can say or act on:
`LoginError::{Denied, Expired, Failed}` and
`SessionError::{LoggedOut, Failed}`. `LoggedOut` means the refresh token
was rejected and only a new login helps; `Failed` is the server's passing
trouble, and the old credentials stay for the next request to try again.

**The device authorization flow (RFC 8628) is the only flow.** Nothing
listens on a port and no browser is opened, so it works over SSH and needs
nothing from the sandbox every process Aldwin starts runs in (ADR 0011).
The developer opens the URL wherever they like. Verified live on
2026-09-25: the device endpoint issued a code for the client below with a
30-minute window and a 5-second interval.

**The client id is xAI's own Grok Build CLI's** — a public client with no
secret, and xAI offers no registration for a third party's own. Zed, Warp,
LiteLLM and DSPy reuse it. It is the one thing here xAI could withdraw, and
`account.rs` says so beside the constant.

Tests run against a canned local HTTP server (`test_server.rs`, the shape of
aldwin-llm's) that keeps every request body, so each test asserts the grant
that was actually sent. Not yet run against a real subscription: the llm
spec records that every provider's real gaps showed up on the first live
run, and this one will be no different.

## Wiring — done

ADR 0012 records the decisions, including the revision that replaced the
first cut's `/login` and `login:` field. How it reaches the app:

- [x] **aldwin-config.** `provider.yaml` unchanged. A new global-only
  `~/.aldwin/connections.yaml`, versioned, one entry per provider, never
  at project scope. `Config::connection` / `set_connection`;
  `ConnectionRecord` with a redacted `Debug`; a test reads the file's mode
  back as `0600`.
- [x] **aldwin-llm.** The xai catalogue row with `account:
  Some(Account::Xai)`. `Auth::{ApiKeyEnv, Connection(Arc<Session>)}`.
  `Transport::with_account` resolves the session's headers per attempt
  through a two-method `Bearer` port; a 401 from the inference endpoint
  invalidates the token and sends the request once more, a second 401 is
  the endpoint's answer, and a session the account server no longer
  honours is a 401 whose message names `/connect xai`.
- [x] **aldwin-cli.** `connect::reach` decides, whenever a client is
  built: a connected account, else the exported key, else a client (`Said`)
  that answers every request with the sentence naming both fixes. A
  provider with no account on offer is untouched. `/connect <provider>`
  shows the code as a notice, spawns the wait so `/quit` stays answerable,
  stores the connection, and moves the session onto it when the session is
  already on that provider.
- [x] **aldwin-tui.** `ProviderChoice::account` carries the subscription
  an account needs. Bare `/connect` opens a list of the providers that
  offer one; the answer submits `/connect <provider>`. The model flow asks
  nothing new. A zero-attempt error — Aldwin could not send at all — now
  leads its failure row with its own message instead of "The provider
  kept failing".
- [x] **baseline.json.** `/connect` joins the command-list entry; a new
  entry records that the list is a QuestionPanel and the sign-in is
  notices, since the design has no connection component.
- [ ] **Live run** on a subscription. Watch the token response's
  `expires_in`, the 403 some SuperGrok tiers get after a successful
  sign-in, and whether a stale token draws the 401 the retry is built for.
- [ ] **Review scenes** for the list, the sentence and the notice.

## Progress

**2026-09-26 — three-pass audit of the crate (correctness, ethos,
quality), with a blind reviewer that had not seen the design.** Nine
reachable defects found and fixed, each with the test that would have
caught it; 29 tests now, from 20.

- **No request timeout** on either client. A connection the far side
  never answered held a wait past its deadline, and held a session's lock
  — and with it every request in the app — for good. Both clients now
  bound each request to 30 s (`oauth::REQUEST_TIMEOUT`). The bound is a
  test knob because tokio's paused clock jumps to any pending timer while
  a socket is still connecting; the paused tests run without it and the
  two stall tests run on the real clock with it cut to 200 ms.
- **A request cancelled mid-refresh lost the rotation.** The developer
  stopping a turn dropped the future holding the refresh; the server had
  already rotated, and the next request refreshed on a token that was
  gone, logging them out for nothing. The refresh now runs as its own
  task with the lock travelling in it, so it lands in memory and on disk
  whether or not its caller is still there.
- **A bare 401 or 403 read as a logout.** A page from whatever fronts
  the server — a bot check — would have sent the developer to log in
  again in a loop. Revoked now means a refusal in the protocol's own
  shape (`invalid_grant`, or a 401/403 whose body is an OAuth error);
  anything else is trouble the next request retries.
- **One failed poll ended a half-hour wait.** A dropped connection, a
  hung one or a 502 mid-wait is now waited out; only an answer the
  protocol names ends the login. The deadline still bounds it.
- **A logged-out session kept asking**, and a 500 to that doomed refresh
  would have turned `LoggedOut` into `Failed`, hiding the one sentence
  that helps. The credentials are dropped on `Revoked`; every later call
  is `LoggedOut` without a request.
- **`expires_in` was required**; the RFC recommends it. A token granted
  without one now lives an hour, then refreshes.
- **A lifetime too long for the clock panicked the wait** through
  `Instant + Duration`. `checked_add`; the server's own `expired_token`
  is the deadline then.
- **An interval of zero polled flat out.** Floored at one second.
- **The bearer header was not marked sensitive**; a `HeaderMap` printed
  anywhere would have shown the token. `set_sensitive(true)`, tested.

Also: `Session::invalidate()` for the inference endpoint's 401 (the
wiring step above names its one use); redirects refused on both clients,
since a redirected POST carries the refresh token; a transport failure's
message now carries reqwest's source chain, which is where "timed out"
lives; the fake server records `METHOD /path body`, so every grant test
asserts the endpoint it hit; and the staleness test no longer straddles a
second boundary.

Considered and left: `persist` runs synchronously under the lock, as
every config write in the app does; the fixed 30 s bound is not a
connect timeout of its own, since one bound covers both.

**2026-09-26 — wired, then revised.** The first wiring shipped `/login`,
a `login:` field in a version 2 `provider.yaml`, and a key-or-account
question between the provider and the model. The developer's direction,
the same day, was a `/connect` command with a list of connections, and
the decision made at model selection instead: account, then key, then an
error when the model is used. The revision removed the version bump, the
field, the question and `Event::LoggedIn`, and renamed `logins.yaml` to
`connections.yaml`; nothing had shipped, so nothing migrates. See the
Wiring section for what each crate has now.

**2026-09-26 — second three-pass audit, over the whole `/connect` diff.**
My own pass found two defects and a blind reviewer ten; every one was
reachable, and each fix landed with the test that would have caught it.

- **`/quit` hung while a `/connect` waited.** The wait held a sender of
  the TUI's event channel, and dropping a `JoinHandle` detaches rather
  than aborts. `Session` now aborts the wait on drop.
- **An approval could move the session behind the developer's back.** The
  wait rebuilt the client from the file, so a `/model` during the wait, or
  an edit picked up by `/reload-config`, was overridden while the status
  line said otherwise. The whole sign-in now runs in the task; approval
  comes back to the interceptor over a channel, and `moved_onto` rebuilds
  only when the session runs that provider's model as the file states it.
  The interceptor no longer blocks on the device-code request either.
- **A revoked account stuck.** The session dropped the tokens in memory
  only, so every start chose the dead account over an exported key. The
  persist hook now takes `Option<&Credentials>`, `None` on a revocation,
  and the cli removes the entry (`Config::remove_connection`).
- **A deleted entry was written back** by the next refresh. The hook now
  skips an entry that is no longer there. Two processes on one account
  still each rotate their own tokens; ADR 0012 records that as left.
- **The sentence gave advice that could not work.** "Export the key and
  pick the model again" — an export never reaches a running process, and
  picking the same model does not rebuild. It says to set the key and
  start Aldwin again.
- **A disconnected account led with the generic 401 row.** It is now a
  zero-attempt error, so the row leads with "Connect it again with
  /connect xai."
- **A provider's text could become the headline** if it began "terminal
  error after 0 attempts:". The zero-attempt rule now applies to the whole
  error only (`provider_sentence` for the rest).
- **The 401 resend ignored the retry budget**, sending a fifth request.
  It is an attempt like any other.
- **Smaller:** a test whose second half passed for the wrong reason
  (dropped; `connect::reach`'s test covers the order), "0 minutes" for a
  short code (`minutes`, rounded up), "the login failed:" doubled into
  every notice, and a leftover `login:` in a doc comment.

Considered and left: the retry row names the provider rather than x.ai's
account server when a refresh fails; `persist` writes the file on a
runtime thread, as every config write does.

## What a second account adds

The seams were checked against ChatGPT's login, read from the Codex
source, before the crate was shaped:

- OpenAI's device flow is not RFC 8628 — its own usercode and token
  endpoints, a server-generated PKCE pair — so it is a second module behind
  the same `Login` and `Session`, and a variant of `Account`.
- Its requests carry `chatgpt-account-id`, read from a claim in the id
  token. That is why `Session::headers()` yields headers, not a bare token.
- Its inference is the Responses API on chatgpt.com, not chat completions:
  a `Dialect` in aldwin-llm and a catalogue row, not this crate's concern.

Nothing in config, cli or tui changes for it. OpenAI has moved against
third-party use of subscription tokens before; that account's policy
standing needs checking on the day, separately from xAI's.
