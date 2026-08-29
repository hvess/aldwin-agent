use std::io;
use std::sync::Arc;

use mjolnir_core::{Command, Event};
use mjolnir_permissions::Engine;
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
/// alone), and always restores the terminal on the way out, success or not.
pub async fn run(events: mpsc::Receiver<Event>, commands: mpsc::Sender<Command>, model_name: String, permissions: Arc<Engine>) -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_loop(&mut terminal, events, commands, model_name, permissions).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
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
