//! `aldwin-review` — the deterministic stages of the review loop.
//!
//! `review` runs stages 1 to 4 and prints what failed. Stage 5 is a subagent
//! and belongs to the skill; this binary's job ends at handing it a directory
//! of frames.
//!
//! The other commands exist because the loop needs them: `tokens` regenerates
//! the design system into the app, `capture` takes the frames stage 5 looks
//! at, and `measure` pins the cell this machine renders.

use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::{Parser, Subcommand};
use aldwin_review::capture::{capture, measure_cell};
use aldwin_review::geometry::{Size, Theme};
use aldwin_review::{scene, stages, tokens, Baseline, Compositor};

#[derive(Parser)]
#[command(about = "The review loop's deterministic stages")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run stages 0–4 and write the report. Stage 5 is the skill's.
    Review {
        /// What this change set out to do, in a sentence. Required: a review
        /// that cannot say what it is reviewing cannot judge whether the
        /// change did it, and stage 5 is handed this verbatim.
        #[arg(long)]
        goal: String,
        /// The scenes the change touched, comma-separated and validated
        /// against the catalogue. Stage 5 judges these and ignores the rest
        /// — without it a judge reports the whole app's backlog instead of
        /// this change.
        ///
        /// Scene names, not prose: the run prints the exact frame paths for
        /// them, so the judge is handed a list rather than a directory to
        /// glob and choose from. Put the prose in `--goal`.
        #[arg(long)]
        focus: String,
        /// Skip the capture. Stages 1–4 do not need it; stage 5 does, and
        /// this is how an iteration pass that only fixed a lint avoids
        /// paying for frames it will not look at.
        #[arg(long)]
        no_capture: bool,
        /// Capture one theme only. The two are a palette swap over one
        /// layout, which `review` proves each time it captures both.
        #[arg(long)]
        theme: Option<Theme>,
        /// How long the app must be silent before a frame is taken. Lower it
        /// for an iteration pass; a frame taken too early is a frame of a
        /// half-drawn app.
        #[arg(long, default_value_t = 400)]
        quiet_ms: u64,
        /// Run the deterministic stages and stop, reporting success on them
        /// alone.
        ///
        /// This is not a review. It is the fast check to run while you are
        /// still working — the full contract requires stage 5, and without
        /// this flag `review` says so by exiting non-zero.
        #[arg(long)]
        stages_only: bool,
    },
    /// Regenerate the app's design system from `.claude/design/tokens/`.
    Tokens {
        /// Write the file rather than only checking it.
        #[arg(long)]
        write: bool,
    },
    /// Capture the frames stage 5 looks at, without running the other stages.
    Capture {
        /// One scene; omit for all of them.
        #[arg(long)]
        scene: Option<String>,
        #[arg(long)]
        size: Option<Size>,
        #[arg(long)]
        theme: Option<Theme>,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long, default_value_t = 400)]
        quiet_ms: u64,
    },
    /// Measure foot's cell for the pinned font and check it against the baseline.
    Measure {
        /// Write the measured cell into the baseline instead of failing.
        #[arg(long)]
        record: bool,
    },
    /// Write stage 5's result into the run's report and print the score.
    ///
    /// A command rather than an instruction to hand-edit HTML: the first
    /// version left the append to the skill's discipline and the section
    /// came back empty on three consecutive runs.
    Stage5 {
        #[arg(long)]
        run: PathBuf,
        /// JSON: `{"iteration": 1, "findings": [{"severity","design","frame","frames"}], "matches": []}`
        #[arg(long)]
        findings: PathBuf,
    },
    /// Print the scene catalogue.
    Scenes,
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("workspace root")
}

