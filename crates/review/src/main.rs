//! `aldwin-review` — the review loop's commands.
//!
//! `review` runs stages 1 to 5, decides from the staged diff which of the
//! judges in stages 6 to 8 the change needs, and hands them their inputs. The
//! judges are subagents and belong to the skill; `judge` writes what each one
//! found, and when every required stage has passed it records the pass
//! against the staged tree. `gate` is stage 10: the pre-commit hook runs it,
//! and an agent's commit without a passing record for exactly its tree fails.
//!
//! The other commands exist because the loop needs them: `tokens` regenerates
//! the design system into the app, `capture` takes the frames stage 8 looks
//! at, and `measure` pins the cell this machine renders.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aldwin_review::capture::{capture, measure_cell};
use aldwin_review::geometry::{Size, Theme};
use aldwin_review::judges::{self, Assignment, Judge, RunState, Standing};
use aldwin_review::report::{self, Verdict};
use aldwin_review::stages::Outcome;
use aldwin_review::{gate, git, scene, stages, tokens, Baseline, Compositor, Error, Result};
use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(about = "The review loop's deterministic stages")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run stages 1–5 on the staged change, and hand the judges it needs
    /// their inputs.
    ///
    /// Takes no statement of intent. What the change is for is the
    /// developer's to judge; which judges it needs is read off what it
    /// touches, and which frames stage 8 looks at off which scenes' snapshot
    /// it changes — so nothing the author writes is an input to its own
    /// review.
    Review(ReviewArgs),
    /// Regenerate the app's design system from `.claude/design/tokens/`.
    Tokens {
        /// Write the file rather than only checking it.
        #[arg(long)]
        write: bool,
    },
    /// Capture the frames stage 8 looks at, without running the other stages.
    Capture(CaptureArgs),
    /// Measure foot's cell for the pinned font and check it against the baseline.
    Measure {
        /// Write the measured cell into the baseline instead of failing.
        #[arg(long)]
        record: bool,
    },
    /// Write a judge's verdict into the run's report; when it completes the
    /// run, record the pass.
    ///
    /// A command rather than an instruction to hand-edit HTML: the first
    /// version left the append to the skill's discipline and the section
    /// came back empty on three consecutive runs.
    Judge {
        #[arg(long)]
        run: PathBuf,
        /// 6 (code), 7 (Rust) or 8 (frames).
        #[arg(long)]
        stage: u8,
        /// The judge's JSON block, saved verbatim: `{"iteration": 1,
        /// "findings": [{"severity","source","expected","found","at"}],
        /// "matches": [], "contradictions": [], "questions": []}`.
        #[arg(long)]
        findings: PathBuf,
    },
    /// Stage 10: fail unless the tree about to be committed has a passing
    /// review. The pre-commit hook runs this for an agent's commit.
    Gate,
    /// Print the scene catalogue.
    Scenes,
}

#[derive(Args)]
struct ReviewArgs {
    /// Skip the capture even when stage 8 needs it. For an iteration pass
    /// that only fixed a lint; the run cannot pass without the frames.
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
    /// still working, and it is the one mode that does not require the
    /// working tree to be staged — it records nothing a commit can use.
    #[arg(long)]
    stages_only: bool,
}

