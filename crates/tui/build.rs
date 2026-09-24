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
        .args([
            "status",
            "--porcelain",
            "--",
            ":(top)",
            ":(top,exclude)target",
        ])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| !out.stdout.is_empty())
        .unwrap_or(false);

    let hash = if dirty { format!("{hash}-dirty") } else { hash };
    println!("cargo:rustc-env=ALDWIN_GIT_HASH={hash}");

    // Rebuild triggers. `HEAD` alone is not enough: committing on a branch
    // leaves it saying `ref: refs/heads/main` and only moves the ref file,
    // so the hash went stale for exactly the build that matters — a release
    // packaged straight after its version-bump commit reported the *previous*
    // commit, and `-dirty` besides, because the last build script run had
    // seen an uncommitted tree. So watch whatever HEAD points at as well,
    // and `packed-refs`, which is where the ref lives once git has packed it.
    //
    // A detached HEAD (a CI checkout of a tag) holds the hash itself, so
    // there is no ref to follow and `HEAD` alone is the right trigger.
    let git = std::path::Path::new("../../.git");
    println!("cargo:rerun-if-changed={}", git.join("HEAD").display());
    println!(
        "cargo:rerun-if-changed={}",
        git.join("packed-refs").display()
    );
    if let Some(reference) = std::fs::read_to_string(git.join("HEAD"))
        .ok()
        .and_then(|h| h.strip_prefix("ref: ").map(|r| r.trim().to_string()))
    {
        println!("cargo:rerun-if-changed={}", git.join(reference).display());
    }
}
