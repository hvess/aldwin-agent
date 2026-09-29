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
    /// `~/.aldwin/` did not exist; created it with the three annotated files
    /// (permissions, mcp, tui).
    Created,
    /// `~/.aldwin/` exists with all three seeded files.
    AlreadyPresent,
    /// `~/.aldwin/` exists but lacks a seeded file. The caller must refuse
    /// to start, not fill the gap.
    PartiallyPresent {
        /// The absent file names, in the order init writes them.
        missing: Vec<&'static str>,
    },
}

/// One (scope, domain) layer that failed to reload; its previous in-memory
/// snapshot is kept.
#[derive(Debug)]
pub struct ReloadFailure {
    /// The file that failed.
    pub path: PathBuf,
    /// Why it failed.
    pub error: ConfigError,
}

impl ReloadFailure {
    /// Each failure as `path: error`, `; `-separated: the one wording for
    /// `/reload` and aldwin-tools' `reload`.
    pub fn describe(failures: &[Self]) -> String {
        failures
            .iter()
            .map(|f| format!("{}: {}", f.path.display(), f.error))
            .collect::<Vec<_>>()
            .join("; ")
    }
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

/// Typed access to Aldwin's on-disk config. Clones share one `Arc`'d set of
/// snapshots. Reads never touch disk: they answer from the snapshot loaded
/// by [`Config::open`] or refreshed by [`Config::reload_all`].
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

/// Loads `provider.yaml` and refuses an empty `api_key_env`, which the
/// schema alone accepts. Every provider read goes through here so all refuse
/// the same file the same way.
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

/// Renames a legacy global dir (`~/.mjolnir/`, then `~/.amundsen/`) to
/// `new_dir` when `new_dir` does not exist.
///
/// Newest name first; only one is moved, never merged, since merging would
/// pick between two files silently. Best-effort and must never block
/// startup: a failed rename leaves `new_dir` absent, which
/// `init_global_if_empty` treats as a fresh install.
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
    /// Reads every existing layer once, with global scope at `~/.aldwin/`
    /// after migrating a legacy dir. Tests use [`Config::open_at`] so they
    /// never touch the real home directory.
    ///
    /// # Errors
    ///
    /// [`ConfigError::NoHomeDir`] when there is no home directory, otherwise
    /// whatever [`Config::open_at`] refuses.
    pub fn open(project_root: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let home = dirs::home_dir().ok_or(ConfigError::NoHomeDir)?;
        let global_dir = home.join(".aldwin");
        migrate_legacy_global_dir(&home, &global_dir);
        Self::open_at(project_root, global_dir)
    }

    /// Reads every existing layer once, with global scope at `global_dir`. A
    /// missing file becomes its domain's empty value, or `None` for provider,
    /// which has no meaningful empty value.
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

    /// This project's transcript directory, `~/.aldwin/history/<slug>/`.
    ///
    /// Never inside the project's `.aldwin/`: a transcript holds whatever
    /// tool results held, and the project tree may be committed.
    pub fn history_dir(&self) -> PathBuf {
        let project_root = self
            .inner
            .project_dir
            .parent()
            .unwrap_or(&self.inner.project_dir);
        crate::history::project_dir(&self.inner.global_dir.join("history"), project_root)
    }

