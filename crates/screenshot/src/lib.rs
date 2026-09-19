//! Screenshot harness for the Mjolnir TUI — see
//! `.claude/spec/mjolnir-screenshot.md`.
//!
//! It runs the **shipped binary** in a real terminal (`foot`) inside a
//! headless compositor, captures frames at three fixed sizes in both themes,
//! and scores them against the design system. That is a different job from
//! `crates/tui/tests/render_snapshot.rs`, which asserts `App`'s cells through
//! a `TestBackend`: the snapshot proves *unchanged*, this judges *correct*,
//! and only this one sees what a terminal actually does with the app's bytes.
//!
//! Dev-only. Nothing in the shipped binary may depend on it.

pub mod baseline;
pub mod capture;
pub mod compositor;
pub mod design;
pub mod fake;
pub mod gates;
pub mod geometry;
pub mod keys;
pub mod png;
pub mod proxy;
pub mod pty;
pub mod regions;
pub mod report;
pub mod regression;
pub mod scene;
pub mod session;
pub mod vt;

pub use baseline::Baseline;
pub use compositor::Compositor;
pub use geometry::{Cell, Size, Theme};
