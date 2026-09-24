use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use crate::{
    annotated,
    domain::{
        ContextFilesConfig, GrantEntry, McpConfig, McpServer, PermissionsConfig, ProviderConfig,
        Rung, TuiConfig, CONTEXT_FILES_VERSION, MCP_VERSION, PERMISSIONS_VERSION, PROVIDER_VERSION,
        TUI_VERSION,
    },
    error::ConfigError,
    fsio,
    scope::Scope,
};

/// Which list a grant belongs to. Kept separate on disk and in memory —
/// collapsing them would turn deny-wins from a structural property into a
/// runtime sort, which is exactly the failure mode the format is meant to
/// prevent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantList {
    Allow,
    Deny,
}

/// Result of [`Config::init_global_if_empty`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InitOutcome {
    /// `~/.aldwin/` did not exist; created it and wrote all four annotated files.
    Created,
    /// `~/.aldwin/` exists and all four domain files are present.
    AlreadyPresent,
    /// `~/.aldwin/` exists but is missing one or more domain files. The
    /// caller must refuse to start rather than auto-fill the gap.
    PartiallyPresent { missing: Vec<&'static str> },
}

/// One (scope, domain) layer that failed to reload; the previous in-memory
/// snapshot for that layer is left untouched.
#[derive(Debug)]
pub struct ReloadFailure {
    pub path: PathBuf,
    pub error: ConfigError,
}

struct Inner {
    project_dir: PathBuf,
    global_dir: PathBuf,

    /// Permissions files moved aside by [`retire_v1_permissions`] during this
    /// open — reported once at startup, never acted on again.
    retired_permissions: Vec<PathBuf>,

    project_permissions: RwLock<PermissionsConfig>,
    global_permissions: RwLock<PermissionsConfig>,
    project_provider: RwLock<Option<ProviderConfig>>,
    global_provider: RwLock<Option<ProviderConfig>>,
    project_mcp: RwLock<McpConfig>,
    global_mcp: RwLock<McpConfig>,
    global_tui: RwLock<TuiConfig>,
    project_context_files: RwLock<ContextFilesConfig>,
}

/// Typed access to Aldwin's on-disk config. Cheap to clone — internally an
/// `Arc`, so every clone shares the same in-memory snapshots. Reads never
/// touch disk; they answer from the snapshot loaded at [`Config::open`] or
/// refreshed by [`Config::reload_all`].
#[derive(Clone, Debug)]
pub struct Config {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Inner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Inner")
            .field("project_dir", &self.project_dir)
            .field("global_dir", &self.global_dir)
            .finish_non_exhaustive()
    }
}

/// Load `provider.yaml` and validate `api_key_env`, since `deny_unknown_fields`
/// alone can't catch a *present but empty* field. Shared by `open`, `init`,
/// and `reload` so all three refuse the same malformed file the same way.
fn load_provider(path: &Path) -> Result<Option<ProviderConfig>, ConfigError> {
    let Some(cfg) = fsio::read_versioned::<ProviderConfig>(path, PROVIDER_VERSION)? else {
        return Ok(None);
    };
    if !cfg.has_valid_api_key_env() {
        return Err(ConfigError::MissingApiKeyEnv {
            path: path.to_path_buf(),
        });
    }
    Ok(Some(cfg))
}

/// One-time best-effort migration across the project's two rebrands:
/// Amundsen→Mjolnir, then Mjolnir→Aldwin. Existing installs have their
/// config at `~/.mjolnir/`, and installs that never saw the middle name at
/// `~/.amundsen/`. If the new `~/.aldwin/` doesn't exist yet but one of the
/// old ones does, move it over so a rebuild-and-reinstall doesn't silently
/// orphan a developer's existing permissions grants and provider config
/// behind a renamed directory `Config::open` no longer looks at.
///
/// The order is newest-first, so a machine carrying both — one that upgraded
/// through the first rebrand while an empty `.amundsen/` was recreated by an
/// older binary — takes the one that was last in use. Only one dir is ever
/// moved; the other is left where it is rather than merged, because merging
/// two permissions files means choosing between them silently.
///
/// Best-effort: a failed rename (e.g. a cross-device home directory) just
/// leaves `global_dir` nonexistent, which `init_global_if_empty` already
/// treats as a normal fresh install — migration must never block startup.
fn migrate_legacy_global_dir(home: &Path, new_dir: &Path) {
    if new_dir.exists() {
        return;
    }
    for legacy in [".mjolnir", ".amundsen"] {
        let legacy_dir = home.join(legacy);
        if legacy_dir.exists() {
            let _ = std::fs::rename(&legacy_dir, new_dir);
            return;
        }
    }
}

/// A v1 `permissions.yaml` described a world that no longer exists: its
/// entries were `kind:pattern` strings naming a tool and a glob, and ADR 0004
/// replaced both halves — a grant is now a program and a class, and the tool
/// those globs were written against (`shell`, taking one opaque command
/// string) is gone.
///
/// There is no honest reading of the old entries, so this does not attempt
/// one. It moves the file aside to `permissions.yaml.v1` and lets the caller
/// start from an empty v2 file, which the annotated header then explains.
/// Reinterpreting the old lines would be the worse failure: a developer would
/// keep a file they recognise while the rules inside it quietly meant
/// something else.
///
/// Returns the backup path when it moved something, so the caller can say so.
fn retire_v1_permissions(path: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(path).ok()?;
    let probe: serde_yaml_ng::Value = serde_yaml_ng::from_str(&text).ok()?;
    if probe.get("version").and_then(serde_yaml_ng::Value::as_u64) != Some(1) {
        return None;
    }
    let backup = path.with_extension("yaml.v1");
    std::fs::rename(path, &backup).ok()?;
    Some(backup)
}

impl Config {
    /// Read every existing layer once, resolving global scope to
    /// `~/.aldwin/`. See [`Config::open_at`] for the same thing with an
    /// explicit global root (used by tests, so they never touch the real
    /// home directory).
    pub fn open(project_root: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let home = dirs::home_dir().ok_or(ConfigError::NoHomeDir)?;
        let global_dir = home.join(".aldwin");
        migrate_legacy_global_dir(&home, &global_dir);
        Self::open_at(project_root, global_dir)
    }

