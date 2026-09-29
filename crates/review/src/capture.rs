//! Measuring the cell, and taking one frame.
//!
//! A capture leaves the PNG and, beside it, the declared cell grid stage 8
//! reads positions from; the two are cross-checked before either is trusted.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread::sleep;
use std::time::{Duration, Instant};

use crate::baseline::Baseline;
use crate::compositor::Compositor;
use crate::geometry::{Cell, Size, Theme};
use crate::proxy::{foot_command, shell_quote, verify_against_pixels, Proxy};
use crate::pty::Pty;
use crate::{fake, png, scene, Error, Result};

/// Measures foot's cell for the pinned font and config.
///
/// Integer division is required: foot centres the remainder, so 800 px of
/// 19 px rows is 42 rows, and `800 / 42` recovers 19 where a float would not.
/// The result depends on the config (the same font measured 8×19 under a
/// developer's `foot.ini`, 8×18 under `--config=/dev/null`).
///
/// # Errors
///
/// When the compositor cannot be cleared or resized, the pty cannot be opened
/// or queried, foot cannot be launched, or foot has not sized its pty within
/// fifteen seconds (usually a missing font).
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
            break Cell {
                w: W / now.0 as u32,
                h: H / now.1 as u32,
            };
        }
        previous = now;
        if Instant::now() > deadline {
            return Err(Error::Capture(
                "foot never sized its pty; is the font installed?".into(),
            ));
        }
        sleep(Duration::from_millis(100));
    };

    comp.clear()?;
    Ok(cell)
}

/// Captures one scene at one size and theme. Returns the PNG's path; the
/// declared grid is beside it with a `.txt` extension.
///
/// Clears the compositor on every exit path, errors included, so a leftover
/// foot never shares the next scene's frame.
///
/// # Errors
///
/// When the scene is unknown or cannot be seeded, the fake provider or the
/// proxy cannot start, the app exits before capture, the output holds other
/// than one surface, no still frame arrives in five
/// tries, the PNG is the wrong size, too few cells can be cross-checked, or
/// any file or compositor call fails.
#[allow(clippy::too_many_arguments)]
pub fn capture(
    comp: &Compositor,
    binary: &Path,
    baseline: &Baseline,
    cell: Cell,
    scene_name: &str,
    size: Size,
    theme: Theme,
    quiet_for: Duration,
    keys: &[Vec<u8>],
    run_dir: &Path,
) -> Result<PathBuf> {
    let outcome = take_frame(
        comp, binary, baseline, cell, scene_name, size, theme, quiet_for, keys, run_dir,
    );
    let _ = comp.clear();
    outcome
}

#[allow(clippy::too_many_arguments)]
fn take_frame(
    comp: &Compositor,
    binary: &Path,
    baseline: &Baseline,
    cell: Cell,
    scene_name: &str,
    size: Size,
    theme: Theme,
    quiet_for: Duration,
    keys: &[Vec<u8>],
    run_dir: &Path,
) -> Result<PathBuf> {
    let (cols, rows) = size.cells();
    let (px_w, px_h) = size.pixels(cell);
    let work = run_dir.join(format!("{scene_name}-{size}-{theme}"));
    std::fs::create_dir_all(&work)?;
    let mut script = scene::script(scene_name)?;

    // The fake must listen before the config is seeded: its port is
    // ephemeral. Multi-threaded, not current-thread: nothing calls
    // `block_on`, so the accept loop needs its own worker. `runtime` must
    // outlive the capture or the provider dies mid-conversation.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()?;
    let guard = runtime.enter();
    let server = fake::spawn(std::mem::take(&mut script.replies));
    let endpoint = fake::endpoint(&server);
    drop(guard);

    let prepared = scene::seed(&script, theme, &work, &endpoint)?;
    let typed = if keys.is_empty() {
        prepared.keys.clone()
    } else {
        keys.to_vec()
    };

    comp.clear()?;
    comp.set_mode(px_w, px_h)?;

    let mut command = Command::new(binary);
    command
        .current_dir(&prepared.cwd)
        .env("HOME", &prepared.home)
        .env("TERM", "foot")
        .env("ALDWIN_SHOT_KEY", "not-a-real-key-the-fake-never-checks");

    let mut proxy = Proxy::start(comp, &baseline.font, cols as u16, rows as u16, command)?;
    proxy.wait_quiet(quiet_for, Duration::from_secs(20))?;
    if !proxy.app_running() {
        return Err(Error::Capture(format!(
            "{} exited before it could be captured",
            binary.display()
        )));
    }

    // One key at a time, each waited out; a burst lands in a nondeterministic
    // state.
    for key in &typed {
        proxy.send_key(key, quiet_for, Duration::from_secs(20))?;
    }

    let surfaces = comp.surface_count()?;
    if surfaces != 1 {
        return Err(Error::Capture(format!(
            "{surfaces} surfaces on the output, expected exactly 1 — something is sharing the frame"
        )));
    }

    // Picture and grid must fall between two changes on screen
    // (`Proxy::wait_for_change`): under the seeded reduced motion the caret
    // holds and only a working line's timer moves, once a second. An
    // unchanged grid after the shot proves it, otherwise retake on the next
    // edge.
    let path = run_dir.join(format!("{scene_name}-{size}-{theme}.png"));
    let mut grid = None;
    for _ in 0..5 {
        let _ = std::fs::remove_file(&path);
        proxy.wait_for_change(Duration::from_millis(1300));
        let before = proxy.grid();
        comp.exec(&format!(
            "grim {}",
            shell_quote(&path.display().to_string())
        ))?;
        wait_for_png(&path)?;
        if proxy.grid().fingerprint() == before.fingerprint() {
            grid = Some(before);
            break;
        }
    }
    let Some(grid) = grid else {
        return Err(Error::Capture(
            "no still frame in five tries — something on screen moves faster than once a second"
                .into(),
        ));
    };

    let (w, h) = png::size(&path)?;
    if (w, h) != (px_w, px_h) {
        return Err(Error::Capture(format!(
            "frame is {w}×{h}, asked for {px_w}×{px_h} — capture invariant failed"
        )));
    }

    // Written before the cross-check: on a mismatch the grid is the evidence.
    let grid_path = run_dir.join(format!("{scene_name}-{size}-{theme}.txt"));
    std::fs::write(&grid_path, grid.text())?;

    let checked = verify_against_pixels(&grid, &path, cell)?;
    if checked < (cols * rows / 10) as usize {
        return Err(Error::Capture(format!(
            "only {checked} cells could be cross-checked against the frame — too few to trust the grid"
        )));
    }

    // No design assertions here: checks on declared cells belong in
    // `crates/tui/tests/render_snapshot.rs` (aldwin-review.md Decision 8).
    drop(proxy);
    Ok(path)
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
    Err(Error::Capture(format!(
        "grim wrote no frame at {}",
        path.display()
    )))
}
