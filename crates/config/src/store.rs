use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use crate::{
    annotated,
    domain::{
        ConnectionRecord, ConnectionsConfig, McpConfig, PermissionsConfig, ProviderConfig,
        TuiConfig, CONNECTIONS_VERSION, MCP_VERSION, PERMISSIONS_VERSION, PROVIDER_VERSION,
        TUI_VERSION,
    },
    error::ConfigError,
    fsio,
    scope::Scope,
};

/// Result of [`Config::init_global_if_empty`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InitOutcome {
    /// `~/.aldwin/` did not exist; created it and wrote all four annotated files.
    Created,
    /// `~/.aldwin/` exists and all four domain files are present.
    AlreadyPresent,
    /// `~/.aldwin/` exists but is missing one or more domain files. The
    /// caller must refuse to start rather than auto-fill the gap.
    PartiallyPresent {
        /// The file names that are absent, in the order init writes them.
        missing: Vec<&'static str>,
    },
}

/// One (scope, domain) layer that failed to reload; the previous in-memory
/// snapshot for that layer is left untouched.
#[derive(Debug)]
pub struct ReloadFailure {
    /// The file that failed, so the developer can be told which.
    pub path: PathBuf,
    /// Why it failed.
    pub error: ConfigError,
}

struct Inner {
    project_dir: PathBuf,
    global_dir: PathBuf,

