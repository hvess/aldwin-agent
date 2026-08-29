use serde::{Deserialize, Serialize};

use crate::error::PermissionError;
use crate::glob::glob_match;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Allow,
    Deny,
}

/// Parsed form of a config-persisted grant string: `kind:pattern`. `kind` is
/// a guarded action's name (a tool name such as `shell` or `read`); `pattern`
/// is matched against a caller-supplied target string via [`glob_match`].
/// mjolnir-config only guarantees these are opaque strings — parsing the
/// grammar is this crate's job (see mjolnir-config's domain.rs doc comment).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantKey {
    pub kind:    String,
    pub pattern: String,
}

impl GrantKey {
    pub fn new(kind: impl Into<String>, pattern: impl Into<String>) -> Self {
        Self { kind: kind.into(), pattern: pattern.into() }
    }

    /// Parses `kind:pattern`. A hand-edited YAML file need not follow the
    /// grammar, so this can fail — callers matching against a whole grant
    /// list should skip unparsable entries rather than propagate the error.
    pub fn parse(entry: &str) -> Result<Self, PermissionError> {
        let (kind, pattern) = entry
            .split_once(':')
            .ok_or_else(|| PermissionError::MalformedGrant { entry: entry.to_string() })?;
        if kind.is_empty() {
            return Err(PermissionError::MalformedGrant { entry: entry.to_string() });
        }
        Ok(Self { kind: kind.to_string(), pattern: pattern.to_string() })
    }

    pub fn matches(&self, kind: &str, target: &str) -> bool {
        self.kind == kind && glob_match(&self.pattern, target)
    }
}

impl std::fmt::Display for GrantKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.kind, self.pattern)
    }
}

impl From<GrantKey> for String {
    fn from(key: GrantKey) -> Self {
        key.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_kind_and_pattern() {
        let key = GrantKey::parse("shell:cargo test*").unwrap();
        assert_eq!(key.kind, "shell");
        assert_eq!(key.pattern, "cargo test*");
    }

    #[test]
    fn pattern_may_itself_contain_colons() {
        let key = GrantKey::parse("read:./src/main.rs:1").unwrap();
        assert_eq!(key.kind, "read");
        assert_eq!(key.pattern, "./src/main.rs:1");
    }

    #[test]
    fn missing_separator_is_malformed() {
        assert!(matches!(GrantKey::parse("no-colon-here"), Err(PermissionError::MalformedGrant { .. })));
    }

    #[test]
    fn empty_kind_is_malformed() {
        assert!(matches!(GrantKey::parse(":pattern"), Err(PermissionError::MalformedGrant { .. })));
    }

    #[test]
    fn round_trips_through_display() {
        let key = GrantKey::new("shell", "cargo test*");
        assert_eq!(key.to_string(), "shell:cargo test*");
        assert_eq!(GrantKey::parse(&key.to_string()).unwrap(), key);
    }

    #[test]
    fn matches_checks_kind_and_pattern() {
        let key = GrantKey::new("shell", "cargo test*");
        assert!(key.matches("shell", "cargo test -- foo"));
        assert!(!key.matches("read", "cargo test -- foo"));
        assert!(!key.matches("shell", "cargo install foo"));
    }
}
