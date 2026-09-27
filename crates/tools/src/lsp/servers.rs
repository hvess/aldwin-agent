//! Per-language server config; Rust only (rust-analyzer). A flat match on
//! purpose, not a registry: a new language is one more arm (aldwin-tools.md,
//! Pitfalls).

use std::path::Path;

pub struct LanguageServer {
    pub language_id: &'static str,
    pub command: &'static str,
    pub args: &'static [&'static str],
}

const RUST: LanguageServer = LanguageServer {
    language_id: "rust",
    command: "rust-analyzer",
    args: &[],
};

pub fn language_for_path(path: &Path) -> Option<&'static LanguageServer> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => Some(&RUST),
        _ => None,
    }
}

pub fn language_by_id(language_id: &str) -> Option<&'static LanguageServer> {
    match language_id {
        "rust" => Some(&RUST),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_files_map_to_rust_analyzer() {
        let server = language_for_path(Path::new("src/main.rs")).unwrap();
        assert_eq!(server.language_id, "rust");
        assert_eq!(server.command, "rust-analyzer");
    }

    #[test]
    fn unknown_extensions_have_no_server() {
        assert!(language_for_path(Path::new("README.md")).is_none());
        assert!(language_for_path(Path::new("no_extension")).is_none());
    }
}
