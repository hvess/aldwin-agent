//! Embeds the current git commit into `ALDWIN_GIT_HASH` at compile time,
//! for `src/version.rs`. `CARGO_PKG_VERSION` alone doesn't move between
//! commits, so it can't tell a developer which build they're actually
//! running — the commit can.

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

    // `target/` is checked into this repo, and cargo has necessarily
    // written to it before this build script runs — so an unfiltered
    // `git status --porcelain` calls *every* build dirty, a clean CI
    // checkout of a release tag included. v0.1.12 shipped reporting
    // `0.1.12 (3117724b-dirty)` for exactly that reason. Only a source
    // change should mark a build dirty; build output never can.
    //
    // `:(top)` makes the pathspec repo-root-relative rather than relative
    // to this crate's directory, which is where a build script runs.
    let dirty = Command::new("git")
        .args(["status", "--porcelain", "--", ":(top)", ":(top,exclude)target"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| !out.stdout.is_empty())
        .unwrap_or(false);

    let hash = if dirty { format!("{hash}-dirty") } else { hash };
    println!("cargo:rustc-env=ALDWIN_GIT_HASH={hash}");

    // Best-effort rebuild trigger on a new commit or checkout — workspace
    // root is two levels up from this crate. Not watching refs/heads too:
    // a missed rebuild here just means a stale hash until something else
    // (a source edit) triggers recompilation, not a build failure.
    println!("cargo:rerun-if-changed=../../.git/HEAD");
}
