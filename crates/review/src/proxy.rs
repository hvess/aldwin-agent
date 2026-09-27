//! The pty proxy: foot on one pty, the app on another, the harness pumping
//! bytes between them.
//!
//! It yields the declared cell grid (what stage 8 reads positions from),
//! quiesce, and key input. foot still renders and answers the app's
//! capability queries itself; only keypresses are synthesised.
//!
//! Startup order is load-bearing: the app starts only after foot has sized its
//! pty, on a pty already at that size, so it never sees foot's initial 80×24.

use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::compositor::Compositor;
use crate::geometry::Cell as CellSize;
use crate::png;
use crate::pty::{self, Pty};
use crate::vt::{Color, Grid, Vt};
use crate::{Error, Result};

/// foot and the app on their own ptys, the app's output parsed on the way
/// through. Dropping it kills the app.
#[derive(Debug)]
pub struct Proxy {
    app: Pty,
    vt: Arc<Mutex<Vt>>,
    idle: Arc<AtomicU64>,
    child: Child,
    start: Instant,
}

/// The `swaymsg exec` command line for foot on the harness's pty `pts`.
///
/// `--config=/dev/null` keeps the developer's `foot.ini` out.
/// `cursor.unfocused-style=unchanged` is needed because headless foot never
/// has focus and would otherwise draw the app's bar caret as a hollow block.
pub(crate) fn foot_command(font: &str, pts: &Path) -> String {
    format!(
        "foot --config=/dev/null -o main.pad=0x0 -o cursor.unfocused-style=unchanged -o {} --pty={}",
        shell_quote(&format!("main.font={font}")),
        shell_quote(&pts.display().to_string())
    )
}

/// `s` single-quoted as one word for the shell `swaymsg exec` runs.
pub(crate) fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

impl Proxy {
    /// Starts foot, waits for it to size its pty, then starts the app on a pty
    /// already at that size.
    ///
    /// # Errors
    ///
    /// When a pty cannot be opened, sized or cloned, foot cannot be launched,
    /// foot has not sized its pty to `cols` × `rows` within fifteen seconds,
    /// or the app cannot be spawned.
    pub fn start(
        comp: &Compositor,
        font: &str,
        cols: u16,
        rows: u16,
        mut app_command: Command,
    ) -> Result<Self> {
        let display = Pty::open()?;
        let app = Pty::open()?;

        comp.exec(&foot_command(font, display.slave_path()))?;

        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if display.winsize()? == (cols, rows) {
                break;
            }
            if Instant::now() > deadline {
                let (c, r) = display.winsize()?;
                return Err(Error::Capture(format!(
                    "foot sized its pty {c}×{r}, expected {cols}×{rows} — output mode and cell disagree"
                )));
            }
            thread::sleep(Duration::from_millis(50));
        }

        app.set_winsize(cols, rows)?;
        let child = app.attach_as_controlling(&mut app_command)?;

        let vt = Arc::new(Mutex::new(Vt::new(cols, rows)));
        let idle = Arc::new(AtomicU64::new(0));
        let start = Instant::now();

        // app → parser → foot: foot renders exactly the bytes the grid is
        // parsed from.
        spawn_pump(
            app.try_clone_master()?,
            display.try_clone_master()?,
            Some(Observer {
                vt: vt.clone(),
                idle: idle.clone(),
                start,
                last_print: 0,
            }),
        );
        // foot → app: foot's replies to the app's capability queries.
        spawn_pump(display.try_clone_master()?, app.try_clone_master()?, None);

