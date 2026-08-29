//! Embeds the current git commit into `MJOLNIR_GIT_HASH` at compile time,
//! for the welcome banner (`ui::intro_lines`). `CARGO_PKG_VERSION` alone
//! (the workspace's shared `0.1.0`) doesn't move between commits on this
//! actively-developed harness, so it can't tell a developer which build
//! they're actually running — the commit can.

use std::process::Command;

fn main() {
    let hash = Command::new("git")
        .args(["rev-parse", "--short=8", "HEAD"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let dirty = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .map(|out| !out.stdout.is_empty())
        .unwrap_or(false);

    let hash = if dirty { format!("{hash}-dirty") } else { hash };
    println!("cargo:rustc-env=MJOLNIR_GIT_HASH={hash}");

    // Best-effort rebuild trigger on a new commit or checkout — workspace
    // root is two levels up from this crate. Not watching refs/heads too:
    // a missed rebuild here just means a stale hash until something else
    // (a source edit) triggers recompilation, not a build failure.
    println!("cargo:rerun-if-changed=../../.git/HEAD");
}
