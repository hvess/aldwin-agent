//! Embeds the current git commit into `ALDWIN_GIT_HASH` for
//! `src/version.rs`; the version alone does not identify a build between
//! releases.

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

    // Must exclude `target`: it is checked in and cargo writes to it before
    // this runs, so an unfiltered status marks every build dirty.
    // `:(top)` makes the pathspec repo-root-relative; a build script runs in
    // the crate directory.
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

    // `HEAD` alone is not enough: a commit on a branch moves only the ref
    // file, so also watch the ref HEAD names and `packed-refs`, where a
    // packed ref lives. A detached HEAD holds the hash itself.
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