    /// Read every existing layer once. Missing files are the normal "nothing
    /// persisted here yet" state and become empty defaults (or `None` for
    /// provider, which has no meaningful empty state) — only a malformed
    /// file, an unknown version, or an empty `api_key_env` refuses to start.
    pub fn open_at(
        project_root: impl AsRef<Path>,
        global_dir: impl Into<PathBuf>,
    ) -> Result<Self, ConfigError> {
        let global_dir = global_dir.into();
        let project_dir = project_root.as_ref().join(".aldwin");

        let mut retired = Vec::new();
        for dir in [&project_dir, &global_dir] {
            if let Some(backup) = retire_v1_permissions(&dir.join("permissions.yaml")) {
                retired.push(backup);
            }
        }

        let project_permissions =
            fsio::read_versioned(&project_dir.join("permissions.yaml"), PERMISSIONS_VERSION)?
                .unwrap_or_else(PermissionsConfig::empty);
        let global_permissions =
            fsio::read_versioned(&global_dir.join("permissions.yaml"), PERMISSIONS_VERSION)?
                .unwrap_or_else(PermissionsConfig::empty);

        let project_provider = load_provider(&project_dir.join("provider.yaml"))?;
        let global_provider = load_provider(&global_dir.join("provider.yaml"))?;

        let project_mcp = fsio::read_versioned(&project_dir.join("mcp.yaml"), MCP_VERSION)?
            .unwrap_or_else(McpConfig::empty);
        let global_mcp = fsio::read_versioned(&global_dir.join("mcp.yaml"), MCP_VERSION)?
            .unwrap_or_else(McpConfig::empty);

        let global_tui = fsio::read_versioned(&global_dir.join("tui.yaml"), TUI_VERSION)?
            .unwrap_or_else(TuiConfig::empty);

        let project_context_files = fsio::read_versioned(
            &project_dir.join("context_files.yaml"),
            CONTEXT_FILES_VERSION,
        )?
        .unwrap_or_else(ContextFilesConfig::empty);

        Ok(Self {
            inner: Arc::new(Inner {
                project_dir,
                global_dir,
                retired_permissions: retired,
                project_permissions: RwLock::new(project_permissions),
                global_permissions: RwLock::new(global_permissions),
                project_provider: RwLock::new(project_provider),
                global_provider: RwLock::new(global_provider),
                project_mcp: RwLock::new(project_mcp),
                global_mcp: RwLock::new(global_mcp),
                global_tui: RwLock::new(global_tui),
                project_context_files: RwLock::new(project_context_files),
            }),
        })
    }

    fn scope_dir(&self, scope: Scope) -> &Path {
        match scope {
            Scope::Project => &self.inner.project_dir,
            Scope::Global => &self.inner.global_dir,
        }
    }

    fn domain_path(&self, scope: Scope, domain: &str) -> PathBuf {
        self.scope_dir(scope).join(format!("{domain}.yaml"))
    }

    /// Where this project's transcripts live — `~/.aldwin/history/<slug>/`.
    ///
    /// Global-scoped and keyed by project, not written into the project's own
    /// `.aldwin/`: a transcript carries whatever the session's tool results
    /// carried, and that is not something to leave sitting inside a tree the
    /// developer may well be committing.
    pub fn history_dir(&self) -> PathBuf {
        let project_root = self
            .inner
            .project_dir
            .parent()
            .unwrap_or(&self.inner.project_dir);
        crate::history::project_dir(&self.inner.global_dir.join("history"), project_root)
    }

    // ── Read ─────────────────────────────────────────────────────────────

    pub fn project_permissions(&self) -> PermissionsConfig {
        self.inner
            .project_permissions
            .read()
            .expect("lock poisoned")
            .clone()
    }

    pub fn global_permissions(&self) -> PermissionsConfig {
        self.inner
            .global_permissions
            .read()
            .expect("lock poisoned")
            .clone()
    }

    /// `None` means no project-scope override — fall back to `global_provider`.
    pub fn project_provider(&self) -> Option<ProviderConfig> {
        self.inner
            .project_provider
            .read()
            .expect("lock poisoned")
            .clone()
    }

    /// Unlike `project_provider`, absence here is an error: there is no
    /// meaningful default model or `api_key_env` to fall back to.
    pub fn global_provider(&self) -> Result<ProviderConfig, ConfigError> {
        self.inner
            .global_provider
            .read()
            .expect("lock poisoned")
            .clone()
            .ok_or_else(|| ConfigError::ProviderNotConfigured {
                path: self.domain_path(Scope::Global, "provider"),
            })
    }

    pub fn project_mcp(&self) -> McpConfig {
        self.inner
            .project_mcp
            .read()
            .expect("lock poisoned")
            .clone()
    }

    pub fn global_mcp(&self) -> McpConfig {
        self.inner.global_mcp.read().expect("lock poisoned").clone()
    }

    pub fn global_tui(&self) -> TuiConfig {
        self.inner.global_tui.read().expect("lock poisoned").clone()
    }

    pub fn project_context_files(&self) -> ContextFilesConfig {
        self.inner
            .project_context_files
            .read()
            .expect("lock poisoned")
            .clone()
    }

    // ── Write ────────────────────────────────────────────────────────────

    /// Read → mutate → persist-atomically → swap, all under one *held*
    /// write lock — not just the final swap. A concurrent writer (another
    /// mutator on this domain, or `reload_all` re-reading it from disk) must
    /// block until this call has landed on both disk and memory, or one of
    /// the two silently clobbers the other. See aldwin-permissions.md's
    /// Pitfall: "storage must express deny-wins, not last-write-wins". Every
    /// domain-mutating method in this file goes through here so the locking
    /// cannot drift between domains.
    ///
    /// `header` is an `annotated::*_HEADER` constant, or `""` for a domain
    /// with none; see `fsio::write_atomic_with_header` for why it is
    /// prepended on every write.
    fn with_domain_mut<T: Clone + serde::Serialize>(
        &self,
        lock: &RwLock<T>,
        path: &Path,
        header: &str,
        f: impl FnOnce(&mut T),
    ) -> Result<(), ConfigError> {
        let mut guard = lock.write().expect("lock poisoned");
        let mut next = guard.clone();
        f(&mut next);
        fsio::write_atomic_with_header(path, header, &next)?;
        *guard = next;
        Ok(())
    }

