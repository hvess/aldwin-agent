//! Filesystem primitives: versioned read, atomic write. Knows a path, a
//! version and a type; never a scope or a domain.

use serde::{de::DeserializeOwned, Serialize};
use std::fs;
use std::io::Write as _;
use std::path::Path;

use crate::error::ConfigError;

#[derive(serde::Deserialize)]
struct VersionOnly {
    version: u32,
}

/// Reads and parses `path` as YAML; `Ok(None)` when the file does not exist.
///
/// The version is probed first, so a version of zero or newer than
/// `current_version` fails as `UnknownVersion`, not as a parse error. An
/// older version is handed to the schema, not refused: permissions (ADR
/// 0011) still reads its v1 files.
pub fn read_versioned<T: DeserializeOwned>(
    path: &Path,
    current_version: u32,
) -> Result<Option<T>, ConfigError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(ConfigError::Io {
                path: path.to_path_buf(),
                source: e,
            })
        }
    };

    let probe: VersionOnly = serde_yaml_ng::from_str(&text).map_err(|e| ConfigError::Parse {
        path: path.to_path_buf(),
        source: e,
    })?;
    if probe.version == 0 || probe.version > current_version {
        return Err(ConfigError::UnknownVersion {
            path: path.to_path_buf(),
            found: probe.version,
            expected: current_version,
        });
    }

    let value: T = serde_yaml_ng::from_str(&text).map_err(|e| ConfigError::Parse {
        path: path.to_path_buf(),
        source: e,
    })?;
    Ok(Some(value))
}

/// Writes `text` to `path` atomically: a synced tempfile in the same
/// directory renamed over the target, so a crash leaves the old file or the
/// new one. Creates the parent directory; the first write into a scope is
/// what creates `<project>/.aldwin/` or `~/.aldwin/`.
pub fn write_atomic_text(path: &Path, text: &str) -> Result<(), ConfigError> {
    let dir = path
        .parent()
        .expect("config domain paths always have a parent directory");
    fs::create_dir_all(dir).map_err(|e| ConfigError::Io {
        path: dir.to_path_buf(),
        source: e,
    })?;

    let mut tmp = tempfile::Builder::new()
        .prefix(".tmp-")
        .tempfile_in(dir)
        .map_err(|e| ConfigError::Io {
            path: dir.to_path_buf(),
            source: e,
        })?;

    tmp.write_all(text.as_bytes())
        .and_then(|_| tmp.as_file().sync_all())
        .map_err(|e| ConfigError::Io {
            path: path.to_path_buf(),
            source: e,
        })?;

    tmp.persist(path).map_err(|e| ConfigError::Io {
        path: path.to_path_buf(),
        source: e.error,
    })?;
    Ok(())
}

/// Serialises `value` as YAML and writes it via [`write_atomic_text`] with
/// `header` prepended: a newline-terminated `#` comment block from
/// `annotated`, or `""`.
///
/// Serialising drops the file's comments, so a domain with a header must
/// pass it on every write.
pub fn write_atomic_with_header<T: Serialize>(
    path: &Path,
    header: &str,
    value: &T,
) -> Result<(), ConfigError> {
    let body = serde_yaml_ng::to_string(value).map_err(|e| ConfigError::Serialize {
        path: path.to_path_buf(),
        source: e,
    })?;
    write_atomic_text(path, &format!("{header}{body}"))
}
