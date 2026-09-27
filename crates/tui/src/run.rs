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
/// The `BufWriter` is not incidental. `io::Stdout` is a `LineWriter` with a
/// ~1KB buffer and ratatui emits no newlines, so a full-frame repaint —
/// which is what *every* scroll step is, since each row's content changes —
/// left the terminal in ~30 separate writes. A terminal composites what has
/// arrived when its own refresh comes round, so a frame delivered in thirty
/// pieces is a frame it can draw halfway through: the tearing behind
/// "scrolling feels jittery". One buffer big enough for a frame makes it
/// one write.
type Out = Terminal<CrosstermBackend<BufWriter<io::Stdout>>>;

/// Enough for a full repaint of a large terminal with a style change on
/// every cell, so a frame is never split across writes by the buffer
/// filling up mid-paint.
const OUT_BUFFER: usize = 1 << 20;

/// What the session needs to know about *where* it is running, beyond the
/// model id the launch card shows: the project and branch the launch card
/// states, the commands the `/` menu offers, the catalogue bare `/model`
/// offers (and the first message asks over, when nothing is configured),
/// which of its rows the session is actually on, and the sessions
/// `/resume` offers. All of it read by aldwin-cli: this crate reads no
/// files.
#[derive(Debug)]
pub struct SessionProvider {
    /// The project — the working directory's own name.
    pub project: String,
    /// The checked-out git branch, when the project is a checkout.
    pub branch: Option<String>,
    /// The `/` menu's rows, in the order drawn. aldwin-cli owns the
    /// commands; this is the part of its table the menu shows.
    pub commands: Vec<CommandChoice>,
    /// Display halves only, in catalogue order. Empty means no question:
    /// bare `/model` is then forwarded to aldwin-cli, which reports rather
    /// than asks.
    pub catalogue: Vec<ProviderChoice>,
    /// The catalogue id of the row `provider.yaml` resolves to, or `None`
    /// for an endpoint the catalogue has never seen — or for nothing
    /// configured at all.
    pub current_provider: Option<String>,
    /// The past sessions bare `/resume` offers, newest first and never
    /// including the one being written. Display halves only: this crate
    /// reads no transcript and formats no timestamp.
    pub sessions: Vec<SessionChoice>,
}

