//! Stages 1, 2, 3 and 5 — the ones that are somebody else's command.
//!
//! Each returns the same shape so the loop's report reads uniformly, and each
//! runs the tool the developer would run by hand. Nothing is reimplemented
//! here: `cargo` is the authority on whether the workspace is clean, and a
//! second opinion about that would be a second thing to keep in sync.

use std::path::Path;
use std::process::Command;

use crate::Result;

/// One stage's verdict, as the report shows it.
#[derive(Debug)]
pub struct Outcome {
    /// The stage's number and name, as the report's first column shows it.
    pub stage: &'static str,
    /// Whether the tool succeeded.
    pub passed: bool,
    /// What the developer should read: the stage's stat line when it
    /// passed, the tail of the tool's output when it failed.
    pub detail: String,
}

impl Outcome {
    /// `stat` is what the stage reports when it passes — a count the tool
    /// itself produced, not one recomputed here.
    fn from(
        stage: &'static str,
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
        // Both streams: cargo puts diagnostics on stderr and test failures on
        // stdout, and a stage that showed only one of them would report a
        // failing suite as an empty error.
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
/// The loop inherits the developer's shell, and a shell that exported
/// `UPDATE_SNAPSHOTS=1` for one deliberate regeneration would otherwise have
/// stages 3 and 5 rewrite the baseline they exist to check, and report the
/// rewrite as a pass.
fn cargo(root: &Path, args: &[&str]) -> Command {
    let mut command = Command::new("cargo");
    command
        .current_dir(root)
        .args(args)
        .env_remove("UPDATE_SNAPSHOTS");
    command
}

/// Stage 2 — formatting and static analysis.
///
/// `cargo fmt --check` first, because it is the cheaper of the two. The
/// workspace is formatted by stable rustfmt with no options, so its verdict
/// is the one `cargo fmt` gives the developer.
///
/// Clippy with `--all-targets` so tests are linted too; a clippy warning that
/// only fires in a test module is still a warning the next reader has to read
/// past. `-D warnings` because a warning nobody fails on is a warning nobody
/// fixes.
///
/// # Errors
///
/// When `cargo` cannot be run at all. A formatting or lint failure is a
/// failed [`Outcome`], not an error.
pub fn lint(root: &Path) -> Result<Vec<Outcome>> {
    let fmt = Outcome::from(
        "2 lint · fmt",
        cargo(root, &["fmt", "--all", "--check"]).output()?,
        40,
        |_| "formatted".to_string(),
    );
    let clippy = Outcome::from(
        "2 lint · clippy",
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
            // Cargo prints a `Checking` line per target it actually builds,
            // so a warm cache reports none. "0 crate targets" would read as
            // a lint that checked nothing, which is the opposite of what a
            // cached pass means.
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

/// Stage 3 — the suite.
///
/// # Errors
///
/// When `cargo` cannot be run at all. A failing test is a failed
/// [`Outcome`], not an error.
pub fn test(root: &Path) -> Result<Vec<Outcome>> {
    Ok(vec![Outcome::from(
        "3 test",
        cargo(root, &["test", "--workspace"]).output()?,
        60,
        |out| {
            let (passed, _, ignored) = crate::report::test_counts(out);
            format!("{passed} passed, {ignored} ignored")
        },
    )])
}

/// Stage 5 — the rendered frames.
///
/// `crates/tui/tests/render_snapshot.rs`, which does two jobs against
/// `TestBackend` buffers for sixteen scenes at three sizes in both themes:
///
/// * **the baseline** — every cell's symbol, foreground, background and
///   modifiers, serialised and diffed against `tests/snapshots/render.snap`;
/// * **design conformance** — every colour is one of the nineteen roles
///   `tokens.rs` carries (or a mark or gauge mix of two of them), every glyph
///   is from the closed table, nothing is stroked, the agent's prose is never
///   blue and nothing outside a diff is red.
///
/// Both are hermetic and together take under two seconds. The conformance
/// half ran against a real terminal until 2026-09-20 — a compositor, a
/// subprocess and 2m45s — until it was noticed that a `TestBackend` buffer
/// holds the same declared cells. The terminal now only makes pictures for
/// stage 8, the frames judge.
///
/// This re-runs tests stage 3 already ran. That is deliberate and costs about
/// a second: a failure here names the design rule that broke, where the same
/// failure inside a workspace-wide run is one line among six hundred.
///
/// # Errors
///
/// When `cargo` cannot be run at all. A changed or non-conforming frame is a
/// failed [`Outcome`], not an error.
pub fn frames(root: &Path) -> Result<Vec<Outcome>> {
    let outcome = Outcome::from(
        "5 frames",
        cargo(
            root,
            &["test", "-p", "aldwin-tui", "--test", "render_snapshot"],
        )
        .output()?,
        60,
        |out| {
            let (passed, _, _) = crate::report::test_counts(out);
            format!("{passed} checks over 16 scenes x 3 sizes x 2 themes")
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

/// The toolchain this run measured against.
///
/// Not hermeticity — `rust-toolchain.toml` is read by rustup and this machine
/// installs Rust from pacman, so there is nothing to pin against. What this
/// buys instead is honesty: clippy's lint set and rustc's diagnostics move
/// between releases, so a stage 2 failure on untouched code is a real
/// possibility, and a recorded version turns it from a mystery into a line in
/// the report.
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
            stage: "1 toolchain",
            passed: true,
            detail: found,
        }
    } else {
        Outcome {
            stage:  "1 toolchain",
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

    /// A shell that exported `UPDATE_SNAPSHOTS=1` once would have stage 5
    /// regenerate `render.snap` and then pass against its own output.
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
