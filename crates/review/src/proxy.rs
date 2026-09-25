//! The pty proxy — foot on one pty, the app on another, the harness between.
//!
//! This is what makes the harness more than a camera. Sitting in the stream
//! gives three things no screenshot can:
//!
//! * **the declared cell grid** — the character and the foreground and
//!   background the app asked for, per cell, which is the declared
//!   grid stage 5 reads positions from;
//! * **quiesce** — the app going quiet is observable, so a frame is captured
//!   when it is finished rather than after a hopeful interval;
//! * **input** — keystrokes go in as bytes.
//!
//! What it does *not* fake is the terminal. foot still renders, and its
//! replies to the app's capability queries are foot's own: a query written by
//! the app is forwarded to foot, and foot's answer is forwarded back. Only
//! synthesised keypresses are the harness's invention, which is the one thing
//! this arrangement cannot vouch for.
//!
//! Startup order matters and is the reason the pre-resize race disappears
//! here. foot sets the window size on its pty once its window is configured;
//! the harness waits for that, copies it to the app's pty, and only then
//! starts the app. The app is therefore born at its final size and never
//! sees the 80×24 foot would otherwise have handed it.

use std::fs::File;
use std::io::{Error, ErrorKind, Result, Write};
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

pub struct Proxy {
    app: Pty,
    vt: Arc<Mutex<Vt>>,
    idle: Arc<AtomicU64>,
    child: Child,
    start: Instant,
}

/// foot, pinned, attached to a pty the harness owns rather than one it makes.
/// Nothing is read from the developer's own `foot.ini`: their font, padding
/// and colours are theirs, not the test's.
///
/// The headless compositor has no keyboard, so foot never has focus and
/// would draw its cursor — the app's caret — as an unfocused hollow block.
/// `unfocused-style=unchanged` draws it as the focused terminal the
/// developer types into does: the bar the app asked for.
pub(crate) fn foot_command(font: &str, pts: &Path) -> String {
    format!(
        "foot --config=/dev/null -o main.pad=0x0 -o cursor.unfocused-style=unchanged -o {} --pty={}",
        shell_quote(&format!("main.font={font}")),
        shell_quote(&pts.display().to_string())
    )
}

/// One word for the shell `swaymsg exec` hands its command to.
pub(crate) fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

impl Proxy {
    /// Start foot, wait for it to size its pty, then start the app on a pty
    /// already at that size.
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
                return Err(Error::new(
                    ErrorKind::TimedOut,
                    format!("foot sized its pty {c}×{r}, expected {cols}×{rows} — output mode and cell disagree"),
                ));
            }
            thread::sleep(Duration::from_millis(50));
        }

        app.set_winsize(cols, rows)?;
        let child = app.attach_as_controlling(&mut app_command)?;

        let vt = Arc::new(Mutex::new(Vt::new(cols, rows)));
        let idle = Arc::new(AtomicU64::new(0));
        let start = Instant::now();

        // app → parser → foot. Every byte the app writes is both the thing
        // foot renders and the thing the grid is parsed from, so there is
        // no way for the two to disagree about what was sent.
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
        // foot → app: keystrokes, and foot's replies to the app's own
        // capability queries.
        spawn_pump(display.try_clone_master()?, app.try_clone_master()?, None);

        Ok(Proxy {
            app,
            vt,
            idle,
            child,
            start,
        })
    }

    /// Block until the app has emitted nothing for `idle`.
    ///
    /// This is the spec's quiesce rule, and it replaces a settle interval:
    /// the spinner stops when there is no turn running, so a frame taken here
    /// is one the app considers finished rather than one caught mid-draw.
    pub fn wait_quiet(&self, idle: Duration, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        loop {
            let last = self.idle.load(Ordering::Relaxed);
            let since = self.start.elapsed().as_millis() as u64 - last;
            if last > 0 && since >= idle.as_millis() as u64 {
                return Ok(());
            }
            if Instant::now() > deadline {
                return Err(Error::new(
                    ErrorKind::TimedOut,
                    if last == 0 {
                        "the app drew nothing at all"
                    } else {
                        "the frame never settled — something on screen is still changing (a spinner? a counter?)"
                    },
                ));
            }
            thread::sleep(Duration::from_millis(25));
        }
    }

    /// Block until the visible state moves, or `timeout` passes without it.
    ///
    /// The caret blinks at the design's 1.05s, stepped, so an app at rest
    /// on a field is quiet for most of a second and then not: `wait_quiet`
    /// still settles between blinks, but a picture and a grid taken either
    /// side of the edge disagree at exactly one cell. Waiting for the edge
    /// first leaves half a period to take both inside. A screen with no
    /// caret never moves; that case falls through the timeout and costs
    /// only that.
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

    /// Send bytes as if typed. The harness chooses these, so they test the
    /// app's handling and not foot's key encoding — the one thing this
    /// arrangement cannot vouch for.
    pub fn send(&self, bytes: &[u8]) -> Result<()> {
        self.app.write_all(bytes)
    }

    /// Press a key and wait for the app to finish reacting to **that key**.
    ///
    /// Waiting for quiet alone is not enough and the difference is not
    /// subtle: an app that has been idle since startup is *already* quiet, so
    /// a bare `wait_quiet` returns before the keystroke has been read at all.
    /// The first capture written this way screenshotted the previous screen
    /// while the parser had already consumed the next one — caught, not by
    /// review, but by `verify_against_pixels` refusing to reconcile the two.
    ///
    /// So: note the last-output mark, send, wait for it to move, then wait
    /// for silence. A key that legitimately draws nothing falls through the
    /// grace period and costs only that.
    pub fn send_key(&self, bytes: &[u8], quiet: Duration, timeout: Duration) -> Result<()> {
        let before = self.idle.load(Ordering::Relaxed);
        self.send(bytes)?;

        let grace = Instant::now() + Duration::from_secs(2);
        while self.idle.load(Ordering::Relaxed) == before && Instant::now() < grace {
            thread::sleep(Duration::from_millis(10));
        }
        self.wait_quiet(quiet, timeout)
    }

    pub fn grid(&self) -> Grid {
        self.vt.lock().expect("vt lock poisoned").grid().clone()
    }

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

