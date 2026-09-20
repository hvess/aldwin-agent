//! `mjolnir-screenshot` — the harness the screenshot skill drives.
//!
//! Two commands exist so far, and they are the two the loop is built on:
//! `measure` establishes the cell this machine renders, and `capture` takes
//! frames. Gates, scoring and the report follow in Steps 8–11 of the spec.

use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::{Parser, Subcommand};
use mjolnir_screenshot::capture::{capture, measure_cell};
use mjolnir_screenshot::geometry::{Size, Theme};
use mjolnir_screenshot::design::Design;
use mjolnir_screenshot::session::Session;
use mjolnir_screenshot::{regression, report, scene, Baseline, Compositor};

#[derive(Parser)]
#[command(about = "Capture and score Mjolnir's TUI as a real terminal renders it")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Measure foot's cell for the pinned font and check it against the baseline.
    Measure {
        /// Write the measured cell into the baseline instead of failing on a mismatch.
        #[arg(long)]
        record: bool,
    },
    /// Capture one scene. Defaults to all three sizes in both themes — six frames.
    Capture {
        /// Scene name; see `scene::CATALOGUE`.
        #[arg(long)]
        scene: String,
        /// Restrict to one size rather than all three.
        #[arg(long)]
        size: Option<Size>,
        /// Restrict to one theme rather than both.
        #[arg(long)]
        theme: Option<Theme>,
        /// Where frames land. Defaults to a fresh run directory under target/screenshot-runs/.
        #[arg(long)]
        out: Option<PathBuf>,
        /// The binary under test.
        #[arg(long)]
        binary: Option<PathBuf>,
        /// Keys to send before capturing, e.g. Down,Down,Enter or "hello".
        #[arg(long)]
        keys: Option<String>,
        /// How long the app must be silent before a frame is taken.
        #[arg(long, default_value_t = 400)]
        quiet_ms: u64,
    },
    /// Print the scene catalogue and which scenes are wired up.
    Scenes,
    /// Delete every run directory but the newest few.
    Clean {
        /// How many to keep.
        #[arg(long, default_value_t = KEEP_RUNS)]
        keep: usize,
    },
    /// Open a session: state the goal, the focus set and the scenes, once,
    /// before anything runs.
    Start {
        #[arg(long)]
        goal: String,
        /// Scenes the change may alter, e.g. "conversation,approval:10-14".
        #[arg(long)]
        focus: String,
        /// Scenes to capture. Defaults to the ones named in the focus set.
        #[arg(long)]
        scenes: Option<String>,
    },
    /// The run-level checks, as one pass. A failure ends the session.
    Preflight {
        #[arg(long)]
        run: PathBuf,
    },
    /// Capture every scene the session names, then gate them.
    Run {
        #[arg(long)]
        run: PathBuf,
        /// How long the app must be silent before a frame is taken. The
        /// default is the settled value; lower it for an iteration pass and
        /// restore it for the one the verdict is read from, since a frame
        /// taken too early is a frame of a half-drawn app.
        #[arg(long, default_value = "400")]
        quiet_ms: u64,
        /// Capture one theme only. The themes are a pure palette swap — the
        /// harness proves it per run, see `themes_are_a_palette_swap` — so a
        /// fix pass that is not about colour can halve its capture time.
        #[arg(long)]
        theme: Option<Theme>,
    },
    /// Assemble the report from what the run directory already holds.
    Report {
        #[arg(long)]
        run: PathBuf,
    },
    /// The exit condition, computed from the run — gates, scores and the
    /// iteration cap. Exits non-zero when the loop may not stop.
    Verdict {
        #[arg(long)]
        run: PathBuf,
    },
    /// Every failed design assertion in the run, grouped by screen, each
    /// with the `HANDOFF.md` line it comes from.
    ///
    /// This is the deterministic half of what six blind judges used to
    /// produce, and it is the first thing to read after `run`.
    Conformance {
        #[arg(long)]
        run: PathBuf,
    },
    /// The frames a blind judge should be handed, and what it may read.
    ///
    /// One theme, because the harness proves per run that the two are a
    /// palette swap; the design frame size, because that is the only frame
    /// the design specifies. Twelve frames rather than seventy-two.
    JudgeSet {
        #[arg(long)]
        run: PathBuf,
        /// Include every size rather than the design frame alone.
        #[arg(long)]
        all_sizes: bool,
    },
    /// The regression gate: which snapshot regions moved, and whether the
    /// focus set accounts for them.
    Regression {
        /// Scenes the change is allowed to alter, e.g. "conversation,approval:10-14".
        #[arg(long)]
        focus: String,
        /// Revision to compare against. Defaults to the merge-base with main.
        #[arg(long)]
        baseline: Option<String>,
    },
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("workspace root")
}

