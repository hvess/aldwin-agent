//! Test helpers shared by this crate's unit tests.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use aldwin_core::{DispatchContext, Event, PendingMap, StepId, TurnId};
use tokio::sync::mpsc;

/// A real `DispatchContext` plus its event receiver and pending map, so a
/// test can resolve a round trip as `Agent`'s command loop does.
pub fn dispatch_context() -> (DispatchContext, mpsc::Receiver<Event>, PendingMap) {
    let (tx, rx) = mpsc::channel(16);
    let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
    let ctx = DispatchContext::for_testing(TurnId(1), StepId(1), tx, pending.clone());
    (ctx, rx, pending)
}

/// A directory for a test workspace, or for somewhere outside one.
///
/// Not `tempfile::tempdir()`: `/tmp` is writable in the sandbox, so a
/// write-outside-refused test there would be wrong.
pub fn scratch_dir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("aldwin-scratch-")
        .tempdir_in(env!("CARGO_MANIFEST_DIR"))
        .expect("a scratch directory outside the incidental paths")
}

/// Whether the tests that exercise confinement can run here.
///
/// Where confinement is unavailable, panics unless `ALDWIN_SKIP_SANDBOX_TESTS`
/// is set: a sandbox test must never pass silently.
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
