//! `mjolnir-review` — the deterministic stages of the review loop.
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
use mjolnir_review::capture::{capture, measure_cell};
use mjolnir_review::geometry::{Size, Theme};
use mjolnir_review::{scene, stages, tokens, Baseline, Compositor};

#[derive(Parser)]
#[command(about = "The review loop's deterministic stages")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run stages 1–4 and report. Stage 5 is the skill's.
    Review {
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
    let dir = parent.join(format!("run-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Capture every scene into a fresh directory, for stage 5 to look at.
///
/// No assertions: everything that reads declared cells is a hermetic test in
/// `crates/tui` now. This exists to make pictures, which is the one thing a
/// `TestBackend` cannot do.
fn capture_all(root: &Path, base: &Baseline, theme: Option<Theme>, quiet_ms: u64) -> std::io::Result<PathBuf> {
    let dir = frames_dir(root)?;
    let binary = root.join("target/debug/mjolnir");
    if !binary.exists() {
        return Err(std::io::Error::other(format!("{} does not exist — cargo build first", binary.display())));
    }
    let comp = Compositor::start(&dir)?;
    let cell = measure_cell(&comp, &base.font)?;
    if cell != base.cell {
        return Err(std::io::Error::other(format!(
            "cell {}×{} disagrees with the baseline's {}×{} — run `measure --record` if the font or machine changed",
            cell.w, cell.h, base.cell.w, base.cell.h
        )));
    }
    let themes: Vec<Theme> = theme.map(|t| vec![t]).unwrap_or_else(|| Theme::ALL.to_vec());
    for name in scene::IMPLEMENTED {
        for size in Size::ALL {
            for theme in themes.iter().copied() {
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
            let binary = root.join("target/debug/mjolnir");
            let comp = Compositor::start(&dir)?;
            let cell = measure_cell(&comp, &base.font)?;
            let names: Vec<&str> = match &one {
                Some(name) => vec![name.as_str()],
                None => scene::IMPLEMENTED.to_vec(),
            };
            for name in names {
                for s in size.map(|s| vec![s]).unwrap_or_else(|| Size::ALL.to_vec()) {
                    for t in theme.map(|t| vec![t]).unwrap_or_else(|| Theme::ALL.to_vec()) {
                        let frame = capture(&comp, &binary, &base, cell, name, s, t, Duration::from_millis(quiet_ms), &[], &dir)?;
                        println!("{name} {s} {t}  {}", frame.path.display());
                    }
                }
            }
            println!("\nframes {}", dir.display());
            Ok(())
        }

        Command::Review { no_capture, theme, quiet_ms } => {
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
                    detail: format!("{detail}\n\nRegenerate with:\n    cargo run -p mjolnir-review -- tokens --write"),
                },
            });
            outcomes.extend(stages::frames(&root)?);

            let frames = match no_capture {
                true => None,
                false => Some(capture_all(&root, &base, theme, quiet_ms)?),
            };

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

            println!();
            match &frames {
                Some(dir) => println!("frames for stage 5: {}", dir.display()),
                None => println!("no frames captured (--no-capture); stage 5 needs them"),
            }
            if failed > 0 {
                return Err(std::io::Error::other(format!("{failed} of {} deterministic stages failed", outcomes.len())));
            }
            println!("stages 1–4 clean, and every one of them hermetic. Stage 5 is the skill's:");
            println!("spawn the judge against those frames.");
            Ok(())
        }
    }
}
