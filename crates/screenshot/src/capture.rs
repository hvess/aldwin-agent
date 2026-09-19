//! Measuring the cell, and taking one frame.
//!
//! Both go through the proxy, which is what removed the two guesses the
//! earlier version of this file had to make. The cell is read from the pty
//! foot sizes rather than from a shell running `stty`; and a frame is taken
//! when the app stops drawing rather than after a hopeful interval.
//!
//! Every capture leaves two artefacts side by side: the PNG a human reads,
//! and the declared cell grid the gates read. They are cross-checked against
//! each other before either is trusted.

use std::io::{Error, ErrorKind, Result};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread::sleep;
use std::time::{Duration, Instant};

use crate::baseline::Baseline;
use crate::compositor::Compositor;
use crate::design::Design;
use crate::{gates, regions};
use crate::geometry::{Cell, Size, Theme};
use crate::proxy::{foot_command, verify_against_pixels, Proxy};
use crate::pty::Pty;
use crate::{fake, png, scene};

/// Measure foot's cell for the pinned font.
///
/// A large output and integer division: foot centres any remainder, so 800px
/// of 19px rows is 42 rows and 2px of slack, and `800 / 42` recovers 19 where
/// a float would not.
///
/// The number this returns is about the *pinned* config, not the machine's
/// taste. Measured with the developer's `foot.ini` in play the same font at
/// the same size gives 8×19; with `--config=/dev/null` it gives 8×18. That
/// gap is why the harness measures rather than remembers.
pub fn measure_cell(comp: &Compositor, font: &str) -> Result<Cell> {
    const W: u32 = 1200;
    const H: u32 = 800;

    comp.clear()?;
    comp.set_mode(W, H)?;

    let display = Pty::open()?;
    comp.exec(&foot_command(font, display.slave_path()))?;

    let deadline = Instant::now() + Duration::from_secs(15);
    let mut previous = (0, 0);
    let cell = loop {
        let now = display.winsize()?;
        if now.0 > 0 && now.1 > 0 && now == previous {
            break Cell { w: W / now.0 as u32, h: H / now.1 as u32 };
        }
        previous = now;
        if Instant::now() > deadline {
            return Err(Error::new(ErrorKind::TimedOut, "foot never sized its pty; is the font installed?"));
        }
        sleep(Duration::from_millis(100));
    };

    comp.clear()?;
    Ok(cell)
}

pub struct Frame {
    pub path:      PathBuf,
    /// A copy with every violation outlined, written only when there are any.
    pub annotated: Option<PathBuf>,
    pub grid:    PathBuf,
    pub scene:   String,
    pub size:    Size,
    pub theme:   Theme,
    /// How many cells the parser and the frame were checked to agree on.
    pub checked: usize,
    pub gates:   gates::Report,
}

/// Capture one frame: one scene, one size, one theme.
///
/// The compositor is cleared on every exit path, not only the happy one. An
/// early error used to leave foot on the output, where it both held the
/// proxy's pump threads open on a pty nobody would close and shared the frame
/// with the next capture — which the surface count would then report as a
/// failure belonging to the wrong scene.
#[allow(clippy::too_many_arguments)]
pub fn capture(
    comp: &Compositor,
    binary: &Path,
    design: &Design,
    baseline: &Baseline,
    cell: Cell,
    scene_name: &str,
    size: Size,
    theme: Theme,
    quiet_for: Duration,
    keys: &[Vec<u8>],
    run_dir: &Path,
) -> Result<Frame> {
    let outcome = take_frame(comp, binary, design, baseline, cell, scene_name, size, theme, quiet_for, keys, run_dir);
    let _ = comp.clear();
    outcome
}