/// A fresh directory for this run's frames, keeping the last few.
fn frames_dir(root: &Path) -> std::io::Result<PathBuf> {
    let parent = root.join("target/review-frames");
    std::fs::create_dir_all(&parent)?;
    let mut existing: Vec<PathBuf> =
        std::fs::read_dir(&parent)?.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir()).collect();
    existing.sort();
    for old in existing.iter().take(existing.len().saturating_sub(3)) {
        let _ = std::fs::remove_dir_all(old);
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(std::io::Error::other)?;
    let dir = parent.join(format!("run-{}", now.as_secs()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Builds `target/debug/aldwin` and returns its path.
///
/// **Building it here rather than checking it exists is the whole point.** An
/// existence check cannot tell a current binary from one built several
/// commits ago, and it did not: capture once spawned a stale *debug* binary
/// after a `--release` build, and the judge spent its whole budget on pixels
/// that predated the change, reporting the old copy as a finding. Stage 5 is
/// the one stage that is neither cheap nor reproducible, so that is the most
/// expensive way this loop can fail.
///
/// `cargo build` is incremental, so on an up-to-date tree this is a few
/// hundred milliseconds against a capture measured in minutes.
fn build_app(root: &Path) -> std::io::Result<PathBuf> {
    let out = std::process::Command::new("cargo")
        .current_dir(root)
        .args(["build", "--bin", "aldwin"])
        .output()?;
    if !out.status.success() {
        return Err(std::io::Error::other(format!(
            "the frames capture spawns target/debug/aldwin, and building it failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    let binary = root.join("target/debug/aldwin");
    if !binary.exists() {
        return Err(std::io::Error::other(format!("{} still does not exist after a successful build", binary.display())));
    }
    Ok(binary)
}

/// Capture every scene into a fresh directory, for stage 5 to look at.
///
/// No assertions: everything that reads declared cells is a hermetic test in
/// `crates/tui` now. This exists to make pictures, which is the one thing a
/// `TestBackend` cannot do.
fn capture_all(root: &Path, base: &Baseline, theme: Option<Theme>, quiet_ms: u64) -> std::io::Result<PathBuf> {
    let dir = frames_dir(root)?;
    let binary = build_app(root)?;
    let comp = Compositor::start(&dir)?;
    let cell = measure_cell(&comp, &base.font)?;
    if cell != base.cell {
        return Err(std::io::Error::other(format!(
            "cell {}×{} disagrees with the baseline's {}×{} — run `measure --record` if the font or machine changed",
            cell.w, cell.h, base.cell.w, base.cell.h
        )));
    }
    let themes = theme.map_or_else(|| Theme::ALL.to_vec(), |t| vec![t]);
    for name in scene::IMPLEMENTED {
        for size in Size::ALL {
            for &theme in &themes {
                capture(&comp, &binary, base, cell, name, size, theme, Duration::from_millis(quiet_ms), &[], &dir)?;
            }
        }
    }
    Ok(dir)
}

fn main() -> std::io::Result<()> {
    let cli = Cli::parse();
    let root = workspace_root();
    let base = Baseline::load()?;

    match cli.command {
        Command::Stage5 { run, findings } => {
            let stage5: aldwin_review::report::Stage5 =
                serde_json::from_str(&std::fs::read_to_string(&findings)?).map_err(std::io::Error::other)?;
            let report = run.join("review.html");
            let score = aldwin_review::report::write_stage5(&report, &stage5)?;
            let threshold = aldwin_review::report::THRESHOLD;
            println!("stage 5: {score}, threshold {threshold} — {}", if score >= threshold { "passes" } else { "does not pass" });
            println!("report: {}", report.display());
            if score < threshold {
                return Err(std::io::Error::other("stage 5 is below the threshold; fix and re-run the loop"));
            }
            Ok(())
        }

        Command::Scenes => {
            for name in scene::CATALOGUE {
                let state = if scene::IMPLEMENTED.contains(name) { "ready" } else { "not wired up" };
                println!("{name:<16} {state}");
            }
            Ok(())
        }

        Command::Measure { record } => {
            let dir = frames_dir(&root)?;
            let comp = Compositor::start(&dir)?;
            let cell = measure_cell(&comp, &base.font)?;
            println!("measured cell {}×{} for {}", cell.w, cell.h, base.font);
            if cell != base.cell {
                if !record {
                    // Loudly, by design: a cell that has moved silently
                    // rewrites every frame's geometry while every frame still
                    // looks right.
                    return Err(std::io::Error::other(format!(
                        "cell {}×{} disagrees with the baseline's {}×{} — rerun with --record if the font or machine changed",
                        cell.w, cell.h, base.cell.w, base.cell.h
                    )));
                }
                let mut updated = base;
                updated.cell = cell;
                updated.save()?;
                println!("recorded in {}", Baseline::path().display());
            }
            Ok(())
        }

        Command::Tokens { write } => {
            let design_dir = tokens::design_dir();
            if write {
                let text = tokens::generate(&design_dir)?;
                let path = tokens::output_path(&root);
                std::fs::write(&path, &text)?;
                println!("wrote {}", path.display());
                return Ok(());
            }
            match tokens::check(&root, &design_dir)? {
                Ok(n) => {
                    println!("{} is current — {n} values from the design", tokens::OUTPUT);
                    Ok(())
                }
                Err(detail) => Err(std::io::Error::other(detail)),
            }
        }

        Command::Capture { scene: one, size, theme, out, quiet_ms } => {
            let dir = match out {
                Some(path) => {
                    std::fs::create_dir_all(&path)?;
                    path
                }
                None => frames_dir(&root)?,
            };
            // Same reason as `capture_all`: never spawn a binary nobody
            // just built.
            let binary = build_app(&root)?;
            let comp = Compositor::start(&dir)?;
            let cell = measure_cell(&comp, &base.font)?;
            let names: Vec<&str> = match &one {
                Some(name) => vec![name.as_str()],
                None => scene::IMPLEMENTED.to_vec(),
            };
            let sizes = size.map_or_else(|| Size::ALL.to_vec(), |s| vec![s]);
            let themes = theme.map_or_else(|| Theme::ALL.to_vec(), |t| vec![t]);
            for name in names {
                for &s in &sizes {
                    for &t in &themes {
                        let frame = capture(&comp, &binary, &base, cell, name, s, t, Duration::from_millis(quiet_ms), &[], &dir)?;
                        println!("{name} {s} {t}  {}", frame.path.display());
                    }
                }
            }
            println!("\nframes {}", dir.display());
            Ok(())
        }

        Command::Review { goal, focus, no_capture, theme, quiet_ms, stages_only } => {
            let focused: Vec<&str> = focus.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
            let unknown: Vec<&&str> = focused.iter().filter(|s| !scene::IMPLEMENTED.contains(s)).collect();
            if focused.is_empty() || !unknown.is_empty() {
                return Err(std::io::Error::other(format!(
                    "--focus must be comma-separated scene names; {} is not one. Known: {}",
                    if unknown.is_empty() { "(nothing)".to_string() } else { format!("{unknown:?}") },
                    scene::IMPLEMENTED.join(", ")
                )));
            }

            let mut outcomes = stages::toolchain(&root, &base.toolchain)?;
            outcomes.extend(stages::lint(&root)?);
            outcomes.extend(stages::test(&root)?);

            // Stage 3 before stage 4, because if the generated palette has
            // drifted then every frame below was drawn with the wrong
            // colours and reporting those would be reporting a consequence
            // as a cause.
            let design_dir = tokens::design_dir();
            outcomes.push(match tokens::check(&root, &design_dir)? {
                Ok(n) => stages::Outcome { stage: "3 tokens", passed: true, detail: format!("{n} values from the design") },
                Err(detail) => stages::Outcome {
                    stage:  "3 tokens",
                    passed: false,
                    detail: format!("{detail}\n\nRegenerate with:\n    cargo run -p aldwin-review -- tokens --write"),
                },
            });
            outcomes.extend(stages::frames(&root)?);

            let frames = match no_capture {
                true => None,
                false => Some(capture_all(&root, &base, theme, quiet_ms)?),
            };
            let captured = frames
                .as_ref()
                .map(|dir| std::fs::read_dir(dir).map(|e| e.filter_map(|e| e.ok()).filter(|e| e.path().extension().is_some_and(|x| x == "png")).count()).unwrap_or(0))
                .unwrap_or(0);

            println!();
            let mut failed = 0;
            for outcome in &outcomes {
                if outcome.passed {
                    let detail = if outcome.detail.is_empty() { String::new() } else { format!(" — {}", outcome.detail) };
                    println!("  ok    {}{detail}", outcome.stage);
                } else {
                    failed += 1;
                    println!("  FAIL  {}", outcome.stage);
                    for line in outcome.detail.lines() {
                        println!("          {line}");
                    }
                }
            }

            // The report goes beside the frames when there are any, and into
            // a directory of its own when there are not — a review that
            // skipped capture is still a review worth keeping.
            let commit = std::process::Command::new("git")
                .current_dir(&root)
                .args(["rev-parse", "--short", "HEAD"])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_default();
            let dir = match &frames {
                Some(dir) => dir.clone(),
                None => frames_dir(&root)?,
            };
            let written = aldwin_review::report::write(
                &dir,
                &aldwin_review::report::Run {
                    goal: &goal,
                    focus: &focus,
                    commit: &commit,
                    outcomes: &outcomes,
                    frames: frames.as_deref(),
                    captured,
                },
            )?;

            println!();
            match &frames {
                Some(dir) => {
                    // The exact frames, enumerated. A judge handed a
                    // directory has to decide what to open, and deciding is
                    // a variance source in a stage that has enough of them.
                    println!("frames for stage 5 — the focused scenes only:");
                    for scene_name in &focused {
                        for size in Size::ALL {
                            for theme in Theme::ALL {
                                let stem = format!("{scene_name}-{size}-{theme}");
                                if dir.join(format!("{stem}.png")).exists() {
                                    println!("  {}", dir.join(format!("{stem}.png")).display());
                                }
                            }
                        }
                    }
                    println!("\nEach has a .txt beside it: the declared grid, every character at its\nexact column. Use it for geometry and the .png only for colour.");
                }
                None => println!("no frames captured (--no-capture); stage 5 needs them"),
            }
            println!("\nreport: {}", written.display());
            // The exact next command, with the directory filled in. Stage 5's
            // section was left empty on three separate runs because writing it
            // depended on the operator recalling a command rather than copying
            // one, and the moment it is needed is the moment attention is on
            // the findings instead.
            println!("\nStage 5 is not written yet. After the judge, run:");
            println!("  ./target/release/aldwin-review stage5 --run {} --findings <file.json>", dir.display());
            if failed > 0 {
                return Err(std::io::Error::other(format!("{failed} of {} deterministic stages failed", outcomes.len())));
            }
            if stages_only {
                println!("stages 0–4 clean. Not a review: stage 5 was not run.");
                return Ok(());
            }
            // Deliberately an error. Stage 5's section came back empty on five
            // of seven runs, and the cause was never that the command to write
            // it was missing — it was that this line used to say "clean" and
            // exit zero, which reads as completion. A review without stage 5
            // is not a review, so the only way to exit zero is through
            // `stage5`, the same way the only way past stage 3 is to
            // regenerate the tokens.
            // Printed rather than carried in the error: `fn main`'s `Err` is
            // Debug-formatted, so a multi-line message comes out with its
            // escapes showing.
            println!("stages 0–4 clean — review INCOMPLETE until stage 5 is written.");
            println!("Spawn the judge against those frames, then run the stage5 command above.");
            println!("(Use --stages-only if you wanted the deterministic check alone.)");
            Err(std::io::Error::other("review incomplete: stage 5 not written"))
        }
    }
}