#[derive(Args)]
struct CaptureArgs {
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
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

/// A fresh directory for this run's frames, keeping the last few.
fn frames_dir(root: &Path) -> Result<PathBuf> {
    let parent = root.join("target/review-frames");
    std::fs::create_dir_all(&parent)?;
    let mut existing: Vec<PathBuf> = std::fs::read_dir(&parent)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    existing.sort();
    for old in existing.iter().take(existing.len().saturating_sub(3)) {
        let _ = std::fs::remove_dir_all(old);
    }
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?;
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
/// that predated the change, reporting the old copy as a finding. Stage 8 is
/// the costliest stage and cannot be reproduced, so that is the most
/// expensive way this loop can fail.
///
/// `cargo build` is incremental, so on an up-to-date tree this is a few
/// hundred milliseconds against a capture measured in minutes.
fn build_app(root: &Path) -> Result<PathBuf> {
    let out = std::process::Command::new("cargo")
        .current_dir(root)
        .args(["build", "--bin", "aldwin"])
        .output()?;
    if !out.status.success() {
        return Err(Error::Build(format!(
            "the frames capture spawns target/debug/aldwin, and building it failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    let binary = root.join("target/debug/aldwin");
    if !binary.exists() {
        return Err(Error::Build(format!(
            "{} still does not exist after a successful build",
            binary.display()
        )));
    }
    Ok(binary)
}

/// Capture `scenes` into `dir`, for stage 8 to look at.
///
/// No assertions: everything that reads declared cells is a hermetic test in
/// `crates/tui` now. This exists to make pictures, which is the one thing a
/// `TestBackend` cannot do.
fn capture_scenes(
    root: &Path,
    base: &Baseline,
    dir: &Path,
    scenes: &[String],
    theme: Option<Theme>,
    quiet_ms: u64,
) -> Result<usize> {
    let binary = build_app(root)?;
    let comp = Compositor::start(dir)?;
    let cell = measure_cell(&comp, &base.font)?;
    if cell != base.cell {
        return Err(Error::Baseline(format!(
            "cell {}×{} disagrees with the baseline's {}×{} — run `measure --record` if the font or machine changed",
            cell.w, cell.h, base.cell.w, base.cell.h
        )));
    }
    let themes = theme.map_or_else(|| Theme::ALL.to_vec(), |t| vec![t]);
    let mut captured = 0;
    for name in scenes {
        for size in Size::ALL {
            for &theme in &themes {
                capture(
                    &comp,
                    &binary,
                    base,
                    cell,
                    name,
                    size,
                    theme,
                    Duration::from_millis(quiet_ms),
                    &[],
                    dir,
                )?;
                captured += 1;
            }
        }
    }
    Ok(captured)
}

/// Writes the pass record for a run that has passed, and says so.
fn record(root: &Path, state: &RunState) -> Result<()> {
    let path = gate::write_record(root, state)?;
    println!(
        "\nreview passed — recorded for tree {} in {}",
        state.tree,
        path.display()
    );
    println!("Commit exactly what is staged; any further change needs the loop again.");
    Ok(())
}

/// Every failure, a refusal or a broken machine, is printed as the sentence
/// it is rather than as `main`'s Debug form: the reader is whoever is
/// committing, and the sentence is what they act on.
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let root = workspace_root();
    let base = Baseline::load()?;

    match cli.command {
        Command::Judge {
            run,
            stage,
            findings,
        } => write_verdict(&root, &run, stage, &findings),

        Command::Gate => {
            let path = gate::check(&root)?;
            println!("reviewed: {}", path.display());
            Ok(())
        }

        Command::Scenes => {
            for name in scene::CATALOGUE {
                println!("{name}");
            }
            Ok(())
        }

        Command::Measure { record } => measure(&root, base, record),

        Command::Tokens { write } => {
            if write {
                write_tokens(&root, &base)
            } else {
                check_tokens(&root, &base)
            }
        }

        Command::Capture(args) => capture_frames(&root, &base, &args),

        Command::Review(args) => review(&root, &base, &args),
    }
}

/// `judge`: writes one judge's verdict into the run's report and state, and
/// records the pass once no judge is left.
fn write_verdict(root: &Path, run: &Path, stage: u8, findings: &Path) -> Result<()> {
    let judge = Judge::from_stage(stage).ok_or_else(|| {
        Error::Review(format!(
            "stage {stage} is not a judge; the judges are 6 (code), 7 (Rust) and 8 (frames)"
        ))
    })?;
    let verdict: Verdict = serde_json::from_str(&std::fs::read_to_string(findings)?)?;
    let report = run.join("review.html");
    let passed = report::write_verdict(&report, judge, &verdict)?;
    let mut state = RunState::load(run)?;
    for assignment in &mut state.assignments {
        if assignment.judge() == judge {
            assignment.record(passed)?;
        }
    }
    state.save(run)?;
    println!(
        "stage {stage}, {}: {} finding(s) — {}",
        judge.title(),
        verdict.findings.len(),
        if passed { "passes" } else { "does not pass" }
    );
    println!("report: {}", report.display());
    if !passed {
        return Err(Error::Review(format!(
            "stage {stage} has findings; fix them and run the loop again from stage 1"
        )));
    }
    let pending: Vec<String> = state
        .assignments
        .iter()
        .filter(|a| a.standing() == Standing::Pending)
        .map(|a| format!("{} ({})", a.judge().stage(), a.judge().title()))
        .collect();
    if pending.is_empty() {
        record(root, &state)?;
    } else {
        println!("still to write: stage {}", pending.join(", stage "));
    }
    Ok(())
}

/// `measure`: measures foot's cell and checks it against the baseline, or
/// records it there.
fn measure(root: &Path, base: Baseline, record: bool) -> Result<()> {
    let dir = frames_dir(root)?;
    let comp = Compositor::start(&dir)?;
    let cell = measure_cell(&comp, &base.font)?;
    println!("measured cell {}×{} for {}", cell.w, cell.h, base.font);
    if cell != base.cell {
        if !record {
            // Loudly, by design: a cell that has moved silently
            // rewrites every frame's geometry while every frame still
            // looks right.
            return Err(Error::Baseline(format!(
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

/// `tokens --write`: regenerates the app's tokens file from the design.
fn write_tokens(root: &Path, base: &Baseline) -> Result<()> {
    let text = tokens::generate(&tokens::design_dir(), base)?;
    let path = tokens::output_path(root);
    std::fs::write(&path, &text)?;
    println!("wrote {}", path.display());
    Ok(())
}

/// `tokens`: fails when the app's tokens file is not what the design
/// generates — the design and the app disagree.
fn check_tokens(root: &Path, base: &Baseline) -> Result<()> {
    let n = tokens::check(root, &tokens::design_dir(), base)?.map_err(Error::Design)?;
    println!("{} is current — {n} values from the design", tokens::OUTPUT);
    Ok(())
}

/// `capture`: takes the frames for one scene or all of them, and prints
/// where each landed.
fn capture_frames(root: &Path, base: &Baseline, args: &CaptureArgs) -> Result<()> {
    let dir = match &args.out {
        Some(path) => {
            std::fs::create_dir_all(path)?;
            path.clone()
        }
        None => frames_dir(root)?,
    };
    // Same reason as `capture_scenes`: never spawn a binary nobody
    // just built.
    let binary = build_app(root)?;
    let comp = Compositor::start(&dir)?;
    let cell = measure_cell(&comp, &base.font)?;
    let names: Vec<&str> = match &args.scene {
        Some(name) => vec![name.as_str()],
        None => scene::CATALOGUE.to_vec(),
    };
    let sizes = args.size.map_or_else(|| Size::ALL.to_vec(), |s| vec![s]);
    let themes = args.theme.map_or_else(|| Theme::ALL.to_vec(), |t| vec![t]);
    for name in names {
        for &s in &sizes {
            for &t in &themes {
                let frame = capture(
                    &comp,
                    &binary,
                    base,
                    cell,
                    name,
                    s,
                    t,
                    Duration::from_millis(args.quiet_ms),
                    &[],
                    &dir,
                )?;
                println!("{name} {s} {t}  {}", frame.display());
            }
        }
    }
    println!("\nframes {}", dir.display());
    Ok(())
}

/// `review`: stages 1 to 5, then the judges' inputs and the exit that says
/// whether the review is finished.
fn review(root: &Path, base: &Baseline, args: &ReviewArgs) -> Result<()> {
    if !args.stages_only {
        require_staged(root)?;
    }
    let tree = git::staged_tree(root)?;
    let outcomes = deterministic_stages(root, base)?;
    print_outcomes(&outcomes);
    let stages_passed = outcomes.iter().all(|o| o.passed);
    if args.stages_only {
        return stages_only_verdict(stages_passed);
    }

    let (assignments, scenes) = judges::assign(
        &git::staged_paths(root)?,
        &git::changed_scenes(root)?,
        scene::CATALOGUE,
    );
    let dir = frames_dir(root)?;
    let frames_needed = stages_passed && !scenes.is_empty() && !args.no_capture;
    let captured = if frames_needed {
        capture_scenes(root, base, &dir, &scenes, args.theme, args.quiet_ms)?
    } else {
        0
    };
    // Empty before the first commit, which is no reason to stop.
    let commit = git::head(root).unwrap_or_default();
    let run = report::Run {
        commit: &commit,
        tree: &tree,
        outcomes: &outcomes,
        assignments: &assignments,
        frames: frames_needed.then_some(dir.as_path()),
        captured,
    };
    let (written, state) = write_run(root, &dir, &run)?;
    print_assignments(&assignments, &written);
    hand_to_judges(root, &dir, &run, &state, &scenes)
}

/// Refuses a working tree that differs from the index.
///
/// A review builds and judges the working tree and records the staged one,
/// so they have to be the same tree. Staging is also the author saying what
/// the commit is — nothing else is asked.
fn require_staged(root: &Path) -> Result<()> {
    let unstaged = git::unstaged(root)?;
    if unstaged.is_empty() {
        return Ok(());
    }
    println!("Not staged, so not reviewable — stage what the commit is, and put the rest aside:");
    for path in &unstaged {
        println!("  {path}");
    }
    Err(Error::Review(
        "the working tree differs from the index".into(),
    ))
}

/// Stages 1 to 5, in order, each with its verdict.
fn deterministic_stages(root: &Path, base: &Baseline) -> Result<Vec<Outcome>> {
    let mut outcomes = stages::toolchain(root, &base.toolchain)?;
    outcomes.extend(stages::lint(root)?);
    outcomes.extend(stages::test(root)?);

    // Stage 4 before stage 5, because if the generated palette has
    // drifted then every frame below was drawn with the wrong
    // colours and reporting those would be reporting a consequence
    // as a cause.
    let design_dir = tokens::design_dir();
    outcomes.push(match tokens::check(root, &design_dir, base)? {
        Ok(n) => Outcome {
            stage: "4 tokens",
            passed: true,
            detail: format!("{n} values from the design"),
        },
        Err(detail) => Outcome {
            stage: "4 tokens",
            passed: false,
            detail: format!(
                "{detail}\n\nRegenerate with:\n    cargo run -p aldwin-review -- tokens --write"
            ),
        },
    });
    outcomes.extend(stages::frames(root)?);
    Ok(outcomes)
}

/// Prints each stage's verdict: a passing one on a line, a failing one with
/// its whole detail beneath.
fn print_outcomes(outcomes: &[Outcome]) {
    println!();
    for outcome in outcomes {
        if outcome.passed {
            let detail = if outcome.detail.is_empty() {
                String::new()
            } else {
                format!(" — {}", outcome.detail)
            };
            println!("  ok    {}{detail}", outcome.stage);
        } else {
            println!("  FAIL  {}", outcome.stage);
            for line in outcome.detail.lines() {
                println!("          {line}");
            }
        }
    }
}

/// `review --stages-only`'s answer, which rests on stages 1 to 5 alone.
fn stages_only_verdict(stages_passed: bool) -> Result<()> {
    if !stages_passed {
        return Err(Error::Review("a deterministic stage failed".into()));
    }
    println!("stages 1–5 clean. Not a review: no judge ran and nothing was recorded.");
    Ok(())
}

/// Writes the run into `dir` — the diff the judges read, the report and
/// `run.json` — and returns the report's path and the state it saved.
fn write_run(root: &Path, dir: &Path, run: &report::Run) -> Result<(PathBuf, RunState)> {
    // The judges read the change from a file, the same bytes for
    // each: a judge that runs `git diff` itself can run a different
    // one.
    std::fs::write(dir.join("change.diff"), git::staged_diff(root)?)?;
    let written = report::write(dir, run)?;
    let state = RunState {
        tree: run.tree.to_string(),
        stages_passed: run.outcomes.iter().all(|o| o.passed),
        assignments: run.assignments.to_vec(),
    };
    state.save(dir)?;
    Ok((written, state))
}

/// Prints whether each judge is required and why, then where the report is.
fn print_assignments(assignments: &[Assignment], report: &Path) {
    println!();
    for a in assignments {
        println!(
            "  stage {} {:<13} {} — {}",
            a.judge().stage(),
            a.judge().title(),
            if a.standing().required() {
                "required"
            } else {
                "not required"
            },
            a.reason()
        );
    }
    println!("\nreport: {}", report.display());
}

/// Ends a review: records it when no judge is needed, and otherwise prints
/// what each judge reads and the command that writes its verdict, and fails
/// — the review is not finished until they have run.
fn hand_to_judges(
    root: &Path,
    dir: &Path,
    run: &report::Run,
    state: &RunState,
    scenes: &[String],
) -> Result<()> {
    if !state.stages_passed {
        return Err(Error::Review(
            "a deterministic stage failed; no judge runs until stages 1–5 pass".into(),
        ));
    }
    let reached: Vec<Judge> = Judge::ALL.into_iter().filter(|&j| run.reaches(j)).collect();
    let unreached = run
        .assignments
        .iter()
        .any(|a| a.standing().required() && !reached.contains(&a.judge()));
    if unreached {
        println!("Stage 8 needs frames: run the review again without --no-capture.");
        return Err(Error::Review(
            "review incomplete: frames not captured".into(),
        ));
    }
    if reached.is_empty() {
        // Nothing in this change is for a judge, so stages 1–5 are
        // the whole review.
        return record(root, state);
    }

    print_judge_inputs(dir, &reached, scenes);
    // Deliberately an error. A review without its judges is not a
    // review, and a command that exits zero reads as completion —
    // that is how the old stage 5 came back empty on five of seven runs. The
    // only way to a pass is through `judge`.
    Err(Error::Review(format!(
        "review incomplete: {} judge(s) to run",
        reached.len()
    )))
}

/// Prints the exact inputs each reached judge reads and the exact command
/// that writes its verdict.
///
/// With the directory filled in: the old stage 5's section was left empty on
/// three separate runs because writing it depended on recalling a command
/// rather than copying one.
fn print_judge_inputs(dir: &Path, reached: &[Judge], scenes: &[String]) {
    println!(
        "\nThe change, for stages 6 and 7: {}",
        dir.join("change.diff").display()
    );
    if reached.contains(&Judge::Frames) {
        println!("The frames for stage 8 — the changed scenes only:");
        for scene_name in scenes {
            for size in Size::ALL {
                for theme in Theme::ALL {
                    let png = dir.join(format!("{scene_name}-{size}-{theme}.png"));
                    if png.exists() {
                        println!("  {}", png.display());
                    }
                }
            }
        }
        println!("Each has a .txt beside it: the declared grid, every character at its\nexact column. Use it for geometry and the .png only for colour.");
    }
    println!("\nAfter each judge, save its JSON block verbatim and run:");
    for judge in reached {
        println!(
            "  cargo run --release -p aldwin-review -- judge --run {} --stage {} --findings <file.json>",
            dir.display(),
            judge.stage()
        );
    }
}