/// What the app-to-terminal direction carries besides bytes: the parser, the
/// last-change mark, and the fingerprint that decides whether the *visible*
/// state moved.
struct Observer {
    vt: Arc<Mutex<Vt>>,
    idle: Arc<AtomicU64>,
    start: Instant,
    last_print: u64,
}

fn spawn_pump(mut from: File, mut to: File, mut observe: Option<Observer>) {
    // `ALDWIN_SHOT_TRACE=<path>` tees the app's byte stream to a file. When
    // the parser and the frame disagree, this is the only place the answer
    // can be: both of them are downstream of these bytes.
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
                        // The mark moves only when the *visible state* moves.
                        // Bytes alone are not evidence of change: an idle app
                        // repaints on every tick and ratatui emits the frame
                        // envelope regardless of whether any cell differs.
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

/// Cross-check the parser against the picture.
///
/// A subtly wrong parser is the same class of defect as the wrong cell size:
/// it produces a grid that looks entirely plausible, and the judge then
/// reports confidently about cells the app never drew. So every cell the
/// parser calls "a space on a known ground" must be a flat block of exactly
/// that colour in the frame. Where they disagree, one of them is lying and
/// the run is not scoreable.
pub fn verify_against_pixels(grid: &Grid, frame: &Path, cell: CellSize) -> Result<usize> {
    let image = png::decode(frame)?;
    let mut checked = 0;
    for (row, col, c) in grid.cells() {
        let Color::Rgb(r, g, b) = c.effective().1 else {
            continue;
        };
        // The caret's cell carries the terminal's bar cursor over its
        // ground, which is exactly the pixel sampled below.
        if c.ch != ' ' || grid.caret() == Some((row, col)) {
            continue;
        }
        // Inset by a pixel: a cell's own edge can carry the neighbouring
        // glyph's antialiasing, and that is not what is under test here.
        let x = col as u32 * cell.w + 1;
        let y = row as u32 * cell.h + 1;
        if x + 1 >= image.width || y + 1 >= image.height {
            continue;
        }
        let got = image.pixel(x, y);
        if got != (r, g, b) {
            return Err(Error::other(format!(
                "parser and frame disagree at cell ({row},{col}): parsed ground #{r:02x}{g:02x}{b:02x}, frame shows #{:02x}{:02x}{:02x}",
                got.0, got.1, got.2
            )));
        }
        checked += 1;
    }
    Ok(checked)
}
