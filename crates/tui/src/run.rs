use std::io;
use std::io::{BufWriter, Write};
use std::time::{Duration, Instant};

use aldwin_core::{Command, Event};
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::cursor::{Hide, SetCursorStyle, Show};
use ratatui::crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, Event as CtEvent,
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, BeginSynchronizedUpdate, EndSynchronizedUpdate,
    EnterAlternateScreen, LeaveAlternateScreen, SetTitle,
};
use ratatui::crossterm::{execute, queue, ExecutableCommand};
use ratatui::style::Color;
use ratatui::Terminal;
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TryRecvError;

use crate::app::{App, CommandChoice, ProviderChoice};
use crate::palette::Theme;
use crate::resume::SessionChoice;
use crate::ui;

/// The terminal this session writes to.
///
/// Keep the `BufWriter`: `io::Stdout` is line-buffered at ~1KB and ratatui
/// emits no newlines, so a frame went out in ~30 writes and tore.
type Out = Terminal<CrosstermBackend<BufWriter<io::Stdout>>>;

/// Holds a full repaint of a large terminal, so a frame is one write.
const OUT_BUFFER: usize = 1 << 20;

/// What the session knows about where it runs, read by aldwin-cli; this
/// crate reads no files.
#[derive(Debug)]
pub struct SessionProvider {
    /// The working directory's name.
    pub project: String,
    /// The checked-out git branch, if any.
    pub branch: Option<String>,
    /// The `/` menu's rows, in drawn order; aldwin-cli owns the commands.
    pub commands: Vec<CommandChoice>,
    /// Display rows, in catalogue order. Empty: bare `/model` goes to
    /// aldwin-cli, which reports rather than asks.
    pub catalogue: Vec<ProviderChoice>,
    /// The catalogue id `provider.yaml` resolves to; `None` for an unknown
    /// endpoint or nothing configured.
    pub current_provider: Option<String>,
    /// Past sessions for bare `/resume`, newest first, excluding the
    /// current one; already formatted.
    pub sessions: Vec<SessionChoice>,
}

/// Runs the TUI to completion, multiplexing input and core events on one
/// `tokio::select!` (aldwin-tui.md, Pitfalls), and always restores the
/// terminal through `TerminalGuard`, on error and panic too.
///
/// Mouse capture stays off in the conversation so the terminal's own text
/// selection works; a terminal cannot give the mouse to both. The wheel
/// scrolls through alternate scroll mode instead. Capture is on only while
/// a review is open (ADR 0010, `sync_mouse`).
///
/// Every mode is best-effort. Bracketed paste keeps a pasted newline from
/// submitting. The Kitty disambiguation flag makes Shift+Enter distinct
/// from Enter; it is pushed without asking `supports_keyboard_enhancement()`,
/// whose round trip through the input stream can wrongly answer no.
/// Synchronized output wraps each frame (`present`).
///
/// `theme` is the caller's `Theme::from_config` result, fixed until
/// `/theme` changes it.
///
/// # Errors
///
/// Returns the I/O error when raw mode, the alternate screen or the
/// terminal cannot be set up, when a frame cannot be drawn or input
/// cannot be read, or when the terminal cannot be restored.
pub async fn run(
    events: mpsc::Receiver<Event>,
    commands: mpsc::Sender<Command>,
    model_name: String,
    theme: Theme,
    session: SessionProvider,
) -> io::Result<()> {
    enable_raw_mode()?;
    // Armed before anything else can fail, and declared before `terminal`
    // so it drops after it: otherwise a panic restores the screen, then the
    // `Terminal`'s drop flushes its frame over the shell.
    let guard = TerminalGuard::new();
    let mut stdout = io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    // The design's title bar is the terminal's: set once, never drawn.
    let _ = execute!(stdout, SetTitle(format!("{} — aldwin", session.project)));
    let _ = execute!(stdout, EnableBracketedPaste);
    // Must follow `EnterAlternateScreen`: the keyboard mode is per screen.
    // Unconditional; see the doc comment.
    let _ = execute!(
        stdout,
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
    );
    let _ = stdout
        .write_all(ALTERNATE_SCROLL_ON)
        .and_then(|()| stdout.flush());
    // The caret is the terminal's cursor as a bar; `sync_caret` colours it.
    let _ = execute!(stdout, SetCursorStyle::SteadyBar);
    let backend = CrosstermBackend::new(BufWriter::with_capacity(OUT_BUFFER, stdout));
    let mut terminal = Terminal::new(backend)?;

    let result = run_loop(&mut terminal, events, commands, model_name, theme, session).await;
    // Must flush before the guard restores: otherwise the last buffered
    // frame lands on the shell after the alternate screen is left.
    let _ = terminal.backend_mut().flush();
    guard.restore()?;

    result
}