        Ok(Proxy {
            app,
            vt,
            idle,
            child,
            start,
        })
    }

    /// Blocks until the visible grid has not changed for `idle` (the quiesce
    /// rule; never replace it with a fixed settle interval).
    ///
    /// # Errors
    ///
    /// When `timeout` passes first, saying whether the app drew nothing or
    /// never settled.
    pub fn wait_quiet(&self, idle: Duration, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        loop {
            let last = self.idle.load(Ordering::Relaxed);
            let since = self.start.elapsed().as_millis() as u64 - last;
            if last > 0 && since >= idle.as_millis() as u64 {
                return Ok(());
            }
            if Instant::now() > deadline {
                return Err(Error::Capture(
                    if last == 0 {
                        "the app drew nothing at all"
                    } else {
                        "the frame never settled — something on screen is still changing (a spinner? a counter?)"
                    }
                    .into(),
                ));
            }
            thread::sleep(Duration::from_millis(25));
        }
    }

    /// Whether the visible state moved before `timeout`.
    ///
    /// Used to align with the caret's 1.05 s stepped blink: a picture and a
    /// grid taken on either side of a blink edge disagree at one cell, so
    /// capture waits for the edge and then has half a period for both.
    pub fn wait_for_change(&self, timeout: Duration) -> bool {
        let before = self.idle.load(Ordering::Relaxed);
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.idle.load(Ordering::Relaxed) != before {
                return true;
            }
            thread::sleep(Duration::from_millis(10));
        }
        false
    }

    /// Sends bytes as typed input; foot's key encoding is bypassed.
    ///
    /// # Errors
    ///
    /// When the write to the app's pty fails.
    pub fn send(&self, bytes: &[u8]) -> Result<()> {
        self.app.write_all(bytes)
    }

    /// Sends a key and waits for the app to finish reacting to that key.
    ///
    /// Waits for the change mark to move (up to a 2 s grace, for a key that
    /// draws nothing) before `wait_quiet`: an already-quiet app would
    /// otherwise return before the key was read.
    ///
    /// # Errors
    ///
    /// When the send fails, or as [`Proxy::wait_quiet`].
    pub fn send_key(&self, bytes: &[u8], quiet: Duration, timeout: Duration) -> Result<()> {
        let before = self.idle.load(Ordering::Relaxed);
        self.send(bytes)?;

        let grace = Instant::now() + Duration::from_secs(2);
        while self.idle.load(Ordering::Relaxed) == before && Instant::now() < grace {
            thread::sleep(Duration::from_millis(10));
        }
        self.wait_quiet(quiet, timeout)
    }

    /// A copy of the screen as the app has drawn it so far.
    ///
    /// # Panics
    ///
    /// When a pump thread panicked while holding the parser's lock.
    pub fn grid(&self) -> Grid {
        self.vt.lock().expect("vt lock poisoned").grid().clone()
    }

    /// Whether the app is still alive; an exited app is reaped here.
    pub fn app_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The app-to-foot pump's parser and change mark. `idle` holds milliseconds
/// since `start` at the last visible change, 0 before the first.
struct Observer {
    vt: Arc<Mutex<Vt>>,
    idle: Arc<AtomicU64>,
    start: Instant,
    last_print: u64,
}

fn spawn_pump(mut from: File, mut to: File, mut observe: Option<Observer>) {
    // `ALDWIN_SHOT_TRACE=<path>` appends the app's byte stream to a file, for
    // debugging a parser/frame disagreement.
    let mut trace = observe
        .is_some()
        .then(|| std::env::var_os("ALDWIN_SHOT_TRACE"))
        .flatten()
        .and_then(|p| {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(p)
                .ok()
        });
    thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match pty::read(&mut from, &mut buf) {
                Ok(0) | Err(_) => return,
                Ok(n) => {
                    if let Some(f) = trace.as_mut() {
                        let _ = f.write_all(&buf[..n]);
                    }
                    if let Some(observer) = observe.as_mut() {
                        let mut vt = observer.vt.lock().expect("vt lock poisoned");
                        vt.feed(&buf[..n]);
                        // Mark only a changed fingerprint, never bare bytes:
                        // ratatui emits a frame envelope every tick even when
                        // no cell differs.
                        let now = vt.grid().fingerprint();
                        if now != observer.last_print {
                            observer.last_print = now;
                            observer.idle.store(
                                observer.start.elapsed().as_millis() as u64,
                                Ordering::Relaxed,
                            );
                        }
                    }
                    if to.write_all(&buf[..n]).is_err() {
                        return;
                    }
                }
            }
        }
    });
}

/// Cross-checks the parser against the picture: every non-caret space the
/// parser gives an RGB ground must show exactly that colour in the frame.
///
/// Returns how many cells were checked, so a caller can refuse a frame where
/// too few were.
///
/// # Errors
///
/// When the frame cannot be decoded, or any checked cell's pixel differs
/// from the ground the parser recorded for it.
pub fn verify_against_pixels(grid: &Grid, frame: &Path, cell: CellSize) -> Result<usize> {
    let image = png::decode(frame)?;
    let mut checked = 0;
    for (row, col, c) in grid.cells() {
        let Color::Rgb(r, g, b) = c.effective().1 else {
            continue;
        };
        // The caret's bar cursor covers the sampled pixel.
        if c.ch != ' ' || grid.caret() == Some((row, col)) {
            continue;
        }
        // Inset a pixel: a cell's edge can carry a neighbour's antialiasing.
        let x = col as u32 * cell.w + 1;
        let y = row as u32 * cell.h + 1;
        if x + 1 >= image.width || y + 1 >= image.height {
            continue;
        }
        let got = image.pixel(x, y);
        if got != (r, g, b) {
            return Err(Error::Capture(format!(
                "parser and frame disagree at cell ({row},{col}): parsed ground #{r:02x}{g:02x}{b:02x}, frame shows #{:02x}{:02x}{:02x}",
                got.0, got.1, got.2
            )));
        }
        checked += 1;
    }
    Ok(checked)
}
