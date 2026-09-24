//! Shared test-only harness helpers, used by this crate's own unit tests
//! across `tools/*.rs`, `sandbox/` and `dispatcher.rs`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use aldwin_core::{DispatchContext, Event, PendingMap, StepId, TurnId};
use tokio::sync::mpsc;

/// Builds a real `DispatchContext` (via core's test-util constructor) plus
/// a held-out clone of the event receiver and the pending map (see
/// `aldwin_core::PendingReply`), so a test can resolve a round trip itself
/// exactly as `Agent`'s command loop does in production.
pub fn dispatch_context() -> (DispatchContext, mpsc::Receiver<Event>, PendingMap) {
    let (tx, rx) = mpsc::channel(16);
    let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
    let ctx = DispatchContext::for_testing(TurnId(1), StepId(1), tx, pending.clone());
    (ctx, rx, pending)
}

/// A directory for a test workspace, or for somewhere outside one.
///
/// Deliberately **not** `tempfile::tempdir()`: that lands under `/tmp`,
/// which the sandbox leaves writable, so a test that a write outside the
/// workspace is refused would pass for the wrong reason — or fail, if the
/// "outside" it picked was `/tmp`.
pub fn scratch_dir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("aldwin-scratch-")
        .tempdir_in(env!("CARGO_MANIFEST_DIR"))
        .expect("a scratch directory outside the incidental paths")
}

/// Whether the tests that exercise confinement can run here.
///
/// On a system that cannot confine a process, those tests have nothing to
/// test — and an early `return` would report them as passing, which is the
/// one thing a sandbox test must never do by accident. So the skip is
/// asked for, not assumed: set `ALDWIN_SKIP_SANDBOX_TESTS=1` and each one
/// says it was skipped; leave it unset and each one fails naming why.
pub fn confinement_or_explicit_skip() -> bool {
    let Some(reason) = crate::sandbox::unavailable() else {
        return true;
    };
    if std::env::var_os("ALDWIN_SKIP_SANDBOX_TESTS").is_some() {
        eprintln!("SKIPPED: processes cannot be confined here ({reason})");
        return false;
    }
    panic!(
        "processes cannot be confined here ({reason}); set ALDWIN_SKIP_SANDBOX_TESTS=1 to skip the sandbox tests on this machine"
    );
}