/// Restores the terminal exactly once: `restore()` on normal return,
/// reporting errors; `Drop`, best-effort, on an early `Err` or a panic.
struct TerminalGuard {
    armed: bool,
}

impl TerminalGuard {
    fn new() -> Self {
        Self { armed: true }
    }

    fn restore(mut self) -> io::Result<()> {
        self.armed = false;
        restore_terminal()
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = restore_terminal();
        }
    }
}

/// DECSET 1007, alternate scroll mode: on the alternate screen, wheel
/// notches arrive as `KeyCode::Up`/`Down`, so the wheel scrolls without
/// mouse capture and shares the arrow keys' path in `App::handle_key`.
/// A literal sequence: crossterm has no command for it.
const ALTERNATE_SCROLL_ON: &[u8] = b"\x1b[?1007h";
const ALTERNATE_SCROLL_OFF: &[u8] = b"\x1b[?1007l";

/// Undoes every mode `run` set, attempting each step even when one fails.
/// The keyboard pop is unconditional, matching the push.
/// `DisableMouseCapture` must stay: it releases the mouse when a review
/// held it at a panic, and clears any inherited capture.
fn restore_terminal() -> io::Result<()> {
    let raw = disable_raw_mode();
    let pop = execute!(io::stdout(), PopKeyboardEnhancementFlags);
    let paste = execute!(io::stdout(), DisableBracketedPaste);
    let scroll = io::stdout()
        .write_all(ALTERNATE_SCROLL_OFF)
        .and_then(|()| io::stdout().flush());
    let mouse = execute!(io::stdout(), DisableMouseCapture);
    let alt = execute!(io::stdout(), LeaveAlternateScreen);
    let caret = io::stdout()
        .write_all(CARET_COLOUR_RESET)
        .and_then(|()| execute!(io::stdout(), SetCursorStyle::DefaultUserShape));
    let cursor = execute!(io::stdout(), Show);
    raw.and(pop)
        .and(paste)
        .and(scroll)
        .and(mouse)
        .and(alt)
        .and(caret)
        .and(cursor)
}

/// Paints one frame as a single atomic update: synchronized output (DECSET
/// 2026) so no frame shows half-painted, and the cursor hidden during the
/// paint so it is not dragged across the frame; ratatui's draw shows it
/// again at the caret.
fn present(terminal: &mut Out, app: &mut App) -> io::Result<()> {
    let _ = queue!(terminal.backend_mut(), BeginSynchronizedUpdate, Hide);
    terminal.draw(|f| ui::draw(f, app))?;
    let _ = execute!(terminal.backend_mut(), EndSynchronizedUpdate);
    Ok(())
}

/// Mouse reporting for the review: presses (1000), drags (1002), SGR
/// encoding (1006) for columns past 223. Not crossterm's
/// `EnableMouseCapture`: its 1003 reports every movement, each a repaint.
const MOUSE_ON: &[u8] = b"\x1b[?1000h\x1b[?1002h\x1b[?1006h";
const MOUSE_OFF: &[u8] = b"\x1b[?1006l\x1b[?1002l\x1b[?1000l";

