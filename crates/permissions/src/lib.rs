//! The permission engine of ADR 0004. Nothing runs that a rule, or an answer
//! the developer gave, has not allowed.
//!
//! A grant is a **program and a class** — `git: read` — and the class belongs
//! to the *call*, not the program: `git status` is a read and `git push` is a
//! write, and they are the same binary. The agent declares a class per call;
//! this crate does not verify the declaration and deliberately cannot. That is
//! the sandbox's job at execution time (mjolnir-tools), and the division is
//! the whole design: a policy engine that also judged what a command does
//! would be guessing, and a wrong guess there runs the command.
//!
//! `Engine` owns precedence — deny as a lock across every scope, then allow,
//! then the standing rung with the narrower file winning — and the in-memory
//! session layer. It has no dependency on mjolnir-core: `PromptPayload` /
//! `PromptResponse` are the plain-data shapes that cross core's opaque
//! `serde_json::Value` boundary.

mod engine;
mod error;
mod prompt;

pub use engine::{
    EffectiveContextFile, EffectiveGrant, EffectiveView, Engine, GrantScope, Outcome,
};
pub use error::PermissionError;
pub use prompt::{Choice, ContextFileTier, PromptPayload, PromptResponse};

// Re-exported so callers reason about one vocabulary: these are persistence
// shapes because config owns the file format, but they are permission
// concepts and a caller should not have to know which crate defines them.
pub use mjolnir_config::{Class, GrantEntry, GrantList, Rung};
