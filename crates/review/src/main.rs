//! `aldwin-review`: the review loop's commands.
//!
//! `review` runs stages 1 to 5 and hands the judges it assigns their inputs;
//! `judge` writes a verdict and records the pass once every required stage
//! has passed; `gate` is stage 10, run by `.githooks/pre-commit`. `tokens`,
//! `capture` and `measure` serve stages 4 and 8.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aldwin_review::capture::{capture, measure_cell};
use aldwin_review::geometry::{Size, Theme};
use aldwin_review::judges::{Assignment, Judge, RunState, Standing};
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
    /// Regenerate the app's design system from `docs/design/`.
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
        /// Each reader's JSON block, saved verbatim: `{"iteration": 1,
        /// "findings": [{"severity","source","expected","found","at"}],
        /// "matches": [], "contradictions": [], "questions": []}`. Once per
        /// reader: twice for stage 6, whose two readers' findings are
        /// merged into one verdict.
        #[arg(long, required = true)]
        findings: Vec<PathBuf>,
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

/// How many earlier runs are kept, and so how far back a judge's pass can
/// carry from (Decision 17).
const KEPT_RUNS: usize = 3;

/// A fresh `<kind>-<seconds>` directory, after pruning that kind to the last
/// [`KEPT_RUNS`]. `run` is a review's; `measure` and `capture` write `probe`,
/// so a probe never prunes, or shares, a run whose pass could carry.
fn frames_dir(root: &Path, kind: &str) -> Result<PathBuf> {
    let parent = root.join("target/review-frames");
    std::fs::create_dir_all(&parent)?;
    let prefix = format!("{kind}-");
    let mut existing: Vec<PathBuf> = std::fs::read_dir(&parent)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with(&prefix))
        })
        .collect();
    existing.sort();
    for old in existing
        .iter()
        .take(existing.len().saturating_sub(KEPT_RUNS))
    {
        let _ = std::fs::remove_dir_all(old);
    }
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?;
    let dir = parent.join(format!("{prefix}{}", now.as_secs()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Builds `target/debug/aldwin` and returns its path.
///
/// Never replace the build with an existence check: a stale binary gives
/// stage 8 frames that predate the change. Incremental, so cheap when
/// current.
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

/// Captures `scenes` into `dir` for stage 8 and returns how many frames.
///
/// Pictures only, no assertions: checks on declared cells belong in
/// `crates/tui`'s hermetic tests (Decision 8).
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
        state.tree(),
        path.display()
    );
    println!("Commit exactly what is staged; any further change needs the loop again.");
    Ok(())
}

/// Prints every failure as its `Display` sentence, never `main`'s `Debug`
/// form.
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
fn write_verdict(root: &Path, run: &Path, stage: u8, findings: &[PathBuf]) -> Result<()> {
    let judge = Judge::from_stage(stage).ok_or_else(|| {
        Error::Review(format!(
            "stage {stage} is not a judge; the judges are 6 (code), 7 (Rust) and 8 (frames)"
        ))
    })?;
    let readers = findings
        .iter()
        .map(|path| Ok(serde_json::from_str(&std::fs::read_to_string(path)?)?))
        .collect::<Result<Vec<Verdict>>>()?;
    let verdict = Verdict::of(judge, readers)?;
    let report = run.join("review.html");
    let passed = report::write_verdict(&report, judge, &verdict)?;
    let mut state = RunState::load(run)?;
    state.record(judge, passed)?;
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
    // Placeholders, not pending judges: a frames judge with no frames
    // captured is pending but has nowhere to be written.
    let open: Vec<String> = report::awaiting(&report)?
        .into_iter()
        .map(|j| format!("{} ({})", j.stage(), j.title()))
        .collect();
    if !open.is_empty() {
        println!("still to write: stage {}", open.join(", stage "));
        return Ok(());
    }
    if state.has_findings() {
        return Err(Error::Review(
            "every judge that could run has, and one has findings; fix them and run the \
             loop again"
                .into(),
        ));
    }
    if !state.passed() {
        return Err(Error::Review(
            "every judge that could run has, and the frames judge was not reached: its \
             frames were not captured. Run the review again without --no-capture."
                .into(),
        ));
    }
    record(root, &state)
}

