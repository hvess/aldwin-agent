use std::io;
use std::sync::Arc;

use mjolnir_core::{Command, Event};
use mjolnir_permissions::Engine;
use crossterm::cursor::Show;
use futures::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{DisableMouseCapture, Event as CtEvent, EventStream};
use ratatui::crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::crossterm::{execute, ExecutableCommand};
use ratatui::Terminal;
use tokio::sync::mpsc;

use crate::app::App;
use crate::palette::Theme;
use crate::ui;

/// Runs the TUI to completion: sets up the terminal, drives the event loop
/// multiplexing crossterm input and core events on one `tokio::select!` (per
/// mjolnir-tui.md's Pitfall on not blocking the draw loop on either channel
/// alone), and always restores the terminal on the way out — success,
/// `Err`, or a panic unwinding through `run_loop` — via `TerminalGuard`.
///
/// Mouse capture is deliberately **off** — the terminal keeps the mouse, so
/// a plain click-drag is its own native text selection, and copying a chunk
/// of the transcript works the way it does in any other terminal output.
/// This reverses a brief experiment with capture on (which had let the wheel
/// scroll the log directly): a terminal hands mouse events either to the
/// application or to its own selection, never to both, so that trade cost
/// selection outright, reported directly as "text selection has been
/// disabled (or is simply not working)". For a harness whose whole premise
/// is that the developer reads and reasons about the transcript, being able
/// to select and copy out of it beats a wheel binding that
/// PageUp/PageDown/arrow-key scrolling already covers.
///
/// Nothing is sent to enable capture, so nothing needs disabling on the way
/// out — but `restore_terminal` still emits `DisableMouseCapture` anyway, as
/// a cheap belt-and-braces reset of a mode this process may have inherited
/// or a previous build may have left on in the same terminal.
///
/// `theme` (resolved by the caller from `tui.yaml`'s `theme` field via
/// `Theme::from_config` — mjolnir-cli's bootstrap does this) selects which
/// fixed `palette::Palette` every draw uses for the whole session; see
/// `palette.rs`'s module doc comment for why this is a one-time, explicit
/// choice rather than a runtime-switchable global.
pub async fn run(events: mpsc::Receiver<Event>, commands: mpsc::Sender<Command>, model_name: String, permissions: Arc<Engine>, theme: Theme) -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let guard = TerminalGuard::new();

    let result = run_loop(&mut terminal, events, commands, model_name, permissions, theme).await;
    guard.restore()?;

    result
}

/// Restores raw mode, the alternate screen, and the cursor exactly once.
/// `restore()` is the normal-return path — it surfaces any restore error to
/// the caller, same as the sequential `?`-chain this replaces, but (unlike
/// that chain) still attempts every step even if an earlier one fails, so a
/// `disable_raw_mode` error can't also skip leaving the alternate screen or
/// showing the cursor. `Drop` is the fallback for a panic unwinding through
/// `run_loop` — the one path that never reaches `restore()` — and is
/// necessarily best-effort (errors can't propagate out of `Drop`).
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

fn restore_terminal() -> io::Result<()> {
    let raw = disable_raw_mode();
    let mouse = execute!(io::stdout(), DisableMouseCapture);
    let alt = execute!(io::stdout(), LeaveAlternateScreen);
    let cursor = execute!(io::stdout(), Show);
    raw.and(mouse).and(alt).and(cursor)
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    mut events: mpsc::Receiver<Event>,
    commands: mpsc::Sender<Command>,
    model_name: String,
    permissions: Arc<Engine>,
    theme: Theme,
) -> io::Result<()> {
    let mut app = App::new(model_name, permissions).with_theme(theme);
    let mut input = EventStream::new();
    // Drives the "working"/"thinking" spinner's animation frame — a plain
    // redraw timer, not tied to any core event, since there'd otherwise be
    // no way to animate anything between events (per explicit developer
    // feedback that waiting for the next turn gave no loading/progress
    // feedback at all).
    let mut ticker = tokio::time::interval(std::time::Duration::from_millis(120));

    terminal.draw(|f| ui::draw(f, &mut app))?;

    loop {
        tokio::select! {
            biased;

            ev = events.recv() => {
                match ev {
                    Some(event) => app.apply_event(event),
                    None => break, // core shut down
                }
            }

            input_event = input.next() => {
                match input_event {
                    Some(Ok(CtEvent::Key(key))) => app.handle_key(key),
                    // Mouse events never arrive (capture is off — see
                    // `run`'s doc comment); resize is picked up on the next
                    // draw naturally; paste events aren't handled in V0.
                    Some(Ok(_)) => {}
                    Some(Err(_)) | None => break,
                }
            }

            _ = ticker.tick() => app.tick(),
        }

        for command in app.outbox.drain(..) {
            let _ = commands.send(command).await;
        }

        if app.should_quit {
            break;
        }

        terminal.draw(|f| ui::draw(f, &mut app))?;
    }

    Ok(())
}
