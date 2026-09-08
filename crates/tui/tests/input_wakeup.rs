//! A source-level guard on the one property of the event loop that no
//! rendering test can see.
//!
//! `run_loop` drains whatever terminal input has already arrived before it
//! paints, so a wheel flick lands in one frame instead of being walked
//! through position by position. The obvious way to write that drain — the
//! way it *was* written — is to poll a `crossterm::event::EventStream` with
//! `futures::FutureExt::now_or_never`, and it is a trap.
//!
//! `now_or_never` polls with a **no-op waker**. `EventStream::poll_next`
//! treats every poll as a subscription: when nothing is ready it hands the
//! waker it was given to its own background reader thread and sets an
//! "already armed" flag, and a later poll carrying the real task waker finds
//! that flag set and does not re-register. So a single `now_or_never` that
//! comes up empty leaves the stream holding a waker that does nothing, and
//! terminal input can no longer wake the loop.
//!
//! Nothing fails outright — the loop is woken by whatever else fires (the
//! spinner tick, a core event, the pending-redraw timer) and drains the
//! backlog when it gets there. Measured against a pty driven at an ordinary
//! scroll rate, that was a median 42ms and a worst case of 100ms between a
//! wheel notch and the frame showing it, arriving in bursts on the tick
//! boundary: the reported "not smooth at all, very laggy and jittery".
//! Across the reader thread and channel that replaced it, the same
//! measurement is a median 0.0ms and a worst case of 1.7ms.
//!
//! The whole defect lives in how a future is polled, so it is invisible to
//! every test in this crate: it needs a real terminal, real timing and a
//! real executor to show up at all. This grep is the only thing standing
//! between a plausible-looking refactor and a silent hundredfold regression
//! in input latency, which is why it is a test and not a comment.

/// `run.rs` must not poll terminal input with a no-op waker, and must not
/// reach for the `Stream` that makes doing so the natural spelling. Input
/// arrives on a `tokio::sync::mpsc` channel filled by a blocking reader
/// thread precisely so that draining it (`try_recv`, an ordinary
/// synchronous method that registers no waker) cannot disturb the `recv()`
/// the `select!` is suspended on.
#[test]
fn the_event_loop_never_polls_terminal_input_with_a_no_op_waker() {
    let source = include_str!("../src/run.rs");
    // Only the code matters; the module's own prose explains the trap by
    // name and must stay free to do so.
    let code: String = source.lines().filter(|line| !line.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");

    for forbidden in ["now_or_never", "EventStream"] {
        assert!(
            !code.contains(forbidden),
            "run.rs uses `{forbidden}`. Polling crossterm's EventStream with a no-op waker \
             (which `now_or_never` does) silently unsubscribes the loop from terminal input — \
             see this file's module comment. Drain `spawn_input_reader`'s channel with \
             `try_recv` instead."
        );
    }
    assert!(code.contains("try_recv"), "run.rs should drain already-arrived terminal input with `try_recv`");
}
