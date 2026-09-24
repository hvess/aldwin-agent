//! The review loop — see `.claude/skills/review/SKILL.md`.
//!
//! The loop runs after a change is ready for submission. Stages 0–4 are
//! deterministic and live here; stage 5 is a subagent and lives in the
//! skill, because no script can do it — this crate only writes its result.
//!
//! | stage | what it answers |
//! | --- | --- |
//! | 0 toolchain | are these results comparable to the last run's |
//! | 1 lint | is it formatted, and does it build clean and pass static analysis |
//! | 2 test | does the suite still pass |
//! | 3 tokens | is the app's design system still the imported one |
//! | 4 frames | do the rendered frames still match the baseline, and does every cell come from the design |
//! | 5 confidence | does the change look like the design, and did it do what it set out to do |
//!
//! Stages 0–4 answer yes or no and say why. None of them scores, ranks or
//! classifies: an earlier version of this harness tried to mechanise "does
//! this match the design" and grew a thousand lines of tables encoding one
//! reading of an ambiguous reference. That question is stage 5's, and stage 5
//! is a model looking at pictures.
//!
//! Dev-only. Nothing in the shipped binary may depend on this crate.

pub mod baseline;
pub mod capture;
pub mod compositor;
pub mod fake;
pub mod geometry;
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
pub use geometry::{Cell, Size, Theme};