#[allow(clippy::too_many_arguments)]
fn take_frame(
    comp: &Compositor,
    binary: &Path,
    design: &Design,
    baseline: &Baseline,
    cell: Cell,
    scene_name: &str,
    size: Size,
    theme: Theme,
    quiet_for: Duration,
    keys: &[Vec<u8>],
    run_dir: &Path,
) -> Result<Frame> {
    let (cols, rows) = size.cells();
    let (px_w, px_h) = size.pixels(cell);
    let work = run_dir.join(format!("{scene_name}-{size}-{theme}"));
    std::fs::create_dir_all(&work)?;
    let mut script = scene::script(scene_name)?;

    // The provider is a real socket the app really talks to, so it has to be
    // listening before the config that points at it is written — the port is
    // ephemeral. A multi-threaded runtime, not a current-thread one: nothing
    // here calls `block_on`, so the accept loop needs a worker of its own to
    // make progress at all. It must outlive the capture: dropping it early
    // takes the provider down mid-conversation, and the app reports a
    // connection error it is entirely right about.
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build()?;
    let guard = runtime.enter();
    let server = fake::spawn(std::mem::take(&mut script.replies));
    let endpoint = fake::endpoint(&server);
    drop(guard);

    let prepared = scene::seed(&script, theme, &work, &endpoint)?;
    let typed = if keys.is_empty() { prepared.keys.clone() } else { keys.to_vec() };

    comp.clear()?;
    comp.set_mode(px_w, px_h)?;

    let mut command = Command::new(binary);
    command
        .current_dir(&prepared.cwd)
        .env("HOME", &prepared.home)
        .env("TERM", "foot")
        .env("MJOLNIR_SHOT_KEY", "not-a-real-key-the-fake-never-checks");

    let mut proxy = Proxy::start(comp, &baseline.font, cols as u16, rows as u16, command)?;
    proxy.wait_quiet(quiet_for, Duration::from_secs(20))?;
    if !proxy.app_running() {
        return Err(Error::other(format!("{} exited before it could be captured", binary.display())));
    }

    // Keys go in one at a time, each waited out, so a scene arrives at the
    // state it names rather than at whatever a burst happened to produce.
    for key in &typed {
        proxy.send_key(key, quiet_for, Duration::from_secs(20))?;
    }

    let surfaces = comp.surface_count()?;
    if surfaces != 1 {
        return Err(Error::other(format!(
            "{surfaces} surfaces on the output, expected exactly 1 — something is sharing the frame"
        )));
    }

    let path = run_dir.join(format!("{scene_name}-{size}-{theme}.png"));
    let _ = std::fs::remove_file(&path);
    comp.exec(&format!("grim '{}'", path.display()))?;
    wait_for_png(&path)?;

    let (w, h) = png::size(&path)?;
    if (w, h) != (px_w, px_h) {
        return Err(Error::other(format!("frame is {w}×{h}, asked for {px_w}×{px_h} — capture invariant failed")));
    }

    // Written before the cross-check, not after: when the two disagree the
    // grid is the evidence for which of them is wrong.
    let grid = proxy.grid();
    let grid_path = run_dir.join(format!("{scene_name}-{size}-{theme}.txt"));
    std::fs::write(&grid_path, grid.text())?;

    let checked = verify_against_pixels(&grid, &path, cell)?;
    if checked < (cols * rows / 10) as usize {
        return Err(Error::other(format!(
            "only {checked} cells could be cross-checked against the frame — too few to trust the grid"
        )));
    }

    // The map is written out beside the frame: it is a judgement the harness
    // made about the app's own layout, and it underpins two gates, so it has
    // to be visible rather than implicit.
    let map = regions::derive(&grid, theme, design);
    std::fs::write(run_dir.join(format!("{scene_name}-{size}-{theme}.regions.txt")), map.render())?;

    // Gates run on the declared cells, never on the picture, and they do not
    // stop a run: one iteration should show the whole picture rather than
    // send a fix loop from one symptom to the next.
    let report = gates::run(&grid, &map, theme, design, baseline);
    let gates_path = run_dir.join(format!("{scene_name}-{size}-{theme}.gates.json"));
    std::fs::write(&gates_path, serde_json::to_string_pretty(&report)?)?;

    // The clean frame stays the evidence; the marked-up copy is the
    // explanation, and only exists when there is something to explain.
    let annotated = if report.violations.is_empty() {
        None
    } else {
        let marks: Vec<(u16, u16)> = report.violations.iter().map(|v| (v.row, v.col)).collect();
        let marked = run_dir.join(format!("{scene_name}-{size}-{theme}.marked.png"));
        png::annotate(&path, &marked, &marks, cell.w, cell.h)?;
        Some(marked)
    };

    drop(proxy);

    Ok(Frame { path, annotated, grid: grid_path, scene: scene_name.to_string(), size, theme, checked, gates: report })
}

fn wait_for_png(path: &Path) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut stable = 0;
    let mut last = 0;
    while Instant::now() < deadline {
        if let Ok(meta) = std::fs::metadata(path) {
            let len = meta.len();
            if len > 0 && len == last {
                stable += 1;
                if stable >= 2 {
                    return Ok(());
                }
            } else {
                stable = 0;
            }
            last = len;
        }
        sleep(Duration::from_millis(100));
    }
    Err(Error::new(ErrorKind::TimedOut, format!("grim wrote no frame at {}", path.display())))
}
