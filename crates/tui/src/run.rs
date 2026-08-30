use std::io;
use std::sync::Arc;

use mjolnir_core::{Command, Event};
use mjolnir_permissions::Engine;
use crossterm::cursor::Show;
use futures::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{Event as CtEvent, EventStream};
use ratatui::crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::crossterm::{execute, ExecutableCommand};
use ratatui::Terminal;
use tokio::sync::mpsc;

use crate::app::App;
use crate::ui;

/// Runs the TUI to completion: sets up the terminal, drives the event loop
/// multiplexing crossterm input and core events on one `tokio::select!` (per
/// mjolnir-tui.md's Pitfall on not blocking the draw loop on either channel
/// alone), and always restores the terminal on the way out — success,
/// `Err`, or a panic unwinding through `run_loop` — via `TerminalGuard`.
pub async fn run(events: mpsc::Receiver<Event>, commands: mpsc::Sender<Command>, model_name: String, permissions: Arc<Engine>) -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let guard = TerminalGuard::new();

    let result = run_loop(&mut terminal, events, commands, model_name, permissions).await;
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
    let alt = execute!(io::stdout(), LeaveAlternateScreen);
    let cursor = execute!(io::stdout(), Show);
    raw.and(alt).and(cursor)
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    mut events: mpsc::Receiver<Event>,
    commands: mpsc::Sender<Command>,
    model_name: String,
    permissions: Arc<Engine>,
) -> io::Result<()> {
    let mut app = App::new(model_name, permissions);
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
                    // Resize is picked up on the next draw naturally; mouse
                    // and paste events aren't handled in V0 (keyboard-only).
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
