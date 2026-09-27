//! The build's identity, for `aldwin --version` and the launch card.
//!
//! `Cargo.toml`'s version is the source of truth: a release bumps it, then
//! tags it, and `.github/workflows/release.yml` refuses a tag that
//! disagrees.

/// The release version, from `Cargo.toml`.
pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Short commit from `build.rs`, suffixed `-dirty` when the source tree had
/// uncommitted changes.
pub(crate) const GIT_HASH: &str = env!("ALDWIN_GIT_HASH");

/// `0.1.12 (a1b2c3d4)`: what `aldwin --version` prints.
pub const VERSION_FULL: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("ALDWIN_GIT_HASH"),
    ")"
);

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: `VERSION` stayed at the manifest's `0.1.0` across tagged
    /// releases. The tag half is checked in `.github/workflows/release.yml`.
    #[test]
    fn version_is_the_manifest_version() {
        assert_eq!(VERSION, env!("CARGO_PKG_VERSION"));
        assert_ne!(
            VERSION, "0.0.0",
            "the workspace version must be a real release number"
        );
    }

    #[test]
    fn version_full_carries_both_the_release_and_the_commit() {
        assert!(
            VERSION_FULL.starts_with(VERSION),
            "{VERSION_FULL} must lead with the release version"
        );
        assert!(
            VERSION_FULL.contains(GIT_HASH),
            "{VERSION_FULL} must name the commit it was built from"
        );
    }
}
