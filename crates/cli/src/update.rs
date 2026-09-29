//! `/update`: installs the latest release over the running binary, checked as
//! `install.sh` checks it — the archive against `SHA256SUMS`, and
//! `SHA256SUMS` against the release key in `allowed_signers`.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use flate2::read::GzDecoder;
use reqwest::Client;
use semver::Version;
use sha2::{Digest, Sha256};
use ssh_key::{PublicKey, SshSig};
use tar::Archive;
use tempfile::NamedTempFile;
use thiserror::Error;

/// The key the running binary was built with, not one served beside the
/// signature: a release signed after a key rotation needs `install.sh` once.
const ALLOWED_SIGNERS: &str = include_str!("../../../allowed_signers");

/// Must match `SIG_NAMESPACE` and `SIG_PRINCIPAL` in `scripts/release.sh`.
const NAMESPACE: &str = "aldwin-release";
const PRINCIPAL: &str = "release@aldwin";

/// This build's archive target, as `scripts/release.sh` names it; `None`
/// where `.github/workflows/release.yml` builds no release.
const TARGET: Option<&str> = if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
    Some("x86_64-unknown-linux-gnu")
} else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
    Some("aarch64-apple-darwin")
} else {
    None
};

/// What an update did.
#[derive(Debug, PartialEq)]
pub(crate) enum Outcome {
    /// The running version is the latest.
    Current(Version),
    /// This version now replaces the binary on disk; the process runs on
    /// the old one until it is started again.
    Installed(Version),
}