/// Captures the mouse exactly while a review is open (ADR 0010); the wheel
/// then arrives as `MouseEventKind::Scroll*`. Called every iteration;
/// writes only on a change.
fn sync_mouse(out: &mut impl Write, wanted: bool, captured: &mut bool) {
    if wanted == *captured {
        return;
    }
    let _ = out
        .write_all(if wanted { MOUSE_ON } else { MOUSE_OFF })
        .and_then(|()| out.flush());
    *captured = wanted;
}

/// OSC 112: the cursor's colour back to the terminal's own.
const CARET_COLOUR_RESET: &[u8] = b"\x1b]112\x07";

/// Colours the caret with the theme's accent (OSC 12) at start and on each
/// theme change. Written into the frame buffer, so it goes out with the
/// next paint.
fn sync_caret(out: &mut impl Write, theme: Theme, painted: &mut Option<Theme>) {
    if *painted == Some(theme) {
        return;
    }
    if let Color::Rgb(r, g, b) = theme.palette().accent {
        let _ = write!(out, "\x1b]12;#{r:02x}{g:02x}{b:02x}\x07");
    }
    *painted = Some(theme);
}

/// Applies one crossterm event; `false` when input ended or failed, which
/// ends the session. Every other event, `Resize` included, is `true` so it
/// triggers a redraw.
fn apply_input(app: &mut App, event: Option<io::Result<CtEvent>>) -> bool {
    match event {
        Some(Ok(CtEvent::Key(key))) => app.handle_key(key),
        Some(Ok(CtEvent::Paste(text))) => app.paste(&text),
        Some(Ok(CtEvent::Mouse(mouse))) => app.handle_mouse(mouse),
        Some(Ok(_)) => {}
        Some(Err(_)) | None => return false,
    }
    true
}

/// Terminal input on a channel, fed by a blocking reader thread.
///
/// Never `crossterm::event::EventStream`: draining it with `now_or_never`
/// polls with a no-op waker, which it keeps, so input stops waking the loop
/// (guarded by `tests/input_wakeup.rs`). `try_recv` registers no waker.
///
/// The thread is never joined: it holds no state and is parked in
/// `read()`. A failed `blocking_send` tells it the session is over.
fn spawn_input_reader() -> mpsc::Receiver<io::Result<CtEvent>> {
    let (tx, rx) = mpsc::channel(INPUT_CHANNEL);
    std::thread::Builder::new()
        .name("aldwin-input".into())
        .spawn(move || loop {
            match ratatui::crossterm::event::read() {
                Ok(event) => {
                    if tx.blocking_send(Ok(event)).is_err() {
                        break;
                    }
                }
                Err(error) => {
                    let _ = tx.blocking_send(Err(error));
                    break;
                }
            }
        })
        .expect("spawning the terminal input reader");
    rx
}

/// Events the reader may run ahead by: well past a wheel flick or paste
/// burst; beyond it the reader blocks, as backpressure.
const INPUT_CHANNEL: usize = 4096;

