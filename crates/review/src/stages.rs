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
    /// `stat` is what the stage reports when it passes — a count the tool
    /// itself produced, not one recomputed here.
    fn from(stage: &'static str, output: std::process::Output, keep: usize, stat: impl Fn(&str) -> String) -> Self {
        let combined = format!("{}{}", String::from_utf8_lossy(&output.stderr), String::from_utf8_lossy(&output.stdout));
        if output.status.success() {
            return Outcome { stage, passed: true, detail: stat(&combined) };
        }
        // Both streams: cargo puts diagnostics on stderr and test failures on
        // stdout, and a stage that showed only one of them would report a
        // failing suite as an empty error.
        let lines: Vec<&str> = combined.lines().filter(|l| !l.trim().is_empty()).collect();
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
        |out| {
            let crates = out.lines().filter(|l| l.trim_start().starts_with("Checking ")).count();
            format!("clippy clean across {crates} crate targets")
        },
    )])
}

/// Stage 2 — the suite.
pub fn test(root: &Path) -> Result<Vec<Outcome>> {
    Ok(vec![Outcome::from("2 test", cargo(root, &["test", "--workspace"])?, 60, |out| {
        let (passed, _, ignored) = crate::report::test_counts(out);
        format!("{passed} passed, {ignored} ignored")
    })])
}

/// Stage 4 — the rendered frames.
///
/// `crates/tui/tests/render_snapshot.rs`, which does two jobs against
/// `TestBackend` buffers for twelve scenes at three sizes in both themes:
///
/// * **the baseline** — every cell's symbol, foreground, background and
///   modifiers, serialised and diffed against `tests/snapshots/render.snap`;
/// * **design conformance** — every colour is one of the forty-two roles
///   `tokens.rs` carries (or one dimmed toward a ground), every glyph is from
///   the closed table, and the app's own copy is third person with no
///   contractions.
///
/// Both are hermetic and together take under two seconds. The conformance
/// half ran against a real terminal until 2026-09-20 — a compositor, a
/// subprocess and 2m45s — until it was noticed that a `TestBackend` buffer
/// holds the same declared cells. The terminal now only makes pictures for
/// stage 5.
///
/// This re-runs tests stage 2 already ran. That is deliberate and costs about
/// a second: a failure here names the design rule that broke, where the same
/// failure inside a workspace-wide run is one line among six hundred.
pub fn frames(root: &Path) -> Result<Vec<Outcome>> {
    let outcome = Outcome::from(
        "4 frames",
        cargo(root, &["test", "-p", "mjolnir-tui", "--test", "render_snapshot"])?,
        60,
        |out| {
            let (passed, _, _) = crate::report::test_counts(out);
            format!("{passed} checks over 12 scenes x 3 sizes x 2 themes")
        },
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

/// The toolchain this run measured against.
///
/// Not hermeticity — `rust-toolchain.toml` is read by rustup and this machine
/// installs Rust from pacman, so there is nothing to pin against. What this
/// buys instead is honesty: clippy's lint set and rustc's diagnostics move
/// between releases, so a stage 1 failure on untouched code is a real
/// possibility, and a recorded version turns it from a mystery into a line in
/// the report.
pub fn toolchain(root: &Path, expected: &str) -> Result<Vec<Outcome>> {
    let output = Command::new("rustc").current_dir(root).arg("--version").output()?;
    let found = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(vec![if found == expected {
        Outcome { stage: "0 toolchain", passed: true, detail: found }
    } else {
        Outcome {
            stage:  "0 toolchain",
            passed: false,
            detail: format!(
                "this run is on {found:?}, the baseline records {expected:?}.\nLint results are not comparable across toolchains. If the upgrade is intended, record it:\n    cargo run -p mjolnir-review -- measure --record-toolchain"
            ),
        }
    }])
}