/// How many run directories survive a new session.
///
/// A run is deleted when its report is accepted, but a failed or abandoned one
/// keeps its directory as evidence for the next attempt — and nothing was ever
/// sweeping those. Building this harness left 70 of them, 24MB, in an
/// afternoon. Evidence is only useful while it is recent, so the newest few
/// stay and the rest go.
const KEEP_RUNS: usize = 5;

/// Drop the oldest run directories, loudly enough to notice.
fn prune_runs(root: &Path) -> std::io::Result<()> {
    let mut runs: Vec<PathBuf> = match std::fs::read_dir(root) {
        Ok(entries) => entries.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir()).collect(),
        Err(_) => return Ok(()),
    };
    if runs.len() <= KEEP_RUNS {
        return Ok(());
    }
    // Names are `run-<unix seconds>`, so lexical order is chronological.
    runs.sort();
    let stale = runs.len() - KEEP_RUNS;
    for old in runs.iter().take(stale) {
        std::fs::remove_dir_all(old)?;
    }
    println!("pruned {stale} older run director{} (keeping {KEEP_RUNS})", if stale == 1 { "y" } else { "ies" });
    Ok(())
}

fn run_dir(explicit: Option<PathBuf>) -> std::io::Result<PathBuf> {
    let dir = match explicit {
        Some(d) => d,
        None => {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            workspace_root().join("target/screenshot-runs").join(format!("run-{stamp}"))
        }
    };
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Rebuild the frame list from what the run directory holds, so `report` and
/// `verdict` read the same measured facts rather than either recomputing them.
fn load_frames(run: &Path) -> std::io::Result<Vec<report::Frame>> {
    let manifest: Vec<serde_json::Value> = serde_json::from_str(&std::fs::read_to_string(run.join("frames.json"))?)?;
    let mut frames = Vec::new();
    for entry in manifest {
        let gates_path = entry["gates"].as_str().unwrap_or_default();
        let gates: report::GateReportShape = serde_json::from_str(&std::fs::read_to_string(gates_path)?)?;
        // A run directory from before the assertion suite existed has no
        // `expect.json`. Treating that as "nothing was checked" rather than
        // as "everything passed" is what makes `verdict` refuse it instead of
        // reporting a clean run it never measured.
        let expect = match entry["expect"].as_str() {
            Some(path) => serde_json::from_str(&std::fs::read_to_string(path)?)?,
            None => mjolnir_screenshot::expect::Outcome::default(),
        };
        frames.push(report::Frame {
            scene:     entry["scene"].as_str().unwrap_or_default().to_string(),
            size:      entry["size"].as_str().unwrap_or_default().to_string(),
            theme:     entry["theme"].as_str().unwrap_or_default().to_string(),
            png:       PathBuf::from(entry["png"].as_str().unwrap_or_default()),
            annotated: entry["annotated"].as_str().map(PathBuf::from),
            gates,
            expect,
        });
    }
    Ok(frames)
}

/// Are the two themes the same frame in different paint?
///
/// This is not a nicety. Half the capture matrix, half the gate time and —
/// before this — half the judging budget went on light frames, and if the
/// declared grids are identical then a light frame *cannot* hold a spatial
/// defect its dark twin does not. Proving it per run is what licenses judging
/// one theme instead of two, and what makes `run --theme dark` a legitimate
/// shortcut rather than a guess.
///
/// It is proved rather than assumed because it is a property of the app, not
/// of the design: a theme-conditional layout would break it, silently, and
/// the run that introduced one is exactly the run that must not be allowed to
/// go on judging one theme. `run-1789850385` had 36 identical pairs.
///
/// Returns the number of pairs compared, or the first frame where they
/// diverge.
fn themes_are_a_palette_swap(run: &Path, scenes: &[String]) -> std::io::Result<std::result::Result<usize, String>> {
    let mut pairs = 0;
    for scene in scenes {
        for size in Size::ALL {
            let dark = run.join(format!("{scene}-{size}-dark.txt"));
            let light = run.join(format!("{scene}-{size}-light.txt"));
            let (Ok(a), Ok(b)) = (std::fs::read_to_string(&dark), std::fs::read_to_string(&light)) else { continue };
            if a != b {
                let row = a.lines().zip(b.lines()).position(|(x, y)| x != y);
                return Ok(Err(match row {
                    Some(n) => format!("{scene} {size}, first at row {n}"),
                    None => format!("{scene} {size}, different row counts"),
                }));
            }
            pairs += 1;
        }
    }
    Ok(Ok(pairs))
}

fn main() -> std::io::Result<()> {
    let cli = Cli::parse();
    let base = Baseline::load()?;
    let design = Design::load()?;

    match cli.command {
        Command::Scenes => {
            for name in scene::CATALOGUE {
                let mark = if scene::IMPLEMENTED.contains(name) { "ready" } else { "needs the fixture endpoint" };
                println!("{name:<16} {mark}");
            }
            Ok(())
        }

        Command::Clean { keep } => {
            let root = workspace_root().join("target/screenshot-runs");
            let before = std::fs::read_dir(&root).map(|e| e.count()).unwrap_or(0);
            // `prune_runs` keeps a constant; honour the flag by trimming to it
            // here and letting the constant handle the default path.
            let mut runs: Vec<PathBuf> = std::fs::read_dir(&root)
                .map(|entries| entries.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir()).collect())
                .unwrap_or_default();
            runs.sort();
            for old in runs.iter().take(runs.len().saturating_sub(keep)) {
                std::fs::remove_dir_all(old)?;
            }
            println!("{} run directories, {} kept", before, keep.min(runs.len()));
            Ok(())
        }

        Command::Start { goal, focus, scenes } => {
            let scenes: Vec<String> = match scenes {
                Some(list) => list.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
                None => focus.split(',').map(|f| f.split(':').next().unwrap_or("").trim().to_string()).filter(|s| !s.is_empty()).collect(),
            };
            for name in &scenes {
                if !scene::IMPLEMENTED.contains(&name.as_str()) {
                    return Err(std::io::Error::other(format!("scene {name:?} is not wired up; see `scenes`")));
                }
            }
            let root = workspace_root();
            prune_runs(&root.join("target/screenshot-runs"))?;
            let dir = run_dir(None)?;
            let session = Session {
                goal,
                focus: focus.clone(),
                scenes,
                baseline: regression::merge_base(&root, "main")?,
                iterations: vec![],
                scores: vec![],
                regression: None,
            };
            session.save(&dir)?;
            println!("run {}", dir.display());
            println!("baseline {}", &session.baseline[..12.min(session.baseline.len())]);
            println!("scenes {}", session.scenes.join(", "));
            Ok(())
        }

        Command::Preflight { run } => {
            let root = workspace_root();
            let session = Session::load(&run)?;
            println!("goal      {}", session.goal.lines().next().unwrap_or(""));
            println!("focus     {}", session.focus);

            let binary = root.join("target/debug/mjolnir");
            if !binary.exists() {
                return Err(std::io::Error::other("no binary under test — cargo build -p mjolnir-cli"));
            }
            println!("binary    ok");

            // The snapshot must match the code, or the regression gate is
            // comparing against fiction and will report "clean".
            let snap = std::process::Command::new("cargo")
                .current_dir(&root)
                .args(["test", "-q", "-p", "mjolnir-tui", "--test", "render_snapshot"])
                .output()?;
            if !snap.status.success() {
                return Err(std::io::Error::other("render_snapshot is not green — the regression gate would compare against a stale snapshot"));
            }
            println!("snapshot  ok");

            let dir = run_dir(Some(run.join("preflight")))?;
            let comp = Compositor::start(&dir)?;
            let cell = measure_cell(&comp, &base.font)?;
            if cell != base.cell {
                return Err(std::io::Error::other(format!(
                    "cell {}×{} disagrees with the baseline's {}×{} — every frame would be wrong in every row",
                    cell.w, cell.h, base.cell.w, base.cell.h
                )));
            }
            println!("cell      {}×{}", cell.w, cell.h);
            println!("\npreflight clear");
            Ok(())
        }

        Command::Run { run, quiet_ms, theme: only_theme } => {
            let root = workspace_root();
            let session = Session::load(&run)?;
            let binary = root.join("target/debug/mjolnir");
            let comp = Compositor::start(&run)?;
            let cell = measure_cell(&comp, &base.font)?;
            if cell != base.cell {
                return Err(std::io::Error::other("cell disagrees with the baseline; run preflight"));
            }

            let themes: Vec<Theme> = match only_theme {
                Some(one) => vec![one],
                None => Theme::ALL.to_vec(),
            };
            let mut manifest = Vec::new();
            let mut assertions = 0usize;
            let mut failed = 0usize;
            for scene_name in &session.scenes {
                for size in Size::ALL {
                    for theme in themes.iter().copied() {
                        let frame = capture(
                            &comp,
                            &binary,
                            &design,
                            &base,
                            cell,
                            scene_name,
                            size,
                            theme,
                            Duration::from_millis(quiet_ms),
                            &[],
                            &run,
                        )?;
                        let gates = match frame.gates.by_gate().as_slice() {
                            [] => "clean".to_string(),
                            counts => counts.iter().map(|(g, n)| format!("{g} {n}")).collect::<Vec<_>>().join(", "),
                        };
                        assertions += frame.expect.checked;
                        failed += frame.expect.failures.len();
                        let expect = match frame.expect.failures.len() {
                            0 => format!("{} assertions clean", frame.expect.checked),
                            n => format!("{n}/{} assertions FAILED", frame.expect.checked),
                        };
                        println!("{scene_name} {size} {theme}  {} cells  {gates}  {expect}", frame.checked);
                        for failure in &frame.expect.failures {
                            println!("    {} expects {} — {}", failure.screen, failure.rule, failure.detail);
                            println!("      {}", failure.cite);
                        }
                        manifest.push(serde_json::json!({
                            "scene": frame.scene,
                            "size": frame.size.to_string(),
                            "theme": frame.theme.to_string(),
                            "png": frame.path,
                            "annotated": frame.annotated,
                            "facts": frame.facts,
                            "gates": run.join(format!("{scene_name}-{size}-{theme}.gates.json")),
                            "expect": run.join(format!("{scene_name}-{size}-{theme}.expect.json")),
                        }));
                    }
                }
            }
            std::fs::write(run.join("frames.json"), serde_json::to_string_pretty(&manifest)?)?;

            println!("\n{assertions} design assertions, {failed} failed");
            if only_theme.is_none() {
                match themes_are_a_palette_swap(&run, &session.scenes)? {
                    Ok(pairs) => println!("themes are a palette swap ({pairs} frame pairs, identical declared grids)"),
                    Err(where_) => println!("THEMES DIVERGE: {where_} — a light frame can now hold a defect its dark twin does not"),
                }
            }

            // Recorded, not just printed. The verdict reads it back out of
            // the session: a regression nobody stored is a gate that cannot
            // block the exit.
            let changes = regression::changes(&root, &session.baseline)?;
            let loose = regression::unaccounted(&changes, &regression::Focus::parse(&session.focus).map_err(std::io::Error::other)?);
            let mut session = session;
            session.regression = Some(mjolnir_screenshot::session::Regression {
                baseline:    session.baseline.clone(),
                moved:       changes.len(),
                unaccounted: loose.iter().map(|c| c.section.clone()).collect(),
            });
            session.save(&run)?;
            if loose.is_empty() {
                println!("\nregression clean ({} sections moved, all in focus)", changes.len());
            } else {
                for change in &loose {
                    println!("\nREGRESSION out of focus: {} rows {:?}", change.section, change.rows);
                }
            }
            println!("\nrun {}", run.display());
            Ok(())
        }

        Command::Verdict { run } => {
            let session = Session::load(&run)?;
            let frames = load_frames(&run)?;
            let (verdict, passed) = report::verdict(&session, &frames);
            println!("{verdict}");
            if passed {
                Ok(())
            } else {
                Err(std::io::Error::other("the exit condition is not met"))
            }
        }

        Command::Report { run } => {
            let session = Session::load(&run)?;
            let frames = load_frames(&run)?;
            let path = report::write(&run, &session, &frames)?;
            println!("report {}", path.display());
            Ok(())
        }

        Command::Conformance { run } => {
            let frames = load_frames(&run)?;
            let mut total = 0;
            let mut checked = 0;
            let mut by_screen: Vec<(String, Vec<String>)> = Vec::new();
            for frame in &frames {
                checked += frame.expect.checked;
                for failure in &frame.expect.failures {
                    total += 1;
                    let line = format!(
                        "  {}-{}-{} row {}  {}\n      {}\n      {}",
                        frame.scene,
                        frame.size,
                        frame.theme,
                        failure.row.map(|r| r.to_string()).unwrap_or_else(|| "-".into()),
                        failure.rule,
                        failure.detail,
                        failure.cite
                    );
                    match by_screen.iter_mut().find(|(s, _)| *s == failure.screen) {
                        Some((_, lines)) => lines.push(line),
                        None => by_screen.push((failure.screen.clone(), vec![line])),
                    }
                }
            }
            for (screen, lines) in &by_screen {
                println!("{screen} — {} failed", lines.len());
                for line in lines {
                    println!("{line}");
                }
                println!();
            }
            let skipped: Vec<&String> = frames.iter().flat_map(|f| f.expect.skipped.iter()).collect();
            if !skipped.is_empty() {
                let mut names: Vec<&str> = skipped.iter().map(|s| s.as_str()).collect();
                names.sort_unstable();
                names.dedup();
                println!("not anchored on some frames, so unchecked there: {}", names.join(", "));
            }
            println!("{checked} assertions checked, {total} failed");
            Ok(())
        }

        Command::JudgeSet { run, all_sizes } => {
            let frames = load_frames(&run)?;
            // Dark rather than light for one reason only: the dark theme is
            // the one the design system was authored in, so its frames are
            // the ones the handoff's own screens can be held against.
            let wanted: Vec<&report::Frame> = frames
                .iter()
                .filter(|f| f.theme == "dark" && (all_sizes || f.size == Size::Medium.to_string()))
                .collect();
            println!("# Frames for the judge — {} of {}", wanted.len(), frames.len());
            println!("#");
            println!("# One theme: the harness proves per run that the declared grids are");
            println!("# identical across themes, so a light frame cannot hold a spatial");
            println!("# defect its dark twin does not. What differs is contrast, and the");
            println!("# `contrast` gate measures that better than an eye can.");
            if !all_sizes {
                println!("# One size: 120x36 is the frame the design specifies. Degradation at");
                println!("# 80x24 and 200x50 is a judgement, not a stated rule.");
            }
            println!();
            for frame in &wanted {
                let stem = format!("{}-{}-{}", frame.scene, frame.size, frame.theme);
                println!("{}", run.join(format!("{stem}.png")).display());
                println!("{}", run.join(format!("{stem}.facts.txt")).display());
            }
            println!();
            println!("# The judge reads .claude/design/ (IMPORT.md first), .claude/design/ERRATA.md");
            println!("# and .claude/adr/*.md — and nothing else. Not crates/, not .claude/spec/.");
            println!("#");
            println!("# Hand it the .facts.txt, not a PNG to decode: it carries every span's");
            println!("# declared role, its hex and its contrast. Six judges on run-1789850385");
            println!("# spent most of 890K tokens recovering exactly that from pixels, and");
            println!("# three of them inferred a role name wrongly while doing it.");
            println!("#");
            println!("# Ask for findings against named rules, not for a 0-100 score. The");
            println!("# score is advisory now; the gate is `mjolnir-screenshot conformance`.");
            Ok(())
        }

        Command::Regression { focus, baseline } => {
            let root = workspace_root();
            let focus = mjolnir_screenshot::regression::Focus::parse(&focus).map_err(std::io::Error::other)?;
            let rev = match baseline {
                Some(rev) => rev,
                None => mjolnir_screenshot::regression::merge_base(&root, "main")?,
            };
            let changes = mjolnir_screenshot::regression::changes(&root, &rev)?;
            println!("baseline {}", &rev[..rev.len().min(12)]);
            for change in &changes {
                println!("  moved: {} ({} rows)", change.section, change.rows.len());
            }
            let loose = mjolnir_screenshot::regression::unaccounted(&changes, &focus);
            if loose.is_empty() {
                println!("regression gate: clean ({} sections moved, all in focus)", changes.len());
                Ok(())
            } else {
                for change in &loose {
                    println!("  OUT OF FOCUS: {} rows {:?}", change.section, change.rows);
                }
                Err(std::io::Error::other(format!("{} snapshot sections moved outside the focus set", loose.len())))
            }
        }

        Command::Measure { record } => {
            let dir = run_dir(None)?;
            let comp = Compositor::start(&dir)?;
            let cell = measure_cell(&comp, &base.font)?;
            println!("measured cell {}×{} for {}", cell.w, cell.h, base.font);
            if cell != base.cell {
                if record {
                    let mut updated = base;
                    updated.cell = cell;
                    updated.save()?;
                    println!("recorded in {}", Baseline::path().display());
                } else {
                    // Loudly, by design: a cell that has moved silently
                    // rewrites every frame's geometry while every frame still
                    // looks right.
                    return Err(std::io::Error::other(format!(
                        "cell {}×{} disagrees with the baseline's {}×{} — rerun with --record if the font or machine changed",
                        cell.w, cell.h, base.cell.w, base.cell.h
                    )));
                }
            }
            Ok(())
        }

        Command::Capture { scene: scene_name, size, theme, out, binary, keys, quiet_ms } => {
            let keys = match keys.as_deref() {
                Some(spec) => mjolnir_screenshot::keys::parse(spec).map_err(std::io::Error::other)?,
                None => Vec::new(),
            };
            let binary = binary.unwrap_or_else(|| workspace_root().join("target/debug/mjolnir"));
            if !binary.exists() {
                return Err(std::io::Error::other(format!("no binary at {} — cargo build -p mjolnir-cli", binary.display())));
            }
            let dir = run_dir(out)?;
            let comp = Compositor::start(&dir)?;

            let cell = measure_cell(&comp, &base.font)?;
            if cell != base.cell {
                return Err(std::io::Error::other(format!(
                    "cell {}×{} disagrees with the baseline's {}×{} — every frame would be wrong in every row; `measure --record` if this is intended",
                    cell.w, cell.h, base.cell.w, base.cell.h
                )));
            }

            let sizes: Vec<Size> = size.map(|s| vec![s]).unwrap_or_else(|| Size::ALL.to_vec());
            let themes: Vec<Theme> = theme.map(|t| vec![t]).unwrap_or_else(|| Theme::ALL.to_vec());

            for size in &sizes {
                for theme in &themes {
                    let frame = capture(
                        &comp,
                        &binary,
                        &design,
                        &base,
                        cell,
                        &scene_name,
                        *size,
                        *theme,
                        Duration::from_millis(quiet_ms),
                        &keys,
                        &dir,
                    )?;
                    let (cols, rows) = size.cells();
                    let gates = match frame.gates.by_gate().as_slice() {
                        [] => "gates clean".to_string(),
                        counts => counts.iter().map(|(g, n)| format!("{g} {n}")).collect::<Vec<_>>().join(", "),
                    };
                    println!(
                        "{} {} {:<5} {cols}×{rows}  {} cells checked  {gates}",
                        frame.scene, frame.size, frame.theme.to_string(), frame.checked
                    );
                    for v in frame.gates.violations.iter().take(4) {
                        println!("    ({},{}) {} — {}", v.row, v.col, v.gate, v.detail);
                    }
                    if frame.gates.violations.len() > 4 {
                        println!("    … {} more, see the .gates.json", frame.gates.violations.len() - 4);
                    }
                    for note in &frame.gates.applied {
                        println!("    exempt: {note}");
                    }
                }
            }
            println!("\nrun directory: {}", dir.display());
            Ok(())
        }
    }
}
