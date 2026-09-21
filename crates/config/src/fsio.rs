//! Filesystem primitives: versioned read, atomic write. No domain knowledge
//! lives here — this module doesn't know about scopes or which domain it's
//! reading, only "a path, a version, a type".

use serde::{de::DeserializeOwned, Serialize};
use std::fs;
use std::io::Write as _;
use std::path::Path;

use crate::error::ConfigError;

#[derive(serde::Deserialize)]
struct VersionOnly {
    version: u32,
}

/// Read and parse `path` as YAML if it exists, checking the version field
/// first so a future major bump fails with `UnknownVersion` instead of a
/// confusing generic parse error. `Ok(None)` means the file does not exist —
/// the normal "nothing persisted here yet" state, not an error.
pub fn read_versioned<T: DeserializeOwned>(
    path: &Path,
    expected_version: u32,
) -> Result<Option<T>, ConfigError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(ConfigError::Io { path: path.to_path_buf(), source: e }),
    };

    let probe: VersionOnly = serde_yaml_ng::from_str(&text)
        .map_err(|e| ConfigError::Parse { path: path.to_path_buf(), source: e })?;
    if probe.version != expected_version {
        return Err(ConfigError::UnknownVersion {
            path:     path.to_path_buf(),
            found:    probe.version,
            expected: expected_version,
        });
    }

    let value: T = serde_yaml_ng::from_str(&text)
        .map_err(|e| ConfigError::Parse { path: path.to_path_buf(), source: e })?;
    Ok(Some(value))
}

/// Write `text` to `path` atomically: a tempfile in the same directory,
/// fsync'd, then renamed over the target. A crash mid-write leaves either the
/// old file or the new one, never a partial write. Creates the parent
/// directory if it doesn't exist yet — the first write into a scope is what
/// materialises `<project>/.aldwin/` or `~/.aldwin/`.
pub fn write_atomic_text(path: &Path, text: &str) -> Result<(), ConfigError> {
    let dir = path.parent().expect("config domain paths always have a parent directory");
    fs::create_dir_all(dir).map_err(|e| ConfigError::Io { path: dir.to_path_buf(), source: e })?;

    let mut tmp = tempfile::Builder::new()
        .prefix(".tmp-")
        .tempfile_in(dir)
        .map_err(|e| ConfigError::Io { path: dir.to_path_buf(), source: e })?;

    tmp.write_all(text.as_bytes())
        .and_then(|_| tmp.as_file().sync_all())
        .map_err(|e| ConfigError::Io { path: path.to_path_buf(), source: e })?;

    tmp.persist(path)
        .map_err(|e| ConfigError::Io { path: path.to_path_buf(), source: e.error })?;
    Ok(())
}

/// Serialise `value` as YAML and write it via [`write_atomic_text`], with
/// `header` — a newline-terminated block of `#` comment lines from the
/// `annotated` module, or `""` — prepended.
///
/// `serde_yaml_ng::to_string` serialises fresh from the in-memory value and
/// knows nothing of the file's comments, so a write without the header would
/// drop a domain's explanation the first time anything is persisted to it
/// (one permission grant is enough). The annotated text is a standing tour
/// of the format, not a one-time greeting.
pub fn write_atomic_with_header<T: Serialize>(path: &Path, header: &str, value: &T) -> Result<(), ConfigError> {
    let body = serde_yaml_ng::to_string(value)
        .map_err(|e| ConfigError::Serialize { path: path.to_path_buf(), source: e })?;
    write_atomic_text(path, &format!("{header}{body}"))
}
