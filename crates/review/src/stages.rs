//! Stages 1 to 5 ([`Stage`]): 1, 2, 3 and 5 run `rustc` or `cargo`, 4 is
//! this crate's token check.
//!
//! Never reimplement what `cargo` checks: it is the authority on whether the
//! workspace is clean.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use crate::{tokens, Baseline, Result};

/// A deterministic stage, stage 2 split into its two tools. Which judges may
/// run is decided on these variants, never on [`Stage::label`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Stage 1: the toolchain against the baseline.
    Toolchain,
    /// Stage 2's formatting half.
    Fmt,
    /// Stage 2's clippy half: the one that shows the workspace builds.
    Clippy,
    /// Stage 3: the suite.
    Test,
    /// Stage 4: the design tokens.
    Tokens,
    /// Stage 5: the frame snapshots.
    Frames,
}

impl Stage {
    /// The stage's number and name, as the report's first column shows it.
    ///
    /// # Examples
    ///
    /// ```
    /// use aldwin_review::stages::Stage;
    /// assert_eq!(Stage::Clippy.label(), "2 lint · clippy");
    /// ```
    pub fn label(self) -> &'static str {
        match self {
            Stage::Toolchain => "1 toolchain",
            Stage::Fmt => "2 lint · fmt",
            Stage::Clippy => "2 lint · clippy",
            Stage::Test => "3 test",
            Stage::Tokens => "4 tokens",
            Stage::Frames => "5 frames",
        }
    }
}

/// One stage's verdict, as the report shows it.
#[derive(Debug)]
pub struct Outcome {
    /// Which stage.
    pub stage: Stage,
    /// Whether the tool succeeded.
    pub passed: bool,
    /// The stage's stat line when it passed, the tail of the tool's output
    /// when it failed.
    pub detail: String,
}

impl Outcome {
    /// `stat` makes the pass line from the tool's own output; `keep` is how
    /// many lines of a failure's tail to show.
    fn from(
        stage: Stage,
        output: std::process::Output,
        keep: usize,
        stat: impl Fn(&str) -> String,
    ) -> Self {
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
        if output.status.success() {
            return Outcome {
                stage,
                passed: true,
                detail: stat(&combined),
            };
        }
        // Both streams: cargo puts diagnostics on stderr, test failures on
        // stdout.
        let lines: Vec<&str> = combined.lines().filter(|l| !l.trim().is_empty()).collect();
        let tail = lines
            .iter()
            .rev()
            .take(keep)
            .rev()
            .copied()
            .collect::<Vec<_>>()
            .join("\n");
        Outcome {
            stage,
            passed: false,
            detail: tail,
        }
    }
}

/// `cargo` in the workspace, with `UPDATE_SNAPSHOTS` removed.
///
/// An inherited `UPDATE_SNAPSHOTS=1` would have stages 3 and 5 rewrite the
/// baseline they check and report the rewrite as a pass.
fn cargo(root: &Path, args: &[&str]) -> Command {
    let mut command = Command::new("cargo");
    command
        .current_dir(root)
        .args(args)
        .env_remove("UPDATE_SNAPSHOTS");
    command
}

/// Stage 2: `cargo fmt --all --check`, then clippy over all targets, tests
/// included, with `-D warnings`.
///
/// # Errors
///
/// When `cargo` cannot be run at all. A formatting or lint failure is a
/// failed [`Outcome`], not an error.
pub fn lint(root: &Path) -> Result<Vec<Outcome>> {
    let fmt = Outcome::from(
        Stage::Fmt,
        cargo(root, &["fmt", "--all", "--check"]).output()?,
        40,
        |_| "formatted".to_string(),
    );
    let clippy = Outcome::from(
        Stage::Clippy,
        cargo(
            root,
            &[
                "clippy",
                "--workspace",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ],
        )
        .output()?,
        40,
        |out| {
            // Cargo prints a `Checking` line only per target it rebuilds, so
            // a warm cache reports none.
            match out
                .lines()
                .filter(|l| l.trim_start().starts_with("Checking "))
                .count()
            {
                0 => "clippy clean (cached)".to_string(),
                n => format!("clippy clean across {n} crate targets"),
            }
        },
    );
    Ok(vec![fmt, clippy])
}

/// Stage 3: `cargo test --workspace`.
///
/// # Errors
///
/// When `cargo` cannot be run at all. A failing test is a failed
/// [`Outcome`], not an error.
pub fn test(root: &Path) -> Result<Vec<Outcome>> {
    Ok(vec![Outcome::from(
        Stage::Test,
        cargo(root, &["test", "--workspace"]).output()?,
        60,
        |out| {
            let (passed, _, ignored) = crate::report::test_counts(out);
            format!("{passed} passed, {ignored} ignored")
        },
    )])
}

