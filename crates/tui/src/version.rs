//! What build this is — the one place the answer is spelled out.
//!
//! [`VERSION`] comes from the workspace's `Cargo.toml`, which is the source
//! of truth: a release bumps it and *then* tags the same number, and
//! `.github/workflows/release.yml` refuses to publish a tag that disagrees
//! with it. Before that discipline, releases were tag-only — the manifest
//! sat at `0.1.0` through eleven tagged releases, so `mjolnir --version`
//! and the TUI's own top bar both reported `0.1.0` no matter which build
//! you were running.
//!
//! [`GIT_HASH`] comes from `build.rs` and is what distinguishes two builds
//! of the *same* version — the common case on an actively developed
//! harness, where most builds sit somewhere after the last release tag.
//!
//! This lives in `mjolnir-tui` because `build.rs` already does, and because
//! `mjolnir-cli` (which needs the same string for `--version`) already
//! depends on this crate. An eighth workspace crate holding three constants
//! would need its own spec under `.claude/spec/` per this project's own
//! workflow, which is a poor trade for three constants.

/// The release version, from `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Short commit this was built from, with a `-dirty` suffix when the tree
/// had uncommitted changes.
pub const GIT_HASH: &str = env!("MJOLNIR_GIT_HASH");

/// `0.1.12 (a1b2c3d4)` — what `mjolnir --version` prints. The version alone
/// can't identify a build between releases; the commit alone doesn't say
/// which release it belongs to.
pub const VERSION_FULL: &str = concat!(env!("CARGO_PKG_VERSION"), " (", env!("MJOLNIR_GIT_HASH"), ")");

#[cfg(test)]
mod tests {
    use super::*;

    /// The bug these guard: releases were tag-only, so `VERSION` — which is
    /// what `mjolnir --version` and the TUI's top bar both print — stayed at
    /// the manifest's `0.1.0` through eleven tagged releases. Nothing in the
    /// build could notice, because the manifest and the tag never met. They
    /// meet in `.github/workflows/release.yml` now; what is checked here is
    /// the other half, that the constants really are wired to the manifest
    /// and to each other.
    #[test]
    fn version_is_the_manifest_version() {
        assert_eq!(VERSION, env!("CARGO_PKG_VERSION"));
        assert_ne!(VERSION, "0.0.0", "the workspace version must be a real release number");
    }

    #[test]
    fn version_full_carries_both_the_release_and_the_commit() {
        assert!(VERSION_FULL.starts_with(VERSION), "{VERSION_FULL} must lead with the release version");
        assert!(VERSION_FULL.contains(GIT_HASH), "{VERSION_FULL} must name the commit it was built from");
    }
}