    fn permissions_lock(&self, scope: Scope) -> &RwLock<PermissionsConfig> {
        match scope {
            Scope::Project => &self.inner.project_permissions,
            Scope::Global => &self.inner.global_permissions,
        }
    }

    fn with_permissions_mut(
        &self,
        scope: Scope,
        f: impl FnOnce(&mut PermissionsConfig),
    ) -> Result<(), ConfigError> {
        self.with_domain_mut(
            self.permissions_lock(scope),
            &self.domain_path(scope, "permissions"),
            annotated::PERMISSIONS_HEADER,
            f,
        )
    }

    /// Adds `entry` to one list, replacing any existing entry for the same
    /// program in that list. Replacement rather than append because two
    /// entries for one program in one list would make the file's meaning
    /// depend on their order — `git: read` then `git: write` reads as a
    /// widening, but a reader has to know which of the two wins to be sure.
    /// One program, one line, per list.
    pub fn add_grant(
        &self,
        scope: Scope,
        list: GrantList,
        entry: GrantEntry,
    ) -> Result<(), ConfigError> {
        self.with_permissions_mut(scope, |cfg| {
            let target = match list {
                GrantList::Allow => &mut cfg.allow,
                GrantList::Deny => &mut cfg.deny,
            };
            target.retain(|e| e.program != entry.program);
            target.push(entry);
        })
    }

    /// Sets this scope's standing rung. Nothing reads it since ADR 0009; kept
    /// so a test can write a file from the old model. The answer for any call no entry
    /// covers (ADR 0004 §6).
    pub fn set_default_rung(&self, scope: Scope, rung: Rung) -> Result<(), ConfigError> {
        self.with_permissions_mut(scope, |cfg| cfg.default = Some(rung))
    }

    /// Permissions files this open moved aside because they were still on the
    /// pre-ADR-0004 schema. Empty in the ordinary case.
    pub fn retired_permissions(&self) -> &[PathBuf] {
        &self.inner.retired_permissions
    }

    /// Writes `permissions.yaml` for `scope` if it does not exist yet,
    /// leaving whatever it already holds untouched if it does.
    ///
    /// The point is the file's *existence*, not its contents: aldwin-cli
    /// treats a project with no permissions file as one whose access
    /// question has never been answered, so an answer of "allow nothing"
    /// still has to leave a file behind or it would be asked again on every
    /// start. Without this, that case could only be expressed by adding a
    /// grant and removing it again.
    pub fn ensure_permissions(&self, scope: Scope) -> Result<(), ConfigError> {
        self.with_permissions_mut(scope, |_| {})
    }

    /// Removes whatever entry names `program` in one list, whatever class it
    /// carried.
    pub fn remove_grant(
        &self,
        scope: Scope,
        list: GrantList,
        program: &str,
    ) -> Result<(), ConfigError> {
        self.with_permissions_mut(scope, |cfg| {
            let target = match list {
                GrantList::Allow => &mut cfg.allow,
                GrantList::Deny => &mut cfg.deny,
            };
            target.retain(|e| e.program != program);
        })
    }

    pub fn set_provider(&self, scope: Scope, provider: ProviderConfig) -> Result<(), ConfigError> {
        let path = self.domain_path(scope, "provider");
        if !provider.has_valid_api_key_env() {
            return Err(ConfigError::MissingApiKeyEnv { path });
        }
        let lock = match scope {
            Scope::Project => &self.inner.project_provider,
            Scope::Global => &self.inner.global_provider,
        };
        self.with_domain_mut(lock, &path, annotated::PROVIDER_HEADER, move |current| {
            *current = Some(provider)
        })
    }

    fn mcp_lock(&self, scope: Scope) -> &RwLock<McpConfig> {
        match scope {
            Scope::Project => &self.inner.project_mcp,
            Scope::Global => &self.inner.global_mcp,
        }
    }

    fn with_mcp_mut(
        &self,
        scope: Scope,
        f: impl FnOnce(&mut McpConfig),
    ) -> Result<(), ConfigError> {
        self.with_domain_mut(
            self.mcp_lock(scope),
            &self.domain_path(scope, "mcp"),
            annotated::MCP_HEADER,
            f,
        )
    }

    /// Upserts by server name — adding a server that already exists in this
    /// scope replaces it wholesale, the same rule the spec uses for how a
    /// project-scope server shadows a global one of the same name.
    pub fn add_mcp_server(&self, scope: Scope, server: McpServer) -> Result<(), ConfigError> {
        self.with_mcp_mut(scope, |cfg| {
            cfg.servers.retain(|s| s.name != server.name);
            cfg.servers.push(server);
        })
    }

    pub fn remove_mcp_server(&self, scope: Scope, name: &str) -> Result<(), ConfigError> {
        self.with_mcp_mut(scope, |cfg| cfg.servers.retain(|s| s.name != name))
    }

    pub fn set_tui(&self, tui: TuiConfig) -> Result<(), ConfigError> {
        let path = self.domain_path(Scope::Global, "tui");
        self.with_domain_mut(
            &self.inner.global_tui,
            &path,
            annotated::TUI_HEADER,
            move |current| *current = tui,
        )
    }

    fn with_context_files_mut(
        &self,
        f: impl FnOnce(&mut ContextFilesConfig),
    ) -> Result<(), ConfigError> {
        // No `annotated` header: this domain is not part of the first-launch tour.
        self.with_domain_mut(
            &self.inner.project_context_files,
            &self.domain_path(Scope::Project, "context_files"),
            "",
            f,
        )
    }

    pub fn add_context_file(&self, path: PathBuf) -> Result<(), ConfigError> {
        self.with_context_files_mut(|cfg| {
            if !cfg.approved.contains(&path) {
                cfg.approved.push(path);
            }
        })
    }

