//! The lock of ADR 0009. Reads and runs need no grant — the sandbox is what
//! holds a read declaration to its word, and the review is the one gate on a
//! change reaching disk — so what is left of the permission model is the
//! part no answer at a prompt could ever lift: **a deny is a lock**.
//!
//! A deny names a program and, optionally, a class — `curl`, or `git: write`
//! — in the project's or the global `permissions.yaml`. A call on a locked
//! program is refused outright, the refusal names the file, and nothing
//! narrower can override it. That was ADR 0004 §7, and it is the one clause
//! of that ADR ADR 0009 keeps whole.
//!
//! What this crate no longer holds: allow lists, the standing rung, the
//! eight-row prompt, the session layer that answers persisted into, and the
//! context-file prompt. `permissions.yaml` still *parses* `allow:` and
//! `default:` so a file from the previous model loads, but nothing reads
//! them — see `aldwin_config::PermissionsConfig`.

mod engine;
mod error;

pub use engine::{LockScope, Locks, Outcome};
pub use error::PermissionError;

// Re-exported so callers reason about one vocabulary: these are persistence
// shapes because config owns the file format, but they are permission
// concepts and a caller should not have to know which crate defines them.
pub use aldwin_config::{Class, GrantEntry};