/// Runs the TUI to completion: sets up the terminal, drives the event loop
/// multiplexing crossterm input and core events on one `tokio::select!` (per
/// aldwin-tui.md's Pitfall on not blocking the draw loop on either channel
/// alone), and always restores the terminal on the way out — success,
/// `Err`, or a panic unwinding through `run_loop` — via `TerminalGuard`.
///
/// Mouse capture is deliberately **off** in the conversation — the terminal
/// keeps the mouse there, so a plain click-drag is its own native text
/// selection, and copying a chunk of the transcript works the way it does
/// in any other terminal output.
/// This reverses a brief experiment with capture on (which had let the wheel
/// scroll the log directly): a terminal hands mouse events either to the
/// application or to its own selection, never to both, so that trade cost
/// selection outright, reported directly as "text selection has been
/// disabled (or is simply not working)". For a harness whose whole premise
/// is that the developer reads and reasons about the transcript, being able
/// to select and copy out of it beats a wheel binding.
///
/// The one exception is the full-window review (ADR 0010): lines are
/// selected for a comment by dragging across them, so capture is on for
/// exactly as long as a review is open — see [`sync_mouse`]. The
/// transcript is not on screen then, so there is nothing of it to copy.
///
/// [`ALTERNATE_SCROLL_ON`] is how the wheel comes back in the conversation,
/// without that trade. `restore_terminal` emits `DisableMouseCapture` on the
/// way out, and it is load-bearing: a panic or an error while a review
/// holds the mouse unwinds through `TerminalGuard` with capture on, and
/// that reset is what hands the mouse back to the terminal. It is also
/// broader than what `sync_mouse` asks for, so it clears a mode this process
/// may have inherited too.
///
/// Three other modes are asked for here, all best-effort:
///
/// * **Bracketed paste**, so a paste arrives as one `CtEvent::Paste` rather
///   than as if it had been typed. Without it every newline in a pasted
///   block was a `KeyCode::Enter` and *submitted the line above it* — the
///   reported "pasting multi-line text sends the first sentence as a
///   command".
/// * **The Kitty keyboard protocol's disambiguation flag**, sent
///   unconditionally like every other mode here. This is what makes
///   Shift+Enter distinguishable from Enter at all; on a terminal without
///   it the two are the same bytes, and `App::handle_key`'s Alt+Enter and
///   Ctrl+J fallbacks are the way in.
///
///   Not gated on `supports_keyboard_enhancement()`: that query is a
///   write-then-wait round trip whose reply is read off the same input this
///   process is taking over, so a multiplexer that eats it or a slow answer
///   made it say "no" for a terminal that would have honoured the push —
///   the reported "Shift+Enter works on Linux but not on macOS". `CSI > 1 u`
///   is a private sequence a terminal without the protocol ignores, and the
///   pop on the way out is unconditional too, so the pair stays symmetric.
/// * **Synchronized output** around each frame (see [`present`]).
///
/// `theme` (resolved by the caller from `tui.yaml`'s `theme` field via
/// `Theme::from_config` — aldwin-cli's bootstrap does this) selects which
/// fixed `palette::Palette` every draw uses for the whole session; see
/// `palette.rs`'s module doc comment for why this is a one-time, explicit
/// choice rather than a runtime-switchable global.
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
    // so that on an unwind it drops *after* it: an `Err` from the two `?`s
    // below used to return with raw mode still on, and a panic restored the
    // screen first and then let the `Terminal`'s drop flush its buffered
    // frame over the developer's shell.
    let guard = TerminalGuard::new();
    let mut stdout = io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    // The design's title bar is the terminal's own — `gateway — aldwin`,
    // set once and never drawn. Best-effort: a terminal that ignores the
    // title ignores the sequence.
    let _ = execute!(stdout, SetTitle(format!("{} — aldwin", session.project)));
    let _ = execute!(stdout, EnableBracketedPaste);
    // Pushed after the alternate screen is up, because the keyboard mode is
    // part of the screen's own state — the flags have to land on the screen
    // the session actually runs on. Best-effort, and unconditional: see this
    // function's doc comment on why asking first was the bug.
    let _ = execute!(
        stdout,
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
    );
    // Best-effort: a terminal that doesn't know the mode ignores the
    // sequence, and one that does gives the wheel back without costing
    // selection. Not worth failing the session over either way.
    let _ = stdout
        .write_all(ALTERNATE_SCROLL_ON)
        .and_then(|()| stdout.flush());
    // The caret is the terminal's cursor, drawn as the design's bar;
    // `sync_caret` gives it the accent. Best-effort, like every mode here.
    let _ = execute!(stdout, SetCursorStyle::SteadyBar);
    let backend = CrosstermBackend::new(BufWriter::with_capacity(OUT_BUFFER, stdout));
    let mut terminal = Terminal::new(backend)?;

    let result = run_loop(&mut terminal, events, commands, model_name, theme, session).await;
    // Before the guard, not after: the frame the loop last painted is still
    // sitting in `OUT_BUFFER` at this point, and restoring writes straight
    // to `io::stdout()`. Left to the `Terminal`'s own drop, that frame would
    // be flushed *after* the alternate screen had already been left — a
    // screenful of transcript printed over the developer's shell.
    let _ = terminal.backend_mut().flush();
    guard.restore()?;

    result
}

/// Restores raw mode, the alternate screen, and the cursor exactly once.
/// `restore()` is the normal-return path and surfaces any restore error,
/// still attempting every step when an earlier one fails. `Drop` covers an
/// early `Err` out of `run`'s setup and a panic unwinding through
/// `run_loop`, and is necessarily best-effort.
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

