//! The review loop — see `.claude/skills/review/SKILL.md`.
//!
//! The loop runs before an agent's commit, and the commit cannot land
//! without it. Stages 1 to 5 are deterministic and live here; stages 6 to 8
//! are subagents and live in the skill, because no script can do them — this
//! crate decides which of them a change needs, writes their verdicts, and
//! keeps the record stage 10 checks.
//!
//! | stage | what it answers |
//! | --- | --- |
//! | 1 toolchain | are these results comparable to the last run's |
//! | 2 lint | is it formatted, and does it build clean and pass static analysis — the `rust` skill's checkable rules included |
//! | 3 test | does the suite still pass |
//! | 4 tokens | is the app's design system still the imported one |
//! | 5 frames | do the rendered frames still match the baseline, and does every cell come from the design |
//! | 6 code judge | does the diff hold to `quality-gate`, the Key Constraints and the ADRs |
//! | 7 Rust judge | does the diff hold to the `rust` skill's rules no lint checks |
//! | 8 frames judge | do the changed scenes look like the design |
//! | 9 iterate | fix, and run from stage 1 again — five passes at most |
//! | 10 gate | is there a passing record for exactly the tree being committed |
//!
//! Stages 1 to 5 answer yes or no and say why. None of them scores, ranks or
//! classifies: an earlier version of this harness tried to mechanise "does
//! this match the design" and grew a thousand lines of tables encoding one
//! reading of an ambiguous reference. That question is stage 8's, and stage 8
//! is a model looking at pictures.
//!
//! Dev-only. Nothing in the shipped binary may depend on this crate.

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