/// Stage 5: `crates/tui/tests/render_snapshot.rs`, hermetic over
/// `TestBackend` buffers (Decisions 1 and 8): every scene's cells diffed
/// against `render.snap`, and design conformance (palette roles, the closed
/// glyph table, no strokes, prose never blue, red only in diffs).
///
/// Deliberately re-runs tests stage 3 ran, so a failure is reported as its
/// own stage.
///
/// # Errors
///
/// When `cargo` cannot be run at all. A changed or non-conforming frame is a
/// failed [`Outcome`], not an error.
pub fn frames(root: &Path) -> Result<Vec<Outcome>> {
    let outcome = Outcome::from(
        Stage::Frames,
        cargo(
            root,
            &["test", "-p", "aldwin-tui", "--test", "render_snapshot"],
        )
        .output()?,
        60,
        |out| {
            let (passed, _, _) = crate::report::test_counts(out);
            // Counted from the snapshot: a written-down count goes stale.
            let scenes = std::fs::read_to_string(root.join(crate::git::SNAPSHOT))
                .map(|text| snapshot_scenes(&text))
                .unwrap_or_default();
            format!("{passed} checks over {scenes} scenes x 3 sizes x 2 themes")
        },
    );
    Ok(vec![if outcome.passed {
        outcome
    } else {
        Outcome {
            detail: format!(
                "{}\n\nIf the change is meant to alter these frames, regenerate deliberately after reading the diff:\n    UPDATE_SNAPSHOTS=1 cargo test -p aldwin-tui --test render_snapshot",
                outcome.detail
            ),
            ..outcome
        }
    }])
}

/// How many scenes a snapshot pins: distinct names in its `=== <theme>
/// <scene> <size>` headers.
fn snapshot_scenes(text: &str) -> usize {
    text.lines()
        .filter_map(crate::git::scene_of)
        .collect::<BTreeSet<_>>()
        .len()
}

/// Stage 4: `crates/tui/src/tokens.rs` regenerated from the design and
/// compared with the committed one.
///
/// # Errors
///
/// When the design cannot be read at all. A drifted file is a failed
/// [`Outcome`], not an error.
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
/// let base = aldwin_review::Baseline::load()?;
/// let outcomes = aldwin_review::stages::tokens(Path::new("."), &base)?;
/// assert!(outcomes.iter().all(|o| o.passed));
/// # Ok::<(), aldwin_review::Error>(())
/// ```
pub fn tokens(root: &Path, base: &Baseline) -> Result<Vec<Outcome>> {
    Ok(vec![
        match tokens::check(root, &tokens::design_dir(), base)? {
            Ok(n) => Outcome {
                stage: Stage::Tokens,
                passed: true,
                detail: format!("{n} values from the design"),
            },
            Err(detail) => Outcome {
                stage: Stage::Tokens,
                passed: false,
                detail: format!(
                "{detail}\n\nRegenerate with:\n    cargo run -p aldwin-review -- tokens --write"
            ),
            },
        },
    ])
}

/// Stage 1: `rustc --version` against the baseline's recorded toolchain.
///
/// Recorded, not pinned (Decision 10): Rust comes from pacman, not rustup.
///
/// # Errors
///
/// When `rustc` cannot be run. A version other than `expected` is a failed
/// [`Outcome`], not an error.
pub fn toolchain(root: &Path, expected: &str) -> Result<Vec<Outcome>> {
    let output = Command::new("rustc")
        .current_dir(root)
        .arg("--version")
        .output()?;
    let found = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(vec![if found == expected {
        Outcome {
            stage: Stage::Toolchain,
            passed: true,
            detail: found,
        }
    } else {
        Outcome {
            stage: Stage::Toolchain,
            passed: false,
            detail: format!(
                "this run is on {found:?}, the baseline records {expected:?}.\nLint results are not comparable across toolchains. If the upgrade is intended, record it in the `toolchain` field of crates/review/baseline.json."
            ),
        }
    }])
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;

    #[test]
    fn scenes_are_counted_once_across_sizes_and_themes() {
        let snap =
            "=== Dark launch 80x24\nrow\n=== Light launch 80x24\nrow\n=== Dark plan 104x32\nrow\n";
        assert_eq!(snapshot_scenes(snap), 2);
    }

    #[test]
    fn cargo_never_inherits_the_snapshot_regeneration_switch() {
        let command = cargo(Path::new("."), &["test"]);
        assert!(
            command
                .get_envs()
                .any(|(key, value)| key == OsStr::new("UPDATE_SNAPSHOTS") && value.is_none()),
            "every cargo the loop runs must have UPDATE_SNAPSHOTS removed"
        );
    }
}