/// DECSET 1007, *alternate scroll mode*: while the alternate screen is up,
/// the terminal translates wheel notches into cursor-key presses instead of
/// scrolling its own (empty) scrollback.
///
/// This is what makes the wheel scroll the transcript **without** taking the
/// mouse away from the terminal — the reason `run`'s doc comment gives for
/// leaving capture off. The events arrive as ordinary `KeyCode::Up`/`Down`
/// and land in `App::handle_key` beside the arrow keys themselves, so there
/// is no second scroll path to keep in step with the first, and a wheel
/// notch moves whatever a press of the same key would.
///
/// Not a crossterm `Command` — crossterm models mouse *capture* (1000/1002/
/// 1006) and has nothing for 1007, so it goes out as the literal sequence.
/// xterm, VTE, kitty, Alacritty, WezTerm, iTerm2 and Windows Terminal all
/// implement it; a terminal that doesn't simply ignores an unknown private
/// mode, which is why both writes below are best-effort.
const ALTERNATE_SCROLL_ON: &[u8] = b"\x1b[?1007h";
const ALTERNATE_SCROLL_OFF: &[u8] = b"\x1b[?1007l";

/// Pops exactly what `run` pushed, in every case — the push is
/// unconditional now, so pairing it no longer depends on a detection result
/// that could differ between the two ends of a session. A terminal without
/// the protocol ignores the pop the same way it ignored the push, and one
/// with it is left holding the mode it had before this process started.
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

/// Paints one frame as a single atomic update.
///
/// Two things wrap ratatui's own draw, and both exist so that a scroll —
/// the one interaction that changes every row at once — reads as the
/// content moving rather than as the screen being rewritten:
///
/// * **DECSET 2026**, synchronized output. The terminal holds everything
///   between the two sequences back and composites it in one go, so a frame
///   can never be shown half-painted. Terminals without it ignore an
///   unknown private mode, which is why this is best-effort.
/// * **Hiding the cursor across the paint.** ratatui writes the whole frame
///   *before* it places the cursor, so on a terminal with no synchronized
///   output the caret would otherwise be dragged visibly across the frame,
///   cell by cell, as the rows go out. ratatui's own draw shows it again at
///   the end, at the position the composer asked for.
fn present(terminal: &mut Out, app: &mut App) -> io::Result<()> {
    let _ = queue!(terminal.backend_mut(), BeginSynchronizedUpdate, Hide);
    terminal.draw(|f| ui::draw(f, app))?;
    let _ = execute!(terminal.backend_mut(), EndSynchronizedUpdate);
    Ok(())
}

/// Mouse reporting for the review, and no more of it than the review uses:
/// presses and releases (1000), motion only while a button is held — a
/// drag — (1002), in SGR encoding (1006) so a column past 223 still
/// reports. Not crossterm's `EnableMouseCapture`, which also turns on
/// 1003, every pointer movement, and would repaint the review each time
/// the pointer merely crossed it.
const MOUSE_ON: &[u8] = b"\x1b[?1000h\x1b[?1002h\x1b[?1006h";
const MOUSE_OFF: &[u8] = b"\x1b[?1006l\x1b[?1002l\x1b[?1000l";

/// Captures the mouse while a review is open and releases it the moment
/// the review closes (ADR 0010). Best-effort, like every other mode here:
/// a terminal that ignores the request keeps its own selection, and every
/// key the review names still works.
///
/// With capture on the wheel arrives as `MouseEventKind::Scroll*` rather
/// than as alternate-scroll arrow keys, which `App::handle_mouse` routes to
/// the diff. Called once per loop iteration; writes only on a change.
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

/// Colours the cursor — the caret — with the theme's accent (OSC 12), once
/// at the start and again whenever `/theme` changes the palette, since
/// blue means you. Queued into the frame's own buffer, so it goes out with
/// the next paint. A terminal without OSC 12 ignores it and keeps its own
/// cursor colour; the caret is still a bar in the right cell.
fn sync_caret(out: &mut impl Write, theme: Theme, painted: &mut Option<Theme>) {
    if *painted == Some(theme) {
        return;
    }
    if let Color::Rgb(r, g, b) = theme.palette().accent {
        let _ = write!(out, "\x1b]12;#{r:02x}{g:02x}{b:02x}\x07");
    }
    *painted = Some(theme);
}

