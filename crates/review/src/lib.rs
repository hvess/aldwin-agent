//! The review loop: `docs/spec/aldwin-review.md` defines its ten stages,
//! `.claude/skills/review/SKILL.md` drives it.
//!
//! This crate runs the deterministic stages 1–5, decides which judges
//! (stages 6–8, subagents owned by the skill) a change needs, writes their
//! verdicts, and keeps the record stage 10 checks. Stages 1–5 answer yes or
//! no with a reason and never score or classify: whether a frame matches the
//! design is stage 8's question.
//!
//! Dev-only: nothing in the shipped binary may depend on this crate.

pub mod baseline;
pub mod capture;
pub mod compositor;
mod error;
pub mod fake;
pub mod gate;
pub mod geometry;
pub mod git;
pub mod judges;
pub mod keys;
pub mod png;
pub mod proxy;
pub mod pty;
pub mod report;
pub mod scene;
pub mod stages;
pub mod tokens;
pub mod vt;

pub use baseline::Baseline;
pub use compositor::Compositor;
pub use error::{Error, Result};
pub use geometry::{Cell, Size, Theme};