    pub fn remove_context_file(&self, path: &Path) -> Result<(), ConfigError> {
        self.with_context_files_mut(|cfg| cfg.approved.retain(|p| p != path))
    }

    // ── Reload ───────────────────────────────────────────────────────────

    /// Re-read every layer that currently exists on disk. A layer that fails
    /// to parse keeps its previous in-memory snapshot — a bad hand-edit must
    /// not collapse an in-progress session — and is reported by path so the
    /// caller (the TUI's `/reload-config` handler) can name the failing file.
    pub fn reload_all(&self) -> Result<(), Vec<ReloadFailure>> {
        let mut failures = Vec::new();

        self.reload_domain(
            &self.inner.project_permissions,
            self.domain_path(Scope::Project, "permissions"),
            PERMISSIONS_VERSION,
            PermissionsConfig::empty,
            &mut failures,
        );
        self.reload_domain(
            &self.inner.global_permissions,
            self.domain_path(Scope::Global, "permissions"),
            PERMISSIONS_VERSION,
            PermissionsConfig::empty,
            &mut failures,
        );
        self.reload_domain(
            &self.inner.project_mcp,
            self.domain_path(Scope::Project, "mcp"),
            MCP_VERSION,
            McpConfig::empty,
            &mut failures,
        );
        self.reload_domain(
            &self.inner.global_mcp,
            self.domain_path(Scope::Global, "mcp"),
            MCP_VERSION,
            McpConfig::empty,
            &mut failures,
        );
        self.reload_domain(
            &self.inner.global_tui,
            self.domain_path(Scope::Global, "tui"),
            TUI_VERSION,
            TuiConfig::empty,
            &mut failures,
        );
        self.reload_domain(
            &self.inner.project_context_files,
            self.domain_path(Scope::Project, "context_files"),
            CONTEXT_FILES_VERSION,
            ContextFilesConfig::empty,
            &mut failures,
        );

        self.reload_provider(Scope::Project, &mut failures);
        self.reload_provider(Scope::Global, &mut failures);

        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures)
        }
    }

    fn reload_domain<T: Clone + serde::de::DeserializeOwned>(
        &self,
        lock: &RwLock<T>,
        path: PathBuf,
        version: u32,
        empty: fn() -> T,
        failures: &mut Vec<ReloadFailure>,
    ) {
        // Locked *before* the read, not after it: `with_domain_mut` could
        // otherwise land a write between the two, and the snapshot would be
        // swapped back to the file as it was before that write.
        let mut guard = lock.write().expect("lock poisoned");
        match fsio::read_versioned::<T>(&path, version) {
            Ok(value) => *guard = value.unwrap_or_else(empty),
            Err(error) => failures.push(ReloadFailure { path, error }),
        }
    }

    fn reload_provider(&self, scope: Scope, failures: &mut Vec<ReloadFailure>) {
        let path = self.domain_path(scope, "provider");
        let lock = match scope {
            Scope::Project => &self.inner.project_provider,
            Scope::Global => &self.inner.global_provider,
        };
        // Locked before the read, for `reload_domain`'s reason.
        let mut guard = lock.write().expect("lock poisoned");
        match load_provider(&path) {
            Ok(value) => *guard = value,
            Err(error) => failures.push(ReloadFailure { path, error }),
        }
    }

    // ── First launch ─────────────────────────────────────────────────────

    /// Create `~/.aldwin/` and write the annotated global files if the
    /// directory does not exist. Idempotent — a directory that already
    /// exists is inspected for completeness rather than touched.
    ///
    /// **`provider.yaml` is deliberately not among them.** Every other file
    /// here has a meaningful empty value — no grants, no MCP servers, no
    /// theme override — so writing one states nothing on the developer's
    /// behalf. A provider does not: any file this could write would name a
    /// host, a model and a key variable nobody chose. It used to write
    /// `anthropic` / `claude-sonnet-5`, and because that ran *before* the
    /// session's own check (`global_provider().is_err()`), the
    /// provider question was never once asked — the seed had already
    /// answered it. Leaving the file absent is what makes "no provider is
    /// configured" a real state, and it is the state the launch card's `Model  not set` exists to
    /// resolve.
    pub fn init_global_if_empty(&self) -> Result<InitOutcome, ConfigError> {
        let dir = &self.inner.global_dir;

        if !dir.exists() {
            let permissions_path = dir.join("permissions.yaml");
            let mcp_path = dir.join("mcp.yaml");
            let tui_path = dir.join("tui.yaml");

            fsio::write_atomic_text(&permissions_path, annotated::PERMISSIONS)?;
            fsio::write_atomic_text(&mcp_path, annotated::MCP)?;
            fsio::write_atomic_text(&tui_path, annotated::TUI)?;

            *self
                .inner
                .global_permissions
                .write()
                .expect("lock poisoned") =
                fsio::read_versioned(&permissions_path, PERMISSIONS_VERSION)?
                    .expect("just wrote a file matching this schema");
            *self.inner.global_mcp.write().expect("lock poisoned") =
                fsio::read_versioned(&mcp_path, MCP_VERSION)?
                    .expect("just wrote a file matching this schema");
            *self.inner.global_tui.write().expect("lock poisoned") =
                fsio::read_versioned(&tui_path, TUI_VERSION)?
                    .expect("just wrote a file matching this schema");

            return Ok(InitOutcome::Created);
        }

        // `provider.yaml` is not required: an existing directory without one
        // is a developer who has not answered the provider question yet (or
        // who deleted the file to be asked again), which the provider question handles.
        // The other three are written together at init, so any of them
        // missing really is a half-deleted config directory.
        let required = ["permissions.yaml", "mcp.yaml", "tui.yaml"];
        let missing: Vec<&'static str> = required
            .iter()
            .copied()
            .filter(|f| !dir.join(f).is_file())
            .collect();

        if missing.is_empty() {
            Ok(InitOutcome::AlreadyPresent)
        } else {
            Ok(InitOutcome::PartiallyPresent { missing })
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────
//
// Covers this crate's real failure modes per aldwin-config.md's Pitfalls:
// deny-wins staying structural, version-bump rejection, partial-init refusing
// to start, reload retaining the previous snapshot on a bad file while still
// naming it, and project scope not materialising until first write.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::McpTransport;
    use crate::domain::{Class, GrantEntry, Rung};
    use tempfile::tempdir;

    /// Fresh (project_dir, global_dir) temp roots and the Config opened on
    /// them — neither exists on disk yet, matching a real fresh checkout.
    fn fresh() -> (tempfile::TempDir, tempfile::TempDir, Config) {
        let project = tempdir().unwrap();
        let global = tempdir().unwrap();
        // Use a not-yet-existing subdirectory so "does the dir exist" checks
        // (init_global_if_empty, project-scope materialisation) start from
        // true absence rather than an empty-but-present tempdir.
        let global_root = global.path().join(".aldwin");
        let config = Config::open_at(project.path(), &global_root).unwrap();
        (project, global, config)
    }

    #[test]
    fn legacy_global_dir_is_migrated_when_the_new_one_does_not_exist() {
        for legacy in [".mjolnir", ".amundsen"] {
            let home = tempdir().unwrap();
            let legacy_dir = home.path().join(legacy);
            std::fs::create_dir(&legacy_dir).unwrap();
            std::fs::write(legacy_dir.join("provider.yaml"), "version: 1\n").unwrap();

            let new_dir = home.path().join(".aldwin");
            migrate_legacy_global_dir(home.path(), &new_dir);

            assert!(
                !legacy_dir.exists(),
                "the old {legacy}/ should be moved, not copied"
            );
            assert!(
                new_dir.join("provider.yaml").exists(),
                "the migrated file must survive the move"
            );
        }
    }

    /// Both rebrands' directories present. The newer name wins, and the older
    /// one is left alone rather than merged into it.
    #[test]
    fn the_more_recent_legacy_dir_wins_when_both_exist() {
        let home = tempdir().unwrap();
        for (legacy, body) in [(".mjolnir", "mjolnir"), (".amundsen", "amundsen")] {
            let dir = home.path().join(legacy);
            std::fs::create_dir(&dir).unwrap();
            std::fs::write(dir.join("provider.yaml"), body).unwrap();
        }

        let new_dir = home.path().join(".aldwin");
        migrate_legacy_global_dir(home.path(), &new_dir);

        assert_eq!(
            std::fs::read_to_string(new_dir.join("provider.yaml")).unwrap(),
            "mjolnir",
            "the directory the developer was last using is the one that carries over"
        );
        assert!(
            home.path().join(".amundsen").exists(),
            "the older one stays put — merging would pick between two files silently"
        );
    }

    #[test]
    fn migration_is_a_no_op_when_the_new_global_dir_already_exists() {
        let home = tempdir().unwrap();
        let legacy_dir = home.path().join(".amundsen");
        std::fs::create_dir(&legacy_dir).unwrap();
        std::fs::write(legacy_dir.join("provider.yaml"), "legacy").unwrap();

        let new_dir = home.path().join(".aldwin");
        std::fs::create_dir(&new_dir).unwrap();
        std::fs::write(new_dir.join("provider.yaml"), "current").unwrap();

        migrate_legacy_global_dir(home.path(), &new_dir);

        assert!(
            legacy_dir.exists(),
            "an already-migrated (or independently created) new dir must not trigger another move"
        );
        assert_eq!(
            std::fs::read_to_string(new_dir.join("provider.yaml")).unwrap(),
            "current",
            "the existing new-dir content must not be clobbered"
        );
    }

    #[test]
    fn migration_is_a_no_op_when_no_legacy_dir_exists() {
        let home = tempdir().unwrap();
        let new_dir = home.path().join(".aldwin");

        migrate_legacy_global_dir(home.path(), &new_dir);

        assert!(
            !new_dir.exists(),
            "nothing to migrate — a fresh install must not have .aldwin/ conjured from nothing"
        );
    }

    #[test]
    fn fresh_config_has_empty_defaults_and_no_provider() {
        let (_project, _global, config) = fresh();

        assert_eq!(config.project_permissions(), PermissionsConfig::empty());
        assert_eq!(config.global_permissions(), PermissionsConfig::empty());
        assert_eq!(config.project_mcp(), McpConfig::empty());
        assert_eq!(config.global_mcp(), McpConfig::empty());
        assert_eq!(config.global_tui(), TuiConfig::empty());
        assert_eq!(config.project_context_files(), ContextFilesConfig::empty());
        assert!(config.project_provider().is_none());
        assert!(matches!(
            config.global_provider(),
            Err(ConfigError::ProviderNotConfigured { .. })
        ));
    }

    #[test]
    fn project_scope_directory_is_not_created_until_first_write() {
        let (project, _global, config) = fresh();
        let aldwin_dir = project.path().join(".aldwin");
        assert!(!aldwin_dir.exists());

        config
            .add_grant(
                Scope::Project,
                GrantList::Allow,
                GrantEntry::classed("rg", Class::Read),
            )
            .unwrap();
        assert!(aldwin_dir.is_dir());
        assert!(aldwin_dir.join("permissions.yaml").is_file());
    }

    #[test]
    fn init_global_if_empty_is_created_then_already_present() {
        let (_project, global, config) = fresh();
        let global_dir = global.path().join(".aldwin");
        assert!(!global_dir.exists());

        assert_eq!(config.init_global_if_empty().unwrap(), InitOutcome::Created);
        for f in ["permissions.yaml", "mcp.yaml", "tui.yaml"] {
            assert!(global_dir.join(f).is_file(), "missing {f}");
        }

        // In-memory snapshot reflects what was just written, not stale
        // defaults. A fresh file states neither a rung nor an allow list —
        // ADR 0009 reads neither — only an empty deny list.
        assert_eq!(config.global_permissions(), PermissionsConfig::empty());

        // Idempotent: a second call sees everything already there.
        assert_eq!(
            config.init_global_if_empty().unwrap(),
            InitOutcome::AlreadyPresent
        );
    }

    /// A fresh global directory names no provider at all — the one question
    /// init must not answer on the developer's behalf. Seeding one is what
    /// made the provider question unreachable: it ran first, so the
    /// screen's own "no provider is configured" test was never true, and
    /// every developer silently got the seeded default.
    #[test]
    fn init_writes_no_provider_so_the_question_is_still_open() {
        let (_project, global, config) = fresh();
        assert_eq!(config.init_global_if_empty().unwrap(), InitOutcome::Created);
        assert!(
            !global.path().join(".aldwin").join("provider.yaml").exists(),
            "init must not guess a provider"
        );
        assert!(
            config.global_provider().is_err(),
            "which is what the session reads to know the question is unanswered"
        );
        assert_eq!(
            config.init_global_if_empty().unwrap(),
            InitOutcome::AlreadyPresent,
            "and a directory without one is complete, not half-deleted"
        );
    }

    #[test]
    fn init_global_if_empty_partial_directory_refuses_to_start() {
        let (_project, global, config) = fresh();
        let global_dir = global.path().join(".aldwin");

        config.init_global_if_empty().unwrap();
        std::fs::remove_file(global_dir.join("mcp.yaml")).unwrap();

        match config.init_global_if_empty().unwrap() {
            InitOutcome::PartiallyPresent { missing } => assert_eq!(missing, vec!["mcp.yaml"]),
            other => panic!("expected PartiallyPresent, got {other:?}"),
        }
    }

    #[test]
    fn allow_and_deny_stay_separate_lists_never_merged() {
        let (_project, _global, config) = fresh();

        config
            .add_grant(
                Scope::Project,
                GrantList::Allow,
                GrantEntry::classed("git", Class::Read),
            )
            .unwrap();
        config
            .add_grant(
                Scope::Project,
                GrantList::Deny,
                GrantEntry::classed("git", Class::Write),
            )
            .unwrap();

        let cfg = config.project_permissions();
        assert_eq!(cfg.allow, vec![GrantEntry::classed("git", Class::Read)]);
        assert_eq!(cfg.deny, vec![GrantEntry::classed("git", Class::Write)]);
    }

    /// Regression: a write lock taken only for the final swap let two
    /// concurrent writers compute `next` from the same stale snapshot, and
    /// the second clobbered the first. `with_domain_mut` holds it across the
    /// whole read-mutate-persist-swap, so every grant here must survive.
    #[test]
    fn concurrent_grant_writes_do_not_lose_updates() {
        let (_project, _global, config) = fresh();
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let config = config.clone();
                std::thread::spawn(move || {
                    config
                        .add_grant(
                            Scope::Project,
                            GrantList::Allow,
                            GrantEntry::classed(format!("prog{i}"), Class::Read),
                        )
                        .unwrap();
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }

        let allow = config.project_permissions().allow;
        assert_eq!(
            allow.len(),
            8,
            "every concurrent grant must survive, got {allow:?}"
        );
        for i in 0..8 {
            assert!(
                allow.contains(&GrantEntry::classed(format!("prog{i}"), Class::Read)),
                "missing grant for prog{i} in {allow:?}"
            );
        }
    }

    /// One program gets one line per list. Re-granting it at a different
    /// class replaces the line rather than appending a second one, so the
    /// file never holds two rules for `git` whose combined meaning depends
    /// on which order a reader takes them in.
    #[test]
    fn add_grant_keeps_one_line_per_program_in_a_list() {
        let (_project, _global, config) = fresh();
        config
            .add_grant(
                Scope::Global,
                GrantList::Allow,
                GrantEntry::classed("git", Class::Read),
            )
            .unwrap();
        config
            .add_grant(
                Scope::Global,
                GrantList::Allow,
                GrantEntry::classed("git", Class::Write),
            )
            .unwrap();
        assert_eq!(
            config.global_permissions().allow,
            vec![GrantEntry::classed("git", Class::Write)]
        );
    }

    /// Regression, reported as "editing permissions.yaml doesn't really
    /// appear to make any sense": a plain re-serialise drops every comment,
    /// so the annotated explanation survived only until the *first* grant
    /// was persisted and the developer's real file was a bare `version`/
    /// `allow`/`deny`.
    #[test]
    fn permissions_yaml_keeps_its_explanatory_header_after_a_grant_is_persisted() {
        let (project, global, config) = fresh();
        config
            .add_grant(
                Scope::Project,
                GrantList::Allow,
                GrantEntry::classed("git", Class::Read),
            )
            .unwrap();
        config
            .add_grant(Scope::Global, GrantList::Deny, GrantEntry::program("curl"))
            .unwrap();

        let project_text =
            std::fs::read_to_string(project.path().join(".aldwin").join("permissions.yaml"))
                .unwrap();
        let global_text =
            std::fs::read_to_string(global.path().join(".aldwin").join("permissions.yaml"))
                .unwrap();
        for text in [&project_text, &global_text] {
            assert!(
                text.starts_with("# Aldwin permissions"),
                "grant persistence must not strip the annotated header: {text:?}"
            );
            assert!(
                text.contains("program"),
                "header should still explain the entry shape: {text:?}"
            );
        }
        assert!(project_text.contains("git: read"));
        assert!(global_text.contains("curl"));
    }

    /// Same bug, the other three annotated domains.
    #[test]
    fn provider_mcp_and_tui_yaml_keep_their_headers_after_a_write() {
        let (_project, global, config) = fresh();
        let provider = ProviderConfig {
            version: PROVIDER_VERSION,
            provider: crate::domain::ProviderKind::Anthropic,
            model: "claude-sonnet-5".into(),
            base_url: None,
            api_key_env: "ANTHROPIC_API_KEY".into(),
            extended_thinking_budget: None,
        };
        config.set_provider(Scope::Global, provider).unwrap();
        config
            .add_mcp_server(
                Scope::Global,
                McpServer {
                    name: "fs".into(),
                    transport: McpTransport::Stdio {
                        command: "fs-server".into(),
                        args: vec![],
                    },
                    env: Default::default(),
                },
            )
            .unwrap();
        config
            .set_tui(TuiConfig {
                theme: Some("dark".into()),
                ..TuiConfig::empty()
            })
            .unwrap();

        let provider_text =
            std::fs::read_to_string(global.path().join(".aldwin").join("provider.yaml")).unwrap();
        let mcp_text =
            std::fs::read_to_string(global.path().join(".aldwin").join("mcp.yaml")).unwrap();
        let tui_text =
            std::fs::read_to_string(global.path().join(".aldwin").join("tui.yaml")).unwrap();
        assert!(
            provider_text.starts_with("# Aldwin provider settings"),
            "{provider_text:?}"
        );
        assert!(
            mcp_text.starts_with("# Aldwin MCP server registry"),
            "{mcp_text:?}"
        );
        assert!(
            tui_text.starts_with("# Aldwin TUI preferences"),
            "{tui_text:?}"
        );
    }

    #[test]
    fn remove_grant_only_touches_its_own_list() {
        let (_project, _global, config) = fresh();
        config
            .add_grant(
                Scope::Global,
                GrantList::Allow,
                GrantEntry::classed("rg", Class::Read),
            )
            .unwrap();
        config
            .add_grant(Scope::Global, GrantList::Deny, GrantEntry::program("rg"))
            .unwrap();

        config
            .remove_grant(Scope::Global, GrantList::Allow, "rg")
            .unwrap();

        let cfg = config.global_permissions();
        assert!(cfg.allow.is_empty());
        assert_eq!(cfg.deny, vec![GrantEntry::program("rg")]);
    }

    /// What a developer actually opens. The file is the model's public
    /// face — the reason this whole area was reopened was "the permissions
    /// model is not clear, and editing permissions.yaml doesn't really
    /// appear to make any sense" — so its shape is pinned rather than left
    /// to whatever serde happens to emit.
    #[test]
    fn a_written_permissions_file_reads_as_the_model_it_implements() {
        let (_project, global, config) = fresh();
        config.set_default_rung(Scope::Global, Rung::Read).unwrap();
        config
            .add_grant(
                Scope::Global,
                GrantList::Allow,
                GrantEntry::classed("git", Class::Read),
            )
            .unwrap();
        config
            .add_grant(
                Scope::Global,
                GrantList::Allow,
                GrantEntry::classed("cargo", Class::Write),
            )
            .unwrap();
        config
            .add_grant(Scope::Global, GrantList::Deny, GrantEntry::program("curl"))
            .unwrap();

        let text = std::fs::read_to_string(global.path().join(".aldwin").join("permissions.yaml"))
            .unwrap();
        let body = text
            .lines()
            .filter(|l| !l.starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n");

        let expected = [
            "version: 2",
            "default: read",
            "allow:",
            "- git: read",
            "- cargo: write",
            "deny:",
            "- curl",
        ]
        .join("\n");
        assert_eq!(
            body.trim(),
            expected,
            "the file a developer opens must read as program-and-class rules, not as a serialisation: {text}"
        );

        // And the explanation survives the writes, which is the half that
        // regressed last time.
        assert!(text.starts_with("# Aldwin permissions"), "{text}");
    }

    /// ADR 0007 §1. `roots` is read, an empty one is never written (so a
    /// file that declares none still reads as the four keys it always had),
    /// and `deny_unknown_fields` still rejects a misspelling rather than
    /// silently ignoring the reach a developer thought they had declared.
    #[test]
    fn roots_are_read_from_a_permissions_file_and_a_misspelling_is_an_error() {
        let parsed: PermissionsConfig =
            serde_yaml_ng::from_str("version: 2\nroots:\n- ../proton-libs\n- /abs/other\n")
                .unwrap();
        assert_eq!(
            parsed.roots,
            vec![PathBuf::from("../proton-libs"), PathBuf::from("/abs/other")]
        );

        let none: PermissionsConfig = serde_yaml_ng::from_str("version: 2\n").unwrap();
        assert!(none.roots.is_empty());
        let written = serde_yaml_ng::to_string(&none).unwrap();
        assert!(
            !written.contains("roots"),
            "an empty list must not be written: {written}"
        );

        assert!(
            serde_yaml_ng::from_str::<PermissionsConfig>("version: 2\nroot:\n- ../x\n").is_err()
        );
    }

    /// A v1 permissions.yaml described a world ADR 0004 deleted — its entries
    /// were `kind:pattern` globs over a `shell` tool that took one opaque
    /// command string. Opening a project that still holds one must not fail
    /// to start, and must not reinterpret the old lines as if they meant
    /// something under the new grammar: the file is moved aside intact and
    /// the developer starts from an empty, annotated v2 file.
    #[test]
    fn a_v1_permissions_file_is_moved_aside_rather_than_reinterpreted() {
        let project = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let dir = project.path().join(".aldwin");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("permissions.yaml"),
            "version: 1\nallow: [\"shell:cargo test*\", \"read:./**\"]\ndeny: []\n",
        )
        .unwrap();

        let config = Config::open_at(project.path(), global.path().join(".aldwin")).unwrap();

        assert_eq!(config.project_permissions(), PermissionsConfig::empty());
        assert_eq!(
            config.retired_permissions(),
            [dir.join("permissions.yaml.v1")]
        );

        let kept = std::fs::read_to_string(dir.join("permissions.yaml.v1")).unwrap();
        assert!(
            kept.contains("shell:cargo test*"),
            "the old file must survive verbatim: {kept:?}"
        );
        assert!(
            !dir.join("permissions.yaml").exists(),
            "the v1 file is moved, not copied"
        );
    }

    #[test]
    fn unknown_major_version_is_rejected() {
        let (project, _global, _config) = fresh();
        let dir = project.path().join(".aldwin");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("permissions.yaml"),
            "version: 99\nallow: []\ndeny: []\n",
        )
        .unwrap();

        let err = Config::open_at(project.path(), _global.path().join(".aldwin")).unwrap_err();
        assert!(matches!(
            err,
            ConfigError::UnknownVersion {
                found: 99,
                expected: PERMISSIONS_VERSION,
                ..
            }
        ));
    }

    #[test]
    fn empty_api_key_env_refuses_to_start() {
        let (_project, global, _config) = fresh();
        let dir = global.path().join(".aldwin");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("provider.yaml"),
            "version: 1\nprovider: anthropic\nmodel: m\napi_key_env: \"\"\n",
        )
        .unwrap();

        let err = Config::open_at(_project.path(), &dir).unwrap_err();
        assert!(matches!(err, ConfigError::MissingApiKeyEnv { .. }));
    }

    #[test]
    fn provider_yaml_without_extended_thinking_budget_still_parses() {
        // Backward compatibility: files written before this field existed
        // must keep loading, with the field defaulting to None.
        let (_project, global, _config) = fresh();
        let dir = global.path().join(".aldwin");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("provider.yaml"),
            "version: 1\nprovider: anthropic\nmodel: m\napi_key_env: X\n",
        )
        .unwrap();

        let config = Config::open_at(_project.path(), &dir).unwrap();
        assert_eq!(
            config.global_provider().unwrap().extended_thinking_budget,
            None
        );
    }

    #[test]
    fn extended_thinking_budget_round_trips_through_set_provider() {
        let (_project, _global, config) = fresh();
        let provider = ProviderConfig {
            version: PROVIDER_VERSION,
            provider: crate::domain::ProviderKind::Anthropic,
            model: "m".into(),
            base_url: None,
            api_key_env: "X".into(),
            extended_thinking_budget: Some(16_000),
        };
        config.set_provider(Scope::Global, provider).unwrap();
        assert_eq!(
            config.global_provider().unwrap().extended_thinking_budget,
            Some(16_000)
        );
    }

    #[test]
    fn raw_api_key_field_is_rejected_by_the_schema() {
        let (_project, global, _config) = fresh();
        let dir = global.path().join(".aldwin");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("provider.yaml"),
            "version: 1\nprovider: anthropic\nmodel: m\napi_key_env: X\napi_key: sk-not-allowed\n",
        )
        .unwrap();

        let err = Config::open_at(_project.path(), &dir).unwrap_err();
        assert!(matches!(err, ConfigError::Parse { .. }));
    }

    #[test]
    fn set_provider_with_empty_api_key_env_is_rejected_and_writes_nothing() {
        let (_project, _global, config) = fresh();
        let bad = ProviderConfig {
            version: PROVIDER_VERSION,
            provider: crate::domain::ProviderKind::Anthropic,
            model: "m".into(),
            base_url: None,
            api_key_env: "".into(),
            extended_thinking_budget: None,
        };
        let err = config.set_provider(Scope::Global, bad).unwrap_err();
        assert!(matches!(err, ConfigError::MissingApiKeyEnv { .. }));
        assert!(!_global
            .path()
            .join(".aldwin")
            .join("provider.yaml")
            .exists());
        assert!(config.global_provider().is_err());
    }

    #[test]
    fn mcp_server_unknown_field_is_rejected_despite_flattened_transport() {
        // McpServer flattens McpTransport into itself; flatten + deny_unknown_fields
        // is a known serde trouble spot, so this checks the combination actually
        // still rejects a bogus field rather than silently accepting it.
        let bad = "name: fs\nkind: stdio\ncommand: fs-server\nbogus: 1\n";
        let result: Result<McpServer, _> = serde_yaml_ng::from_str(bad);
        assert!(
            result.is_err(),
            "expected bogus field to be rejected, got {result:?}"
        );
    }

    #[test]
    fn add_mcp_server_upserts_by_name_replacing_wholesale() {
        let (_project, _global, config) = fresh();
        config
            .add_mcp_server(
                Scope::Global,
                McpServer {
                    name: "fs".into(),
                    transport: McpTransport::Stdio {
                        command: "fs-server".into(),
                        args: vec![],
                    },
                    env: Default::default(),
                },
            )
            .unwrap();
        config
            .add_mcp_server(
                Scope::Global,
                McpServer {
                    name: "fs".into(),
                    transport: McpTransport::Http {
                        url: "http://localhost:9/".into(),
                    },
                    env: Default::default(),
                },
            )
            .unwrap();

        let servers = config.global_mcp().servers;
        assert_eq!(servers.len(), 1);
        assert!(matches!(servers[0].transport, McpTransport::Http { .. }));
    }

    #[test]
    fn reload_all_retains_previous_snapshot_on_parse_failure_but_names_the_file() {
        let (project, _global, config) = fresh();
        config
            .add_grant(
                Scope::Project,
                GrantList::Allow,
                GrantEntry::classed("rg", Class::Read),
            )
            .unwrap();
        config
            .add_grant(
                Scope::Global,
                GrantList::Allow,
                GrantEntry::classed("cargo", Class::Write),
            )
            .unwrap();

        // Hand-edit project permissions.yaml into garbage, but leave global alone.
        let dir = project.path().join(".aldwin");
        std::fs::write(dir.join("permissions.yaml"), "not: [valid, yaml: at all").unwrap();

        let result = config.reload_all();
        let failures = result.unwrap_err();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].path, dir.join("permissions.yaml"));

        // Previous snapshot retained for the broken layer...
        assert_eq!(
            config.project_permissions().allow,
            vec![GrantEntry::classed("rg", Class::Read)]
        );
        // ...while an unrelated, still-valid layer still reloads fine.
        assert_eq!(
            config.global_permissions().allow,
            vec![GrantEntry::classed("cargo", Class::Write)]
        );
    }

    #[test]
    fn reload_all_picks_up_hand_edits_that_are_still_valid() {
        let (project, _global, config) = fresh();
        let dir = project.path().join(".aldwin");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("permissions.yaml"),
            "version: 2\ndefault: read\nallow:\n  - git: read\n  - curl\ndeny: []\n",
        )
        .unwrap();

        config.reload_all().unwrap();
        let cfg = config.project_permissions();
        assert_eq!(cfg.default, Some(Rung::Read));
        assert_eq!(
            cfg.allow,
            vec![
                GrantEntry::classed("git", Class::Read),
                GrantEntry::program("curl")
            ]
        );
    }

    #[test]
    fn clones_share_the_same_in_memory_state() {
        let (_project, _global, config) = fresh();
        let other = config.clone();
        config
            .add_grant(
                Scope::Global,
                GrantList::Allow,
                GrantEntry::classed("rg", Class::Read),
            )
            .unwrap();
        assert_eq!(
            other.global_permissions().allow,
            vec![GrantEntry::classed("rg", Class::Read)]
        );
    }
}