/// One crossterm event applied to the app. `false` means the input stream
/// ended or failed, and with it the session.
///
/// A `Resize` needs no handling of its own — ratatui re-reads the terminal
/// size on the next draw — but it does need the *redraw*, which is why it
/// is `true` rather than swallowed: the frame it invalidates would
/// otherwise sit stale until some unrelated event or the next tick
/// came along.
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

/// Terminal input, carried to the loop on a channel by a dedicated blocking
/// reader thread.
///
/// **Not `crossterm::event::EventStream`**, and the difference is the whole
/// reason this exists. `EventStream` is a `Stream`, so the only way to ask
/// it "is there another event right now?" without awaiting is
/// `now_or_never()` — and that polls it with a **no-op waker**.
/// `EventStream::poll_next` treats every poll as a subscription: when no
/// event is ready it hands the waker it was given to its own background
/// reader thread and sets an "already armed" flag, and a later poll carrying
/// the *real* task waker finds that flag set and does not re-register. So
/// one `now_or_never` that comes up empty leaves the stream holding a waker
/// that does nothing, and terminal input stops being able to wake the loop
/// at all.
///
/// It still moved, which is why this survived review: the loop was woken by
/// whatever else happened to fire — the 120ms tick, a core event,
/// the pending-redraw timer — and drained the backlog when it got there.
/// Measured against a pty driven at an ordinary scroll rate, that put a
/// **median 42ms and a worst case of 100ms** between a wheel notch and the
/// frame that showed it, in bursts landing on the tick boundary: the
/// transcript moved eight times a second in uneven jumps instead of sixty
/// times a second smoothly, which is exactly the reported "not smooth at
/// all, very laggy and jittery". The same measurement across this channel is
/// a **median 0.0ms and a worst case of 1.0ms**, at the same one-frame
/// coalescing.
///
/// A channel has no such trap: [`mpsc::Receiver::try_recv`] is an ordinary
/// synchronous method that never registers a waker, so draining what has
/// already arrived cannot disturb the `recv()` the `select!` is suspended
/// on. That is the same shape core events already use here — one `recv()` in
/// the `select!`, one bounded `try_recv` drain after it — so both halves of
/// the loop now work the same way rather than one of them being subtly
/// special.
///
/// The thread blocks in `crossterm::event::read()` and is deliberately never
/// joined: it holds no state the session needs on the way out, and the read
/// it is parked in only returns when a key arrives. `blocking_send` gives it
/// real backpressure if the loop ever falls behind, and the send failing is
/// how it learns the session is over.
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