/// `measure`: measures foot's cell and checks it against the baseline, or
/// records it there.
fn measure(root: &Path, base: Baseline, record: bool) -> Result<()> {
    let dir = frames_dir(root, "probe")?;
    let comp = Compositor::start(&dir)?;
    let cell = measure_cell(&comp, &base.font)?;
    println!("measured cell {}×{} for {}", cell.w, cell.h, base.font);
    if cell != base.cell {
        if !record {
            // Must fail: a moved cell changes every frame's geometry while
            // every frame still looks right.
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
/// generates.
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
        None => frames_dir(root, "probe")?,
    };
    // See `build_app`: never spawn a binary not just built.
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

/// `review`: stages 1 to 5, then the judges' inputs; exits non-zero while a
/// judge is left to run (Decision 12).
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

    let fingerprints = Judge::ALL
        .into_iter()
        .map(|judge| Ok((judge, git::fingerprint(root, judge.reads())?)))
        .collect::<Result<Vec<_>>>()?;
    let (mut state, scenes) = RunState::assess(
        tree.clone(),
        stages_passed,
        &git::staged_paths(root)?,
        &git::changed_scenes(root)?,
        // A one-theme capture gets no frames fingerprint, so no frames pass
        // carries into or out of it.
        |judge| {
            fingerprints
                .iter()
                .find(|(j, _)| *j == judge && !(judge == Judge::Frames && args.theme.is_some()))
                .map(|(_, f)| f.clone())
        },
    );
    let dir = frames_dir(root, "run")?;
    for (name, earlier) in earlier_runs(&dir) {
        state.carry_from(&earlier, &name);
    }
    let frames_pending = state
        .assignments()
        .iter()
        .any(|a| a.judge() == Judge::Frames && a.standing() == Standing::Pending);
    let frames_needed = stages_passed && frames_pending && !args.no_capture;
    let captured = if frames_needed {
        capture_scenes(root, base, &dir, &scenes, args.theme, args.quiet_ms)?
    } else {
        0
    };
    // Empty before the first commit.
    let commit = git::head(root).unwrap_or_default();
    let run = report::Run {
        commit: &commit,
        tree: &tree,
        outcomes: &outcomes,
        assignments: state.assignments(),
        frames: frames_needed.then_some(dir.as_path()),
        captured,
    };
    let written = write_run(root, &dir, &run, &state)?;
    print_assignments(state.assignments(), &written);
    hand_to_judges(root, &dir, &run, &state, &scenes)
}

/// Refuses a working tree that differs from the index.
///
/// A review builds and judges the working tree but records the staged one
/// (Decision 15).
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

    // Stage 4 before 5: frames drawn with a drifted palette would report a
    // consequence as a cause.
    outcomes.extend(stages::tokens(root, base)?);
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
            println!("  ok    {}{detail}", outcome.stage.label());
        } else {
            println!("  FAIL  {}", outcome.stage.label());
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
/// `run.json` — and returns the report's path.
fn write_run(root: &Path, dir: &Path, run: &report::Run, state: &RunState) -> Result<PathBuf> {
    // One file, so every judge reads the same bytes rather than its own
    // `git diff`.
    std::fs::write(dir.join("change.diff"), git::staged_diff(root)?)?;
    let written = report::write(dir, run)?;
    state.save(dir)?;
    Ok(written)
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

/// Ends a review: records it when no judge is left, and otherwise prints
/// each judge's inputs and command, and fails.
fn hand_to_judges(
    root: &Path,
    dir: &Path,
    run: &report::Run,
    state: &RunState,
    scenes: &[String],
) -> Result<()> {
    let reached: Vec<Judge> = Judge::ALL.into_iter().filter(|&j| run.reaches(j)).collect();
    if reached.is_empty() && state.passed() {
        // No judge left: none called for, or each carried.
        return record(root, state);
    }
    if !reached.is_empty() {
        print_judge_inputs(dir, &reached, scenes);
    }
    let unreached = run
        .assignments
        .iter()
        .any(|a| a.standing() == Standing::Pending && !reached.contains(&a.judge()));
    // Must stay an error (Decision 12): a zero exit reads as completion. A
    // pass comes only from `judge`, or from above when no judge is left.
    Err(Error::Review(if !state.stages_passed() {
        // `reaches` left no judge (Decision 18); this only says why.
        "a deterministic stage failed, so no judge runs; fix it with `review \
         --stages-only` until stages 1–5 pass, then run the loop again"
            .into()
    } else if unreached {
        println!("Stage 8 needs frames: run the review again without --no-capture.");
        "review incomplete: frames not captured".into()
    } else {
        format!("review incomplete: {} judge(s) to run", reached.len())
    }))
}

/// The runs before `current` whose state loads, by directory name, newest
/// first so a carried pass names the latest run it could come from. Capped
/// at [`KEPT_RUNS`].
fn earlier_runs(current: &Path) -> Vec<(String, RunState)> {
    let Some(parent) = current.parent() else {
        return Vec::new();
    };
    let mut runs: Vec<(String, RunState)> = std::fs::read_dir(parent)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path != current)
        .filter_map(|path| {
            let state = RunState::load(&path).ok()?;
            Some((path.file_name()?.to_string_lossy().into_owned(), state))
        })
        .collect();
    // `run-<seconds>`: the names sort as the times do.
    runs.sort_by(|a, b| b.0.cmp(&a.0));
    runs.truncate(KEPT_RUNS);
    runs
}

/// Prints the exact inputs each reached judge reads and the exact command,
/// directory filled in, that writes its verdict.
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
    println!("\nAfter each judge, save each reader's JSON block verbatim and run:");
    for judge in reached {
        let files: Vec<String> = (1..=judge.readers())
            .map(|n| format!("--findings <reader-{n}.json>"))
            .collect();
        println!(
            "  cargo run --release -p aldwin-review -- judge --run {} --stage {} {}",
            dir.display(),
            judge.stage(),
            files.join(" ")
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: a `measure` or `capture` counted toward the kept runs and
    /// pruned a review run whose pass could carry.
    #[test]
    fn a_probe_never_prunes_a_review_run() {
        let root = tempfile::tempdir().unwrap();
        let parent = root.path().join("target/review-frames");
        for name in [
            "run-1", "run-2", "run-3", "probe-4", "probe-5", "probe-6", "probe-7",
        ] {
            std::fs::create_dir_all(parent.join(name)).unwrap();
        }

        let probe = frames_dir(root.path(), "probe").unwrap();

        for run in ["run-1", "run-2", "run-3"] {
            assert!(parent.join(run).is_dir(), "{run} was pruned");
        }
        assert!(!parent.join("probe-4").exists(), "the oldest probe goes");
        assert!(probe
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("probe-"));
    }
}
