//! Stages 1, 2 and 4 — the ones that are somebody else's command.
//!
//! Each returns the same shape so the loop's report reads uniformly, and each
//! runs the tool the developer would run by hand. Nothing is reimplemented
//! here: `cargo` is the authority on whether the workspace is clean, and a
//! second opinion about that would be a second thing to keep in sync.

use std::io::Result;
use std::path::Path;
use std::process::Command;

pub struct Outcome {
    pub stage:  &'static str,
    pub passed: bool,
    /// What the developer should read. Empty when the stage passed.
    pub detail: String,
}

impl Outcome {
    fn from(stage: &'static str, output: std::process::Output, keep: usize) -> Self {
        if output.status.success() {
            return Outcome { stage, passed: true, detail: String::new() };
        }
        // Both streams: cargo puts diagnostics on stderr and test failures on
        // stdout, and a stage that showed only one of them would report a
        // failing suite as an empty error.
        let mut text = String::from_utf8_lossy(&output.stderr).into_owned();
        text.push_str(&String::from_utf8_lossy(&output.stdout));
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        let tail = lines.iter().rev().take(keep).rev().copied().collect::<Vec<_>>().join("\n");
        Outcome { stage, passed: false, detail: tail }
    }
}

fn cargo(root: &Path, args: &[&str]) -> Result<std::process::Output> {
    Command::new("cargo").current_dir(root).args(args).output()
}

/// Stage 1 — static analysis.
///
/// `--all-targets` so tests are linted too; a clippy warning that only fires
/// in a test module is still a warning the next reader has to read past.
/// `-D warnings` because a warning nobody fails on is a warning nobody fixes.
///
/// # Why `cargo fmt --check` is not here
///
/// It was, and it fails on 500 files. This codebase is written with aligned
/// struct fields and grouped imports, which need `struct_field_align_threshold`
/// and `group_imports` — both nightly-only rustfmt options — and the
/// workspace pins no nightly toolchain. Under stable rustfmt the check
/// demands a reformat that would flatten the alignment the code is
/// deliberately written in, which is a large unrelated diff to buy a
/// consistency the codebase already has by hand.
///
/// Worth revisiting when either option stabilises or the project pins a
/// nightly; until then clippy is the static analysis that carries this
/// stage, and it is clean workspace-wide.
pub fn lint(root: &Path) -> Result<Vec<Outcome>> {
    Ok(vec![Outcome::from(
        "1 lint · clippy",
        cargo(root, &["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"])?,
        40,
    )])
}

/// Stage 2 — the suite.
pub fn test(root: &Path) -> Result<Vec<Outcome>> {
    Ok(vec![Outcome::from("2 test", cargo(root, &["test", "--workspace"])?, 60)])
}

/// Stage 4 — the rendered frames, against the committed baseline.
///
/// This is `crates/tui/tests/render_snapshot.rs`, which serialises every
/// cell's symbol, foreground, background and modifiers for every scene at
/// every size in both themes. It runs in-process against a `TestBackend` in
/// under a second, and it is the *only* baseline: capturing the same frames
/// through a real terminal and diffing those too would be a second fixture
/// asserting the same thing, on a slower clock.
///
/// What the real terminal is for is stage 5's pictures — see
/// [`crate::capture`]. It sees one thing this cannot, which is what foot
/// actually does with the app's bytes, and that is worth a picture rather
/// than a second baseline.
pub fn screenshots(root: &Path) -> Result<Vec<Outcome>> {
    let outcome = Outcome::from(
        "4 screenshots",
        cargo(root, &["test", "-p", "mjolnir-tui", "--test", "render_snapshot"])?,
        60,
    );
    Ok(vec![if outcome.passed {
        outcome
    } else {
        Outcome {
            detail: format!(
                "{}\n\nIf the change is meant to alter these frames, regenerate deliberately after reading the diff:\n    UPDATE_SNAPSHOTS=1 cargo test -p mjolnir-tui --test render_snapshot",
                outcome.detail
            ),
            ..outcome
        }
    }])
}