/// Why nothing was installed. Every variant leaves the binary untouched.
#[derive(Debug, Error)]
pub(crate) enum UpdateError {
    /// No release is built for this platform.
    #[error("there is no release for this platform, so build it from source")]
    NoRelease,
    /// A request failed or answered with an error status.
    #[error("the release could not be downloaded: {0}")]
    Download(#[from] reqwest::Error),
    /// `/releases/latest` redirected somewhere that names no version.
    #[error("{0} does not name a release version")]
    NotAVersion(String),
    /// `SHA256SUMS` is not signed by the release key.
    #[error("SHA256SUMS does not carry a valid release signature")]
    Signature,
    /// `SHA256SUMS` has no line for this platform's archive.
    #[error("SHA256SUMS does not list {0}")]
    Unlisted(String),
    /// The archive's hash is not the one `SHA256SUMS` lists.
    #[error("{0} does not match its checksum")]
    Checksum(String),
    /// The archive is not a gzipped tar.
    #[error("the archive could not be read: {0}")]
    Archive(#[source] io::Error),
    /// The archive has no `aldwin` entry.
    #[error("the archive holds no aldwin binary")]
    NoBinary,
    /// The binary could not be replaced.
    #[error("{path} could not be replaced: {source}")]
    Install {
        /// The binary being replaced.
        path: PathBuf,
        /// The underlying failure.
        #[source]
        source: io::Error,
    },
}

/// Installs the latest release of `repo` (a GitHub URL such as
/// `CARGO_PKG_REPOSITORY`) over `exe` when it is newer than this build.
///
/// # Errors
///
/// Any [`UpdateError`]; `exe` is replaced only after every check passes, in
/// one rename.
pub(crate) async fn update(repo: &str, exe: &Path) -> Result<Outcome, UpdateError> {
    let current = Version::parse(env!("CARGO_PKG_VERSION"))
        .expect("the workspace version is semver: scripts/release.sh check compares it to a tag");
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(30))
        .build()?;

    // The redirect `/releases/latest` answers with, as `install.sh` reads
    // it: no API token or rate limit.
    let latest = client
        .get(format!("{repo}/releases/latest"))
        .send()
        .await?
        .error_for_status()?;
    let latest = latest_version(latest.url().as_str())?;
    if latest <= current {
        return Ok(Outcome::Current(current));
    }

    let target = TARGET.ok_or(UpdateError::NoRelease)?;
    let base = format!("{repo}/releases/download/v{latest}");
    let sums = download(&client, &format!("{base}/SHA256SUMS")).await?;
    let signature = download(&client, &format!("{base}/SHA256SUMS.sig")).await?;
    verify(&sums, &signature)?;
    let name = format!("aldwin-v{latest}-{target}.tar.gz");
    let archive = download(&client, &format!("{base}/{name}")).await?;

    let exe = exe.to_owned();
    tokio::task::spawn_blocking(move || {
        check(&sums, &name, &archive)?;
        let binary = binary_in(&archive)
            .map_err(UpdateError::Archive)?
            .ok_or(UpdateError::NoBinary)?;
        install(&binary, &exe).map_err(|source| UpdateError::Install { path: exe, source })
    })
    .await
    .expect("the closure returns its failures rather than panicking, and a blocking task is never cancelled")?;
    Ok(Outcome::Installed(latest))
}

async fn download(client: &Client, url: &str) -> Result<Vec<u8>, UpdateError> {
    let response = client.get(url).send().await?.error_for_status()?;
    Ok(response.bytes().await?.to_vec())
}

/// The version in a release page's URL, `…/releases/tag/v0.7.0`.
fn latest_version(url: &str) -> Result<Version, UpdateError> {
    url.rsplit('/')
        .next()
        .and_then(|tag| tag.strip_prefix('v'))
        .and_then(|version| Version::parse(version).ok())
        .ok_or_else(|| UpdateError::NotAVersion(url.to_string()))
}

/// Checks `sums` against `signature` and the key [`ALLOWED_SIGNERS`] gives
/// [`PRINCIPAL`].
fn verify(sums: &[u8], signature: &[u8]) -> Result<(), UpdateError> {
    let signature = SshSig::from_pem(signature).map_err(|_| UpdateError::Signature)?;
    let verified = ALLOWED_SIGNERS
        .lines()
        .filter_map(|line| line.strip_prefix(PRINCIPAL)?.strip_prefix(' '))
        .filter_map(|key| PublicKey::from_openssh(key).ok())
        .any(|key| key.verify(NAMESPACE, sums, &signature).is_ok());
    verified.then_some(()).ok_or(UpdateError::Signature)
}

/// Checks `archive` against the hash `sums` lists for `name`, in
/// `sha256sum`'s `<hex>  <name>` lines.
fn check(sums: &[u8], name: &str, archive: &[u8]) -> Result<(), UpdateError> {
    let listed = String::from_utf8_lossy(sums)
        .lines()
        .find_map(|line| {
            let (hash, file) = line.split_once(char::is_whitespace)?;
            (file.trim_start() == name).then(|| hash.to_ascii_lowercase())
        })
        .ok_or_else(|| UpdateError::Unlisted(name.to_string()))?;
    if listed == sha256_hex(archive) {
        Ok(())
    } else {
        Err(UpdateError::Checksum(name.to_string()))
    }
}

/// Lowercase hex, as `sha256sum` writes it.
fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The `aldwin` entry of a gzipped tar, if it has one.
fn binary_in(archive: &[u8]) -> io::Result<Option<Vec<u8>>> {
    let mut entries = Archive::new(GzDecoder::new(archive));
    for entry in entries.entries()? {
        let mut entry = entry?;
        if entry.path()?.as_ref() == Path::new("aldwin") {
            let mut binary = Vec::new();
            entry.read_to_end(&mut binary)?;
            return Ok(Some(binary));
        }
    }
    Ok(None)
}

/// Replaces `exe` with `binary` in one rename, keeping its permissions.
///
/// The new file is written beside `exe` so the rename stays on one
/// filesystem; the running process keeps the inode it started from.
fn install(binary: &[u8], exe: &Path) -> io::Result<()> {
    let dir = exe
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "it has no directory"))?;
    let mut file = NamedTempFile::new_in(dir)?;
    file.write_all(binary)?;
    file.as_file()
        .set_permissions(fs::metadata(exe)?.permissions())?;
    file.persist(exe)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use flate2::{write::GzEncoder, Compression};
    use tar::{Builder, Header};

    const SUMS: &[u8] = include_bytes!("../tests/fixtures/release-v0.6.0/SHA256SUMS");
    const SIGNATURE: &[u8] = include_bytes!("../tests/fixtures/release-v0.6.0/SHA256SUMS.sig");

    fn archive_holding(path: &str, contents: &[u8]) -> Vec<u8> {
        let gz = GzEncoder::new(Vec::new(), Compression::fast());
        let mut tar = Builder::new(gz);
        let mut header = Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(&mut header, path, contents).unwrap();
        tar.into_inner().unwrap().finish().unwrap()
    }

    fn sums_for(name: &str, archive: &[u8]) -> Vec<u8> {
        format!("{}  {name}\n", sha256_hex(archive)).into_bytes()
    }

    #[test]
    fn a_published_release_verifies_against_the_committed_key() {
        verify(SUMS, SIGNATURE).unwrap();
    }

    #[test]
    fn altered_checksums_fail_the_signature() {
        let mut sums = SUMS.to_vec();
        sums[0] = if sums[0] == b'0' { b'1' } else { b'0' };
        assert!(matches!(
            verify(&sums, SIGNATURE),
            Err(UpdateError::Signature)
        ));
        assert!(matches!(
            verify(SUMS, b"not a signature"),
            Err(UpdateError::Signature)
        ));
    }

    #[test]
    fn the_version_is_read_from_the_release_page_redirect() {
        let url = "https://github.com/hvess/aldwin-agent/releases/tag/v0.7.0";
        assert_eq!(latest_version(url).unwrap(), Version::new(0, 7, 0));
        // No release yet: GitHub answers with the list, which names none.
        let list = "https://github.com/hvess/aldwin-agent/releases";
        assert!(matches!(
            latest_version(list),
            Err(UpdateError::NotAVersion(_))
        ));
    }

    #[test]
    fn an_archive_passes_only_with_the_hash_listed_for_its_name() {
        let archive = archive_holding("aldwin", b"new");
        let name = "aldwin-v0.7.0-x86_64-unknown-linux-gnu.tar.gz";
        check(&sums_for(name, &archive), name, &archive).unwrap();

        let other = archive_holding("aldwin", b"tampered");
        assert!(matches!(
            check(&sums_for(name, &archive), name, &other),
            Err(UpdateError::Checksum(_))
        ));
        assert!(matches!(
            check(&sums_for("elsewhere.tar.gz", &archive), name, &archive),
            Err(UpdateError::Unlisted(_))
        ));
    }

    #[test]
    fn the_binary_is_taken_from_the_archive_by_name() {
        let binary = binary_in(&archive_holding("aldwin", b"new")).unwrap();
        assert_eq!(binary.as_deref(), Some(&b"new"[..]));
        assert_eq!(binary_in(&archive_holding("README", b"new")).unwrap(), None);
        assert!(binary_in(b"not an archive").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn installing_replaces_the_binary_and_keeps_its_mode() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("aldwin");
        fs::write(&exe, b"old").unwrap();
        fs::set_permissions(&exe, fs::Permissions::from_mode(0o750)).unwrap();

        install(b"new", &exe).unwrap();

        assert_eq!(fs::read(&exe).unwrap(), b"new");
        let mode = fs::metadata(&exe).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o750);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
