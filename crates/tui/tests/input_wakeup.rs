//! Source-level guard on terminal-input wakeup in `run.rs`, which no
//! rendering test can see.
//!
//! `run_loop` drains already-arrived input before painting. Draining a
//! crossterm `EventStream` with `now_or_never` polls with a no-op waker;
//! an empty poll leaves the stream armed with that waker and never
//! re-registers the real one, so input stops waking the loop (measured:
//! median 42ms, worst 100ms from wheel notch to frame).

/// `run.rs` must not use `now_or_never` or `EventStream`. Input arrives on
/// a `tokio::sync::mpsc` channel from a blocking reader thread, so
/// `try_recv` drains it without disturbing the `recv()` in `select!`.
#[test]
fn the_event_loop_never_polls_terminal_input_with_a_no_op_waker() {
    let source = include_str!("../src/run.rs");
    // Comments are skipped: `run.rs` may name the trap in prose.
    let code: String = source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");

    for forbidden in ["now_or_never", "EventStream"] {
        assert!(
            !code.contains(forbidden),
            "run.rs uses `{forbidden}`. Polling crossterm's EventStream with a no-op waker \
             (which `now_or_never` does) silently unsubscribes the loop from terminal input — \
             see this file's module comment. Drain `spawn_input_reader`'s channel with \
             `try_recv` instead."
        );
    }
    assert!(
        code.contains("try_recv"),
        "run.rs should drain already-arrived terminal input with `try_recv`"
    );
}