/// Events the reader thread may run ahead by. Comfortably past the burst a
/// wheel flick or a bracketed paste arrives in, so the reader is never the
/// thing throttling input; past that it blocks, which is the correct
/// backpressure — the events are already in the terminal's buffer either
/// way.
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
    // Drives the caret's blink — a plain redraw timer, not tied to any core
    // event, since there'd otherwise be no way to animate anything between
    // events — and the double-Ctrl+C window, which is measured in ticks.
    //
    // `Delay`, not the default `Burst`: a tick that arrives while the loop
    // is busy must not queue up behind the ones after it. `Burst` replays
    // every missed tick back to back the moment the loop is free, so one
    // slow frame turns into a run of catch-up frames that have nothing new
    // to draw — the loop falls behind and then thrashes trying not to be.
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
        // A pending redraw needs a wakeup of its own: a burst that goes
        // quiet inside the frame window would otherwise leave its last
        // change unpainted until some unrelated event arrived. Disabled
        // while nothing is pending, so an idle session waits on real input.
        let flush_at = last_draw + MIN_FRAME;

        tokio::select! {
            biased;

            // Input first, deliberately. With core events polled first, a
            // streaming reply — one `TextDelta` per token, hundreds a
            // second — kept this branch permanently ready and `biased`
            // meant the keyboard was never looked at until the model
            // stopped talking. Scrolling during a reply was the reported
            // "laggy": the keys were not slow, they were queued behind the
            // stream. There is no starvation the other way round, since a
            // developer cannot type fast enough to hold the loop.
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

            // The counter always advances (the double-Ctrl+C window is
            // measured in it), but a tick is only worth a frame while the
            // caret is on screen.
            _ = ticker.tick() => { app.tick(); dirty |= app.is_animating(); }

            _ = tokio::time::sleep_until(flush_at.into()), if dirty => {}
        }

        // Everything else already waiting goes into the *same* frame.
        //
        // The loop used to take one event per iteration and then consider
        // drawing, which made a wheel flick — a burst of thirty-odd
        // alternate-scroll cursor keys — land as thirty separate scroll
        // positions the renderer had to walk through in order. The
        // transcript kept sliding for as long as it took to drain them,
        // well after the developer had stopped scrolling. Applying the
        // whole burst before painting makes a flick land where it was
        // aimed, in one frame.
        //
        // `try_recv`, not a `now_or_never` poll of a `Stream`. See
        // `spawn_input_reader` for why that distinction is the difference
        // between a 0ms and a 42ms median response to a wheel notch.
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
        // Both drains are bounded, and for the same reason: a producer that
        // never goes quiet — a fast provider's deltas, a terminal being
        // pumped bytes by something else — must not be able to hold the
        // loop past a frame and move the starvation problem inside it.
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

        // Coalesced, not one draw per event. A streaming reply arrives as
        // one `TextDelta` per token — hundreds a second on a fast model —
        // and drawing each one spends a frame's work to paint a difference
        // no one can see. Bounded to `MIN_FRAME`, the extra deltas fold into
        // the next frame instead, and the `sleep_until` branch above
        // guarantees the last one is still painted promptly.
        if dirty && last_draw.elapsed() >= MIN_FRAME {
            sync_caret(terminal.backend_mut(), app.theme, &mut caret_theme);
            present(terminal, &mut app)?;
            last_draw = Instant::now();
            dirty = false;
        }
    }

    Ok(())
}

/// How often the tick advances: the caret's blink and the double-Ctrl+C
/// window are both counted in it.
const TICK: Duration = Duration::from_millis(120);

/// The floor on the gap between two redraws — 60fps. Not a target: the loop
/// draws as soon as something changes and it has been this long, so an idle
/// session with no field on screen does not draw at all, one with the
/// caret draws 8 times a second, and a keystroke paints immediately. It is only a ceiling on how
/// fast a *burst* can drive the renderer, and a terminal cannot show more
/// than this anyway.
const MIN_FRAME: Duration = Duration::from_millis(16);

/// How many core events one frame will absorb before painting what it has.
/// Generous enough that an ordinary streamed reply is drained whole, small
/// enough that a runaway producer still yields a frame.
const MAX_EVENTS_PER_FRAME: usize = 512;

/// The same, for terminal input. A wheel flick is a few dozen
/// alternate-scroll cursor keys and has to land whole; nothing a developer
/// can do with a keyboard comes near this.
const MAX_INPUT_PER_FRAME: usize = 1024;

#[cfg(test)]
mod tests {
    use super::*;

    /// ADR 0010 §3: capture follows the review, and each change is written
    /// once — the loop calls this every iteration.
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

    /// Button and drag reporting only: 1003 would report every movement of
    /// the pointer, and each report is a repaint.
    #[test]
    fn capture_asks_for_presses_and_drags_not_every_movement() {
        let on = String::from_utf8_lossy(MOUSE_ON);
        assert!(on.contains("?1000h") && on.contains("?1002h") && on.contains("?1006h"));
        assert!(!on.contains("1003"));
    }
}