async fn run_loop(
    terminal: &mut Out,
    mut events: mpsc::Receiver<Event>,
    commands: mpsc::Sender<Command>,
    model_name: String,
    theme: Theme,
    session: SessionProvider,
) -> io::Result<()> {
    let mut app = App::new(model_name)
        .with_theme(theme)
        .with_facts(&session.project, session.branch.as_deref())
        .with_commands(session.commands)
        .with_sessions(session.sessions)
        .with_catalogue(session.catalogue, session.current_provider);
    let mut input = spawn_input_reader();
    // Drives the caret's blink and the double-Ctrl+C window.
    // `Delay`, not the default `Burst`: `Burst` replays missed ticks back to
    // back after a slow frame, as useless catch-up frames.
    let mut ticker = tokio::time::interval(TICK);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    let mut caret_theme = None;
    sync_caret(terminal.backend_mut(), app.theme, &mut caret_theme);
    present(terminal, &mut app)?;
    let mut mouse_captured = false;
    let mut last_draw = Instant::now();
    let mut dirty = false;
    let mut closed = false;

    loop {
        // Wakes for a pending redraw, so a burst ending inside the frame
        // window still paints its last change; off while nothing is dirty.
        let flush_at = last_draw + MIN_FRAME;

        tokio::select! {
            biased;

            // Input must come first under `biased`: a streaming reply keeps
            // the events branch always ready and would starve the keyboard.
            // A person cannot type fast enough to starve events.
            input_event = input.recv() => {
                if !apply_input(&mut app, input_event) { break }
                dirty = true;
            }

            ev = events.recv() => {
                match ev {
                    Some(event) => { app.apply_event(event); dirty = true; }
                    None => break, // core shut down
                }
            }

            // The counter always advances (the double-Ctrl+C window counts
            // it); a frame only while something animates.
            _ = ticker.tick() => { app.tick(); dirty |= app.is_animating(); }

            _ = tokio::time::sleep_until(flush_at.into()), if dirty => {}
        }

        // Drain everything already waiting into the same frame, so a wheel
        // flick lands in one frame. `try_recv`, never `now_or_never`: see
        // `spawn_input_reader`.
        for _ in 0..MAX_INPUT_PER_FRAME {
            match input.try_recv() {
                Ok(event) => {
                    if !apply_input(&mut app, Some(event)) {
                        closed = true;
                        break;
                    }
                    dirty = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    closed = true;
                    break;
                }
            }
        }
        // Both drains are bounded so a producer that never goes quiet cannot
        // hold the loop past a frame.
        for _ in 0..MAX_EVENTS_PER_FRAME {
            match events.try_recv() {
                Ok(event) => {
                    app.apply_event(event);
                    dirty = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    closed = true;
                    break;
                }
            }
        }

        for command in app.outbox.drain(..) {
            let _ = commands.send(command).await;
        }
        sync_mouse(&mut io::stdout(), app.wants_mouse(), &mut mouse_captured);

        if app.should_quit || closed {
            break;
        }

        // Coalesced to at most one draw per `MIN_FRAME`, not one per
        // `TextDelta`; the `sleep_until` branch paints the last change.
        if dirty && last_draw.elapsed() >= MIN_FRAME {
            sync_caret(terminal.backend_mut(), app.theme, &mut caret_theme);
            present(terminal, &mut app)?;
            last_draw = Instant::now();
            dirty = false;
        }
    }

    Ok(())
}

/// The tick period; the caret's blink and the double-Ctrl+C window count
/// ticks.
const TICK: Duration = Duration::from_millis(120);

/// Minimum gap between redraws (60fps): a ceiling on burst redraws, not a
/// frame rate; an idle session does not draw.
const MIN_FRAME: Duration = Duration::from_millis(16);

/// Core events one frame absorbs before painting: an ordinary streamed
/// reply drains whole, a runaway producer still yields a frame.
const MAX_EVENTS_PER_FRAME: usize = 512;

/// The same for terminal input; a wheel flick must land whole.
const MAX_INPUT_PER_FRAME: usize = 1024;

#[cfg(test)]
mod tests {
    use super::*;

    /// ADR 0010 §3.
    #[test]
    fn the_mouse_is_captured_for_the_review_and_released_after_it() {
        let mut out = Vec::new();
        let mut captured = false;
        sync_mouse(&mut out, false, &mut captured);
        assert!(
            out.is_empty(),
            "nothing is sent while the conversation keeps the mouse"
        );
        sync_mouse(&mut out, true, &mut captured);
        sync_mouse(&mut out, true, &mut captured);
        assert_eq!(
            out, MOUSE_ON,
            "asked for once, however many frames the review is open"
        );
        out.clear();
        sync_mouse(&mut out, false, &mut captured);
        assert_eq!(out, MOUSE_OFF);
        assert!(!captured);
    }

    #[test]
    fn capture_asks_for_presses_and_drags_not_every_movement() {
        let on = String::from_utf8_lossy(MOUSE_ON);
        assert!(on.contains("?1000h") && on.contains("?1002h") && on.contains("?1006h"));
        assert!(!on.contains("1003"));
    }
}