    project_permissions: RwLock<PermissionsConfig>,
    global_permissions: RwLock<PermissionsConfig>,
    project_provider: RwLock<Option<ProviderConfig>>,
    global_provider: RwLock<Option<ProviderConfig>>,
    project_mcp: RwLock<McpConfig>,
    global_mcp: RwLock<McpConfig>,
    global_tui: RwLock<TuiConfig>,
    global_connections: RwLock<ConnectionsConfig>,
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

impl Config {
    /// Read every existing layer once, resolving global scope to
    /// `~/.aldwin/`. See [`Config::open_at`] for the same thing with an
    /// explicit global root (used by tests, so they never touch the real
    /// home directory).
    ///
    /// # Errors
    ///
    /// [`ConfigError::NoHomeDir`] when there is no home directory, and
    /// otherwise whatever [`Config::open_at`] refuses.
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
    ///
    /// # Errors
    ///
    /// [`ConfigError::Io`] for a file that exists but cannot be read,
    /// [`ConfigError::Parse`] for one that is not valid for its schema,
    /// [`ConfigError::UnknownVersion`] for one from a newer build, and
    /// [`ConfigError::MissingApiKeyEnv`] for a `provider.yaml` with an empty
    /// `api_key_env`.
    pub fn open_at(
        project_root: impl AsRef<Path>,
        global_dir: impl Into<PathBuf>,
    ) -> Result<Self, ConfigError> {
        let global_dir = global_dir.into();
        let project_dir = project_root.as_ref().join(".aldwin");

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
        let global_connections =
            fsio::read_versioned(&global_dir.join("connections.yaml"), CONNECTIONS_VERSION)?
                .unwrap_or_else(ConnectionsConfig::empty);

        Ok(Self {
            inner: Arc::new(Inner {
                project_dir,
                global_dir,
                project_permissions: RwLock::new(project_permissions),
                global_permissions: RwLock::new(global_permissions),
                project_provider: RwLock::new(project_provider),
                global_provider: RwLock::new(global_provider),
                project_mcp: RwLock::new(project_mcp),
                global_mcp: RwLock::new(global_mcp),
                global_tui: RwLock::new(global_tui),
                global_connections: RwLock::new(global_connections),
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

    /// The project's `permissions.yaml`, or an empty one when it has none.
    ///
    /// # Panics
    ///
    /// If a writer panicked while holding this domain's lock.
    pub fn project_permissions(&self) -> PermissionsConfig {
        self.inner
            .project_permissions
            .read()
            .expect("lock poisoned")
            .clone()
    }

    /// The global `permissions.yaml`, or an empty one when there is none.
    ///
    /// # Panics
    ///
    /// If a writer panicked while holding this domain's lock.
    pub fn global_permissions(&self) -> PermissionsConfig {
        self.inner
            .global_permissions
            .read()
            .expect("lock poisoned")
            .clone()
    }

    /// `None` means no project-scope override — fall back to `global_provider`.
    ///
    /// # Panics
    ///
    /// If a writer panicked while holding this domain's lock.
    pub fn project_provider(&self) -> Option<ProviderConfig> {
        self.inner
            .project_provider
            .read()
            .expect("lock poisoned")
            .clone()
    }

    /// Unlike `project_provider`, absence here is an error: there is no
    /// meaningful default model or `api_key_env` to fall back to.
    ///
    /// # Errors
    ///
    /// [`ConfigError::ProviderNotConfigured`] when there is no global
    /// `provider.yaml`.
    ///
    /// # Panics
    ///
    /// If a writer panicked while holding this domain's lock.
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

    /// The provider settings in force: the project file over the global one
    /// ([`ProviderConfig::over`]), either alone, or `None` when neither
    /// exists. What a session is built on, at startup and after `/model`.
    pub fn effective_provider(&self) -> Option<ProviderConfig> {
        let global = self.global_provider().ok();
        match self.project_provider() {
            Some(project) => Some(project.over(global.as_ref())),
            None => global,
        }
    }

    /// The MCP servers the project's `mcp.yaml` names.
    ///
    /// # Panics
    ///
    /// If a writer panicked while holding this domain's lock.
    pub fn project_mcp(&self) -> McpConfig {
        self.inner
            .project_mcp
            .read()
            .expect("lock poisoned")
            .clone()
    }

    /// The MCP servers the global `mcp.yaml` names.
    ///
    /// # Panics
    ///
    /// If a writer panicked while holding this domain's lock.
    pub fn global_mcp(&self) -> McpConfig {
        self.inner.global_mcp.read().expect("lock poisoned").clone()
    }

    /// The TUI's settings; `tui.yaml` is global only.
    ///
    /// # Panics
    ///
    /// If a writer panicked while holding this domain's lock.
    pub fn global_tui(&self) -> TuiConfig {
        self.inner.global_tui.read().expect("lock poisoned").clone()
    }

    /// The account connected to `provider` — a catalogue id — or `None`
    /// when the developer has not connected one (ADR 0012).
    ///
    /// # Panics
    ///
    /// If a writer panicked while holding this domain's lock.
    pub fn connection(&self, provider: &str) -> Option<ConnectionRecord> {
        self.inner
            .global_connections
            .read()
            .expect("lock poisoned")
            .accounts
            .get(provider)
            .cloned()
    }

    /// The `permissions.yaml` files that still say something through a key
    /// nothing reads — `allow:`, `default:` or `deny:` — nearest first, so
    /// the developer can be told once that the file promises what the
    /// product no longer does (ADR 0011).
    pub fn stale_permissions(&self) -> Vec<PathBuf> {
        [Scope::Project, Scope::Global]
            .into_iter()
            .filter(|&scope| {
                let permissions = match scope {
                    Scope::Project => self.project_permissions(),
                    Scope::Global => self.global_permissions(),
                };
                permissions.has_stale_keys()
            })
            .map(|scope| self.domain_path(scope, "permissions"))
            .collect()
    }

    // ── Write ────────────────────────────────────────────────────────────

    /// Read → mutate → persist-atomically → swap, all under one *held*
    /// write lock — not just the final swap. A concurrent writer (another
    /// mutator on this domain, or `reload_all` re-reading it from disk) must
    /// block until this call has landed on both disk and memory, or one of
    /// the two silently clobbers the other. Every domain-mutating method in
    /// this file goes through here so the locking cannot drift between
    /// domains.
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

    /// Writes `provider.yaml` at `scope`, replacing what it said.
    ///
    /// # Errors
    ///
    /// [`ConfigError::MissingApiKeyEnv`] when `api_key_env` is empty, before
    /// anything is written; [`ConfigError::Io`] or
    /// [`ConfigError::Serialize`] when the file cannot be written.
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

    /// Writes the global `tui.yaml`, replacing what it said.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Io`] or [`ConfigError::Serialize`] when the file cannot
    /// be written; the in-memory snapshot is then left as it was.
    pub fn set_tui(&self, tui: TuiConfig) -> Result<(), ConfigError> {
        let path = self.domain_path(Scope::Global, "tui");
        self.with_domain_mut(
            &self.inner.global_tui,
            &path,
            annotated::TUI_HEADER,
            move |current| *current = tui,
        )
    }

    /// Stores the account connected to `provider`, replacing any it had:
    /// what `/connect` writes, and what every refresh that rotates the
    /// token writes again. Global scope only, by decision (ADR 0012): a
    /// token inside a repository is a leak waiting to be committed. The
    /// atomic writer creates the file owner-only.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Io`] or [`ConfigError::Serialize`] when the file cannot
    /// be written; the in-memory snapshot is then left as it was.
    pub fn set_connection(
        &self,
        provider: &str,
        record: ConnectionRecord,
    ) -> Result<(), ConfigError> {
        let path = self.domain_path(Scope::Global, "connections");
        let provider = provider.to_string();
        self.with_domain_mut(
            &self.inner.global_connections,
            &path,
            annotated::CONNECTIONS_HEADER,
            move |current| {
                current.accounts.insert(provider, record);
            },
        )
    }

    /// Forgets the account connected to `provider`: what a revoked refresh
    /// token leads to, so the next client built on the provider falls back
    /// to its key rather than to tokens the server will refuse.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Io`] or [`ConfigError::Serialize`] when the file cannot
    /// be written; the in-memory snapshot is then left as it was.
    pub fn remove_connection(&self, provider: &str) -> Result<(), ConfigError> {
        let path = self.domain_path(Scope::Global, "connections");
        let provider = provider.to_string();
        self.with_domain_mut(
            &self.inner.global_connections,
            &path,
            annotated::CONNECTIONS_HEADER,
            move |current| {
                current.accounts.remove(&provider);
            },
        )
    }

    // ── Reload ───────────────────────────────────────────────────────────

    /// Re-read every layer that currently exists on disk. A layer that fails
    /// to parse keeps its previous in-memory snapshot — a bad hand-edit must
    /// not collapse an in-progress session — and is reported by path so the
    /// caller (the TUI's `/reload-config` handler) can name the failing file.
    ///
    /// # Errors
    ///
    /// One [`ReloadFailure`] for every layer that could not be re-read; the
    /// layers that could are reloaded all the same.
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
            &self.inner.global_connections,
            self.domain_path(Scope::Global, "connections"),
            CONNECTIONS_VERSION,
            ConnectionsConfig::empty,
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
    /// here has a meaningful empty value — no roots, no MCP servers, no
    /// theme override — so writing one states nothing on the developer's
    /// behalf. A provider does not: any file this could write would name a
    /// host, a model and a key variable nobody chose. It used to write
    /// `anthropic` / `claude-sonnet-5`, and because that ran *before* the
    /// session's own check (`global_provider().is_err()`), the
    /// provider question was never once asked — the seed had already
    /// answered it. Leaving the file absent is what makes "no provider is
    /// configured" a real state, and it is the state the launch card's `Model  not set` exists to
    /// resolve.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Io`] when a file cannot be written, and whatever
    /// reading it back refuses.
    ///
    /// # Panics
    ///
    /// If a writer panicked while holding a domain's lock, or if a file this
    /// just wrote does not read back — the annotated text disagreeing with
    /// its own schema.
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
// version-bump rejection, old permissions files still loading, partial-init refusing
// to start, reload retaining the previous snapshot on a bad file while still
// naming it, and project scope not materialising until first write.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::McpServer;
    use tempfile::tempdir;

    fn provider(model: &str) -> ProviderConfig {
        ProviderConfig {
            version: PROVIDER_VERSION,
            provider: crate::domain::ProviderKind::Anthropic,
            model: model.into(),
            base_url: None,
            api_key_env: "ANTHROPIC_API_KEY".into(),
            extended_thinking_budget: None,
        }
    }

    fn record(access_token: &str) -> ConnectionRecord {
        ConnectionRecord {
            access_token: access_token.into(),
            refresh_token: "refresh".into(),
            expires_at: 1_800_000_000,
        }
    }

    fn write_project_permissions(project: &Path, text: &str) -> PathBuf {
        let dir = project.join(".aldwin");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("permissions.yaml");
        std::fs::write(&path, text).unwrap();
        path
    }

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

        config.set_provider(Scope::Project, provider("m")).unwrap();
        assert!(aldwin_dir.is_dir());
        assert!(aldwin_dir.join("provider.yaml").is_file());
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
        // defaults — and a fresh file says nothing a notice would report.
        assert_eq!(config.global_permissions(), PermissionsConfig::empty());
        assert!(config.stale_permissions().is_empty());

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

    /// Regression, reported as "editing permissions.yaml doesn't really
    /// appear to make any sense": a plain re-serialise drops every comment,
    /// so the annotated explanation survived only until the *first* write.
    #[test]
    fn provider_and_tui_yaml_keep_their_headers_after_a_write() {
        let (_project, global, config) = fresh();
        config
            .set_provider(Scope::Global, provider("claude-sonnet-5"))
            .unwrap();
        config
            .set_tui(TuiConfig {
                theme: Some("dark".into()),
                ..TuiConfig::empty()
            })
            .unwrap();

        let provider_text =
            std::fs::read_to_string(global.path().join(".aldwin").join("provider.yaml")).unwrap();
        let tui_text =
            std::fs::read_to_string(global.path().join(".aldwin").join("tui.yaml")).unwrap();
        assert!(
            provider_text.starts_with("# Aldwin provider settings"),
            "{provider_text:?}"
        );
        assert!(
            tui_text.starts_with("# Aldwin TUI preferences"),
            "{tui_text:?}"
        );
    }

    /// ADR 0007 §1. `roots` is read, and `deny_unknown_fields` still rejects
    /// a misspelling rather than silently ignoring the reach a developer
    /// thought they had declared.
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

        assert!(
            serde_yaml_ng::from_str::<PermissionsConfig>("version: 2\nroot:\n- ../x\n").is_err()
        );
    }

    /// Files from every earlier model still load — a v1 file's `kind:pattern`
    /// globs, a v2 file's rung and grants, a deny list — because refusing
    /// one would stop an existing project from starting. What they say is
    /// reported, once, rather than honoured (ADR 0011).
    #[test]
    fn a_permissions_file_from_an_earlier_model_loads_and_is_reported_stale() {
        for text in [
            "version: 1\nallow: [\"shell:cargo test*\", \"read:./**\"]\ndeny: []\n",
            "version: 2\ndefault: read\nallow:\n  - git: read\n  - curl\n",
            "version: 2\ndeny:\n  - curl\n  - npm: write\nroots:\n  - ../libs\n",
        ] {
            let (project, _global, _config) = fresh();
            let path = write_project_permissions(project.path(), text);
            let config = Config::open_at(project.path(), _global.path().join(".aldwin")).unwrap();
            assert_eq!(config.stale_permissions(), vec![path], "{text}");
        }
    }

    /// The `deny: []` every first launch wrote says nothing, and a notice
    /// about it would be noise on every existing install.
    #[test]
    fn an_empty_list_or_roots_alone_is_not_stale() {
        let (project, _global, config) = fresh();
        write_project_permissions(
            project.path(),
            "version: 2\nallow: []\ndeny: []\nroots:\n  - ../libs\n",
        );
        config.reload_all().unwrap();
        assert!(config.stale_permissions().is_empty());
        assert_eq!(
            config.project_permissions().roots,
            [PathBuf::from("../libs")]
        );
    }

    #[test]
    fn unknown_major_version_is_rejected() {
        let (project, _global, _config) = fresh();
        let dir = project.path().join(".aldwin");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("permissions.yaml"), "version: 99\n").unwrap();

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
    fn a_connection_is_stored_globally_owner_only_and_read_back() {
        let (_project, global, config) = fresh();
        assert_eq!(config.connection("xai"), None);

        config.set_connection("xai", record("first")).unwrap();
        config.set_connection("xai", record("second")).unwrap();
        config.set_connection("other", record("third")).unwrap();

        assert_eq!(config.connection("xai"), Some(record("second")));
        assert_eq!(config.connection("other"), Some(record("third")));
        config.remove_connection("other").unwrap();
        assert_eq!(config.connection("other"), None);
        let path = global.path().join(".aldwin").join("connections.yaml");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# Aldwin connected accounts"), "{text:?}");
        assert!(
            !_project.path().join(".aldwin").exists(),
            "a connection never lands at project scope"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "the file holds tokens");
        }

        // What was written is what a fresh open reads, and what a reload
        // picks up after a hand edit — deleting an entry disconnects it.
        let reopened = Config::open_at(_project.path(), global.path().join(".aldwin")).unwrap();
        assert_eq!(reopened.connection("xai"), Some(record("second")));
        std::fs::write(&path, "version: 1\naccounts:\n  other:\n    access_token: a\n    refresh_token: r\n    expires_at: 1\n").unwrap();
        config.reload_all().unwrap();
        assert_eq!(config.connection("xai"), None);
    }

    #[test]
    fn a_connection_records_debug_output_carries_no_token() {
        let printed = format!("{:?}", record("the-access-token"));
        assert!(!printed.contains("the-access-token"), "{printed}");
        assert!(!printed.contains("refresh"), "{printed}");
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
            extended_thinking_budget: Some(16_000),
            ..provider("m")
        };
        config.set_provider(Scope::Global, provider).unwrap();
        assert_eq!(
            config.global_provider().unwrap().extended_thinking_budget,
            Some(16_000)
        );
    }

    /// A project `provider.yaml` with no global one used to boot the
    /// session unconfigured: the overlay needed a global file to lay the
    /// project over.
    #[test]
    fn a_project_provider_alone_is_in_force() {
        let (_project, _global, config) = fresh();
        assert_eq!(config.effective_provider(), None);
        config
            .set_provider(Scope::Project, provider("project-model"))
            .unwrap();
        assert_eq!(
            config.effective_provider().map(|p| p.model),
            Some("project-model".to_string())
        );
    }

    #[test]
    fn the_project_provider_wins_and_its_unset_fields_fall_back_to_global() {
        let (_project, _global, config) = fresh();
        let global = ProviderConfig {
            base_url: Some("https://global".into()),
            extended_thinking_budget: Some(20_000),
            ..provider("global-model")
        };
        config.set_provider(Scope::Global, global.clone()).unwrap();
        assert_eq!(config.effective_provider(), Some(global));

        config
            .set_provider(Scope::Project, provider("project-model"))
            .unwrap();
        let effective = config.effective_provider().unwrap();
        assert_eq!(effective.model, "project-model");
        assert_eq!(effective.base_url.as_deref(), Some("https://global"));
        assert_eq!(effective.extended_thinking_budget, Some(20_000));

        let project = ProviderConfig {
            base_url: Some("https://project".into()),
            extended_thinking_budget: Some(1_000),
            ..provider("project-model")
        };
        config
            .set_provider(Scope::Project, project.clone())
            .unwrap();
        assert_eq!(config.effective_provider(), Some(project));
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
            api_key_env: "".into(),
            ..provider("m")
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
    fn reload_all_retains_previous_snapshot_on_parse_failure_but_names_the_file() {
        let (project, global, config) = fresh();
        let path = write_project_permissions(project.path(), "version: 2\nroots: [../libs]\n");
        config.set_tui(TuiConfig::empty()).unwrap();
        config.reload_all().unwrap();

        // Hand-edit project permissions.yaml into garbage, and the global
        // tui.yaml into something new but valid.
        std::fs::write(&path, "not: [valid, yaml: at all").unwrap();
        std::fs::write(
            global.path().join(".aldwin").join("tui.yaml"),
            "version: 1\ntheme: light\n",
        )
        .unwrap();

        let failures = config.reload_all().unwrap_err();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].path, path);

        // Previous snapshot retained for the broken layer...
        assert_eq!(
            config.project_permissions().roots,
            [PathBuf::from("../libs")]
        );
        // ...while an unrelated, still-valid layer still reloads fine.
        assert_eq!(config.global_tui().theme.as_deref(), Some("light"));
    }

    #[test]
    fn clones_share_the_same_in_memory_state() {
        let (_project, _global, config) = fresh();
        let other = config.clone();
        config
            .set_tui(TuiConfig {
                theme: Some("light".into()),
                ..TuiConfig::empty()
            })
            .unwrap();
        assert_eq!(other.global_tui().theme.as_deref(), Some("light"));
    }

    #[test]
    fn mcp_servers_are_read_from_their_file() {
        let (_project, global, _config) = fresh();
        let dir = global.path().join(".aldwin");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("mcp.yaml"),
            "version: 1\nservers:\n  - name: fs\n    kind: stdio\n    command: fs-server\n",
        )
        .unwrap();
        let config = Config::open_at(_project.path(), &dir).unwrap();
        assert_eq!(
            config.global_mcp().servers,
            vec![McpServer {
                name: "fs".into(),
                transport: crate::domain::McpTransport::Stdio {
                    command: "fs-server".into(),
                    args: vec![],
                },
                env: Default::default(),
            }]
        );
    }
}