    /// The project's `.aldwin/permissions.yaml`, whose `roots:` widen the
    /// workspace; it need not exist.
    pub fn project_permissions_path(&self) -> PathBuf {
        self.domain_path(Scope::Project, "permissions")
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

    /// The `permissions.yaml` files, project first, whose `allow:`,
    /// `default:` or `deny:` says something ([`PermissionsConfig::has_stale_keys`]);
    /// nothing reads those keys since ADR 0011, and the developer is told once.
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

    /// Clone, mutate, write atomically, swap, all under one held write lock.
    /// Do not narrow the lock to the swap: another mutator or `reload_all`
    /// would clobber this write on disk or in memory. Every domain write
    /// must go through here.
    ///
    /// `header` is an `annotated::*_HEADER` constant, or `""` for a domain
    /// with none.
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

    /// Stores the account connected to `provider`, replacing any it had;
    /// written by `/connect` and by every refresh that rotates the token.
    /// Global only (ADR 0012), so a token never lands in a repository. The
    /// file is owner-only (0600) because the tempfile the atomic write
    /// renames is.
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

    /// Forgets the account connected to `provider`, after its refresh token
    /// is revoked, so the next client falls back to the API key.
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

    /// Re-reads every layer, for `/reload` and `reload`; a missing file becomes its
    /// empty value. A layer that fails keeps its previous snapshot, so a bad
    /// hand edit cannot break a running session.
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
        // Lock before the read: otherwise a `with_domain_mut` write landing
        // between them is reverted in memory.
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

    /// Creates `~/.aldwin/` with the annotated permissions, mcp and tui files
    /// if it does not exist; an existing directory is only checked for them.
    ///
    /// Never seed `provider.yaml`: any value would be a model nobody chose,
    /// and its absence (`global_provider().is_err()`) is what makes the
    /// session ask the provider question (the launch card's `Model  not set`).
    ///
    /// # Errors
    ///
    /// [`ConfigError::Io`] when a file cannot be written, and whatever
    /// reading it back refuses.
    ///
    /// # Panics
    ///
    /// If a writer panicked while holding a domain's lock, or if a file just
    /// written does not read back (annotated text that fails its schema).
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

        // `provider.yaml` is not required: without it the provider question
        // is asked. The other three are written together at init, so one
        // missing means a half-deleted directory.
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
// Pins the Pitfalls in `docs/spec/archive/aldwin-config.md`.

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

    /// Project and global temp roots and a `Config` opened on them; neither
    /// `.aldwin/` exists yet.
    fn fresh() -> (tempfile::TempDir, tempfile::TempDir, Config) {
        let project = tempdir().unwrap();
        let global = tempdir().unwrap();
        // A subdirectory, not the tempdir itself, so existence checks start
        // from absence.
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

        // The snapshot is what was written, and it reports nothing stale.
        assert_eq!(config.global_permissions(), PermissionsConfig::empty());
        assert!(config.stale_permissions().is_empty());

        assert_eq!(
            config.init_global_if_empty().unwrap(),
            InitOutcome::AlreadyPresent
        );
    }

    /// Regression: a seeded `provider.yaml` meant the provider question was
    /// never asked.
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

    /// Regression: re-serialising dropped the annotated header on the first
    /// write.
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

    /// ADR 0007 §1. A misspelt `roots` must fail, not silently widen nothing.
    #[test]
    fn roots_are_read_from_a_permissions_file_and_a_misspelling_is_an_error() {
        let parsed: PermissionsConfig =
            serde_yaml_ng::from_str("version: 2\nroots:\n- ../shared-lib\n- /abs/other\n").unwrap();
        assert_eq!(
            parsed.roots,
            vec![PathBuf::from("../shared-lib"), PathBuf::from("/abs/other")]
        );

        let none: PermissionsConfig = serde_yaml_ng::from_str("version: 2\n").unwrap();
        assert!(none.roots.is_empty());

        assert!(
            serde_yaml_ng::from_str::<PermissionsConfig>("version: 2\nroot:\n- ../x\n").is_err()
        );
    }

    /// ADR 0011: an old file must load, or an existing project cannot start;
    /// its stale keys are reported, not honoured.
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

    /// Earlier first launches wrote `deny: []`; reporting it would be noise
    /// on every existing install.
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

        // A fresh open reads it back; a reload after a hand edit that deletes
        // an entry disconnects it.
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
        // Files older than this field must keep loading.
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

    /// Regression: a project `provider.yaml` with no global one left the
    /// session unconfigured.
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
        // Unknown fields are refused by the flattened `McpTransport`, not by
        // `McpServer`; see `McpTransport`'s doc.
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

        std::fs::write(&path, "not: [valid, yaml: at all").unwrap();
        std::fs::write(
            global.path().join(".aldwin").join("tui.yaml"),
            "version: 1\ntheme: light\n",
        )
        .unwrap();

        let failures = config.reload_all().unwrap_err();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].path, path);

        assert_eq!(
            config.project_permissions().roots,
            [PathBuf::from("../libs")]
        );
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
