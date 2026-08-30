use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use crate::{
    annotated,
    domain::{
        ContextFilesConfig, McpConfig, McpServer, PermissionsConfig, ProviderConfig,
        CONTEXT_FILES_VERSION, MCP_VERSION, PERMISSIONS_VERSION, PROVIDER_VERSION, TUI_VERSION,
    },
    domain::TuiConfig,
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
    /// `~/.mjolnir/` did not exist; created it and wrote all four annotated files.
    Created,
    /// `~/.mjolnir/` exists and all four domain files are present.
    AlreadyPresent,
    /// `~/.mjolnir/` exists but is missing one or more domain files. The
    /// caller must refuse to start rather than auto-fill the gap.
    PartiallyPresent { missing: Vec<&'static str> },
}

/// One (scope, domain) layer that failed to reload; the previous in-memory
/// snapshot for that layer is left untouched.
#[derive(Debug)]
pub struct ReloadFailure {
    pub path:  PathBuf,
    pub error: ConfigError,
}

struct Inner {
    project_dir: PathBuf,
    global_dir:  PathBuf,

    project_permissions: RwLock<PermissionsConfig>,
    global_permissions:  RwLock<PermissionsConfig>,
    project_provider:    RwLock<Option<ProviderConfig>>,
    global_provider:     RwLock<Option<ProviderConfig>>,
    project_mcp:         RwLock<McpConfig>,
    global_mcp:          RwLock<McpConfig>,
    global_tui:          RwLock<TuiConfig>,
    project_context_files: RwLock<ContextFilesConfig>,
}

/// Typed access to Mjolnir's on-disk config. Cheap to clone — internally an
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
        return Err(ConfigError::MissingApiKeyEnv { path: path.to_path_buf() });
    }
    Ok(Some(cfg))
}

/// One-time best-effort migration for the Amundsen→Mjolnir rebrand:
/// existing installs have their config at the old `~/.amundsen/`. If the
/// new `~/.mjolnir/` doesn't exist yet but the old one does, move it over
/// so a rebuild-and-reinstall doesn't silently orphan a developer's
/// existing permissions grants and provider config behind a renamed
/// directory `Config::open` no longer looks at. Best-effort: a failed
/// rename (e.g. a cross-device home directory) just leaves `global_dir`
/// nonexistent, which `init_global_if_empty` already treats as a normal
/// fresh install — migration must never block startup.
fn migrate_legacy_global_dir(home: &Path, new_dir: &Path) {
    if new_dir.exists() {
        return;
    }
    let legacy_dir = home.join(".amundsen");
    if legacy_dir.exists() {
        let _ = std::fs::rename(&legacy_dir, new_dir);
    }
}

impl Config {
    /// Read every existing layer once, resolving global scope to
    /// `~/.mjolnir/`. See [`Config::open_at`] for the same thing with an
    /// explicit global root (used by tests, so they never touch the real
    /// home directory).
    pub fn open(project_root: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let home = dirs::home_dir().ok_or(ConfigError::NoHomeDir)?;
        let global_dir = home.join(".mjolnir");
        migrate_legacy_global_dir(&home, &global_dir);
        Self::open_at(project_root, global_dir)
    }

    /// Read every existing layer once. Missing files are the normal "nothing
    /// persisted here yet" state and become empty defaults (or `None` for
    /// provider, which has no meaningful empty state) — only a malformed
    /// file, an unknown version, or an empty `api_key_env` refuses to start.
    pub fn open_at(
        project_root: impl AsRef<Path>,
        global_dir:   impl Into<PathBuf>,
    ) -> Result<Self, ConfigError> {
        let global_dir = global_dir.into();
        let project_dir = project_root.as_ref().join(".mjolnir");

        let project_permissions =
            fsio::read_versioned(&project_dir.join("permissions.yaml"), PERMISSIONS_VERSION)?
                .unwrap_or_else(PermissionsConfig::empty);
        let global_permissions =
            fsio::read_versioned(&global_dir.join("permissions.yaml"), PERMISSIONS_VERSION)?
                .unwrap_or_else(PermissionsConfig::empty);

        let project_provider = load_provider(&project_dir.join("provider.yaml"))?;
        let global_provider  = load_provider(&global_dir.join("provider.yaml"))?;

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
                project_permissions: RwLock::new(project_permissions),
                global_permissions:  RwLock::new(global_permissions),
                project_provider:    RwLock::new(project_provider),
                global_provider:     RwLock::new(global_provider),
                project_mcp: RwLock::new(project_mcp),
                global_mcp:  RwLock::new(global_mcp),
                global_tui:  RwLock::new(global_tui),
                project_context_files: RwLock::new(project_context_files),
            }),
        })
    }

    fn scope_dir(&self, scope: Scope) -> &Path {
        match scope {
            Scope::Project => &self.inner.project_dir,
            Scope::Global  => &self.inner.global_dir,
        }
    }

    fn domain_path(&self, scope: Scope, domain: &str) -> PathBuf {
        self.scope_dir(scope).join(format!("{domain}.yaml"))
    }

    // ── Read ─────────────────────────────────────────────────────────────

    pub fn project_permissions(&self) -> PermissionsConfig {
        self.inner.project_permissions.read().expect("lock poisoned").clone()
    }

    pub fn global_permissions(&self) -> PermissionsConfig {
        self.inner.global_permissions.read().expect("lock poisoned").clone()
    }

    /// `None` means no project-scope override — fall back to `global_provider`.
    pub fn project_provider(&self) -> Option<ProviderConfig> {
        self.inner.project_provider.read().expect("lock poisoned").clone()
    }

    /// Unlike `project_provider`, absence here is an error: there is no
    /// meaningful default model or `api_key_env` to fall back to.
    pub fn global_provider(&self) -> Result<ProviderConfig, ConfigError> {
        self.inner.global_provider.read().expect("lock poisoned").clone().ok_or_else(|| {
            ConfigError::ProviderNotConfigured { path: self.domain_path(Scope::Global, "provider") }
        })
    }

    pub fn project_mcp(&self) -> McpConfig {
        self.inner.project_mcp.read().expect("lock poisoned").clone()
    }

    pub fn global_mcp(&self) -> McpConfig {
        self.inner.global_mcp.read().expect("lock poisoned").clone()
    }

    pub fn global_tui(&self) -> TuiConfig {
        self.inner.global_tui.read().expect("lock poisoned").clone()
    }

    pub fn project_context_files(&self) -> ContextFilesConfig {
        self.inner.project_context_files.read().expect("lock poisoned").clone()
    }

    // ── Write ────────────────────────────────────────────────────────────

    /// Read → mutate → persist-atomically → swap, all under one *held*
    /// write lock — not just the final swap. A concurrent writer (another
    /// mutator on this same domain, or `reload_all` re-reading it from disk
    /// on a different task) must block until this call has fully landed on
    /// both disk and memory, or the two writes can silently clobber each
    /// other. See mjolnir-permissions.md's Pitfall: "storage must express
    /// deny-wins, not last-write-wins" — a lost concurrent write is exactly
    /// that failure mode, just for grants generally rather than only
    /// allow/deny ordering. Every domain-mutating method in this file
    /// (`with_permissions_mut`, `with_mcp_mut`, `with_context_files_mut`,
    /// `set_provider`, `set_tui`) is this same shape parameterized by which
    /// `RwLock` and which on-disk path it targets — consolidated into one
    /// generic here so the shape can't drift between domains, and a future
    /// domain doesn't have to re-derive the locking argument above.
    fn with_domain_mut<T: Clone + serde::Serialize>(&self, lock: &RwLock<T>, path: &Path, f: impl FnOnce(&mut T)) -> Result<(), ConfigError> {
        let mut guard = lock.write().expect("lock poisoned");
        let mut next = guard.clone();
        f(&mut next);
        fsio::write_atomic(path, &next)?;
        *guard = next;
        Ok(())
    }

    fn permissions_lock(&self, scope: Scope) -> &RwLock<PermissionsConfig> {
        match scope {
            Scope::Project => &self.inner.project_permissions,
            Scope::Global  => &self.inner.global_permissions,
        }
    }

    fn with_permissions_mut(&self, scope: Scope, f: impl FnOnce(&mut PermissionsConfig)) -> Result<(), ConfigError> {
        self.with_domain_mut(self.permissions_lock(scope), &self.domain_path(scope, "permissions"), f)
    }

    pub fn add_grant(
        &self,
        scope: Scope,
        list:  GrantList,
        entry: impl Into<String>,
    ) -> Result<(), ConfigError> {
        let entry = entry.into();
        self.with_permissions_mut(scope, |cfg| {
            let target = match list { GrantList::Allow => &mut cfg.allow, GrantList::Deny => &mut cfg.deny };
            if !target.contains(&entry) {
                target.push(entry);
            }
        })
    }

    pub fn remove_grant(&self, scope: Scope, list: GrantList, entry: &str) -> Result<(), ConfigError> {
        self.with_permissions_mut(scope, |cfg| {
            let target = match list { GrantList::Allow => &mut cfg.allow, GrantList::Deny => &mut cfg.deny };
            target.retain(|e| e != entry);
        })
    }

    pub fn set_provider(&self, scope: Scope, provider: ProviderConfig) -> Result<(), ConfigError> {
        let path = self.domain_path(scope, "provider");
        if !provider.has_valid_api_key_env() {
            return Err(ConfigError::MissingApiKeyEnv { path });
        }
        let lock = match scope {
            Scope::Project => &self.inner.project_provider,
            Scope::Global  => &self.inner.global_provider,
        };
        self.with_domain_mut(lock, &path, move |current| *current = Some(provider))
    }

    fn mcp_lock(&self, scope: Scope) -> &RwLock<McpConfig> {
        match scope {
            Scope::Project => &self.inner.project_mcp,
            Scope::Global  => &self.inner.global_mcp,
        }
    }

    fn with_mcp_mut(&self, scope: Scope, f: impl FnOnce(&mut McpConfig)) -> Result<(), ConfigError> {
        self.with_domain_mut(self.mcp_lock(scope), &self.domain_path(scope, "mcp"), f)
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
        self.with_domain_mut(&self.inner.global_tui, &path, move |current| *current = tui)
    }

    fn with_context_files_mut(&self, f: impl FnOnce(&mut ContextFilesConfig)) -> Result<(), ConfigError> {
        self.with_domain_mut(&self.inner.project_context_files, &self.domain_path(Scope::Project, "context_files"), f)
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

        if failures.is_empty() { Ok(()) } else { Err(failures) }
    }

    fn reload_domain<T: Clone + serde::de::DeserializeOwned>(
        &self,
        lock: &RwLock<T>,
        path: PathBuf,
        version: u32,
        empty: fn() -> T,
        failures: &mut Vec<ReloadFailure>,
    ) {
        match fsio::read_versioned::<T>(&path, version) {
            Ok(Some(value)) => *lock.write().expect("lock poisoned") = value,
            Ok(None) => *lock.write().expect("lock poisoned") = empty(),
            Err(error) => failures.push(ReloadFailure { path, error }),
        }
    }

    fn reload_provider(&self, scope: Scope, failures: &mut Vec<ReloadFailure>) {
        let path = self.domain_path(scope, "provider");
        let lock = match scope {
            Scope::Project => &self.inner.project_provider,
            Scope::Global  => &self.inner.global_provider,
        };
        match load_provider(&path) {
            Ok(value) => *lock.write().expect("lock poisoned") = value,
            Err(error) => failures.push(ReloadFailure { path, error }),
        }
    }

    // ── First launch ─────────────────────────────────────────────────────

    /// Create `~/.mjolnir/` and write the four annotated global files if the
    /// directory does not exist. Idempotent — a directory that already
    /// exists is inspected for completeness rather than touched.
    pub fn init_global_if_empty(&self) -> Result<InitOutcome, ConfigError> {
        let dir = self.inner.global_dir.clone();

        if !dir.exists() {
            let permissions_path = dir.join("permissions.yaml");
            let provider_path    = dir.join("provider.yaml");
            let mcp_path         = dir.join("mcp.yaml");
            let tui_path         = dir.join("tui.yaml");

            fsio::write_atomic_text(&permissions_path, annotated::PERMISSIONS)?;
            fsio::write_atomic_text(&provider_path, annotated::PROVIDER)?;
            fsio::write_atomic_text(&mcp_path, annotated::MCP)?;
            fsio::write_atomic_text(&tui_path, annotated::TUI)?;

            *self.inner.global_permissions.write().expect("lock poisoned") =
                fsio::read_versioned(&permissions_path, PERMISSIONS_VERSION)?
                    .expect("just wrote a file matching this schema");
            *self.inner.global_provider.write().expect("lock poisoned") =
                Some(load_provider(&provider_path)?.expect("just wrote a file matching this schema"));
            *self.inner.global_mcp.write().expect("lock poisoned") =
                fsio::read_versioned(&mcp_path, MCP_VERSION)?
                    .expect("just wrote a file matching this schema");
            *self.inner.global_tui.write().expect("lock poisoned") =
                fsio::read_versioned(&tui_path, TUI_VERSION)?
                    .expect("just wrote a file matching this schema");

            return Ok(InitOutcome::Created);
        }

        let required = ["permissions.yaml", "provider.yaml", "mcp.yaml", "tui.yaml"];
        let missing: Vec<&'static str> =
            required.iter().copied().filter(|f| !dir.join(f).is_file()).collect();

        if missing.is_empty() {
            Ok(InitOutcome::AlreadyPresent)
        } else {
            Ok(InitOutcome::PartiallyPresent { missing })
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────
//
// Covers this crate's real failure modes per mjolnir-config.md's Pitfalls:
// deny-wins staying structural, version-bump rejection, partial-init refusing
// to start, reload retaining the previous snapshot on a bad file while still
// naming it, and project scope not materialising until first write.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::McpTransport;
    use tempfile::tempdir;

    /// Fresh (project_dir, global_dir) temp roots and the Config opened on
    /// them — neither exists on disk yet, matching a real fresh checkout.
    fn fresh() -> (tempfile::TempDir, tempfile::TempDir, Config) {
        let project = tempdir().unwrap();
        let global = tempdir().unwrap();
        // Use a not-yet-existing subdirectory so "does the dir exist" checks
        // (init_global_if_empty, project-scope materialisation) start from
        // true absence rather than an empty-but-present tempdir.
        let global_root = global.path().join(".mjolnir");
        let config = Config::open_at(project.path(), &global_root).unwrap();
        (project, global, config)
    }

    #[test]
    fn legacy_global_dir_is_migrated_when_the_new_one_does_not_exist() {
        let home = tempdir().unwrap();
        let legacy_dir = home.path().join(".amundsen");
        std::fs::create_dir(&legacy_dir).unwrap();
        std::fs::write(legacy_dir.join("provider.yaml"), "version: 1\n").unwrap();

        let new_dir = home.path().join(".mjolnir");
        migrate_legacy_global_dir(home.path(), &new_dir);

        assert!(!legacy_dir.exists(), "the old .amundsen/ should be moved, not copied");
        assert!(new_dir.join("provider.yaml").exists(), "the migrated file must survive the move");
    }

    #[test]
    fn migration_is_a_no_op_when_the_new_global_dir_already_exists() {
        let home = tempdir().unwrap();
        let legacy_dir = home.path().join(".amundsen");
        std::fs::create_dir(&legacy_dir).unwrap();
        std::fs::write(legacy_dir.join("provider.yaml"), "legacy").unwrap();

        let new_dir = home.path().join(".mjolnir");
        std::fs::create_dir(&new_dir).unwrap();
        std::fs::write(new_dir.join("provider.yaml"), "current").unwrap();

        migrate_legacy_global_dir(home.path(), &new_dir);

        assert!(legacy_dir.exists(), "an already-migrated (or independently created) new dir must not trigger another move");
        assert_eq!(std::fs::read_to_string(new_dir.join("provider.yaml")).unwrap(), "current", "the existing new-dir content must not be clobbered");
    }

    #[test]
    fn migration_is_a_no_op_when_no_legacy_dir_exists() {
        let home = tempdir().unwrap();
        let new_dir = home.path().join(".mjolnir");

        migrate_legacy_global_dir(home.path(), &new_dir);

        assert!(!new_dir.exists(), "nothing to migrate — a fresh install must not have .mjolnir/ conjured from nothing");
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
        assert!(matches!(config.global_provider(), Err(ConfigError::ProviderNotConfigured { .. })));
    }

    #[test]
    fn project_scope_directory_is_not_created_until_first_write() {
        let (project, _global, config) = fresh();
        let mjolnir_dir = project.path().join(".mjolnir");
        assert!(!mjolnir_dir.exists());

        config.add_grant(Scope::Project, GrantList::Allow, "read:**").unwrap();
        assert!(mjolnir_dir.is_dir());
        assert!(mjolnir_dir.join("permissions.yaml").is_file());
    }

    #[test]
    fn init_global_if_empty_is_created_then_already_present() {
        let (_project, global, config) = fresh();
        let global_dir = global.path().join(".mjolnir");
        assert!(!global_dir.exists());

        assert_eq!(config.init_global_if_empty().unwrap(), InitOutcome::Created);
        for f in ["permissions.yaml", "provider.yaml", "mcp.yaml", "tui.yaml"] {
            assert!(global_dir.join(f).is_file(), "missing {f}");
        }

        // In-memory snapshot reflects what was just written, not stale defaults.
        assert_eq!(config.global_permissions(), PermissionsConfig::empty());
        assert!(config.global_provider().unwrap().has_valid_api_key_env());

        // Idempotent: a second call sees everything already there.
        assert_eq!(config.init_global_if_empty().unwrap(), InitOutcome::AlreadyPresent);
    }

    #[test]
    fn init_global_if_empty_partial_directory_refuses_to_start() {
        let (_project, global, config) = fresh();
        let global_dir = global.path().join(".mjolnir");

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

        config.add_grant(Scope::Project, GrantList::Allow, "shell:git *").unwrap();
        config.add_grant(Scope::Project, GrantList::Deny, "shell:git *").unwrap();

        let cfg = config.project_permissions();
        assert_eq!(cfg.allow, vec!["shell:git *".to_string()]);
        assert_eq!(cfg.deny, vec!["shell:git *".to_string()]);
    }

    /// Regression test for the lost-update race the audit found: the
    /// `with_*_mut` helpers used to release the read lock before mutating,
    /// then reacquire a write lock only for the final swap, leaving a
    /// window where two concurrent writers could both compute `next` from
    /// the same stale snapshot and the second write would silently clobber
    /// the first. Holding the write lock across the entire
    /// read-mutate-persist-swap sequence (this test's real regression
    /// target) serializes concurrent writers instead, so every one of these
    /// threads' grants must survive.
    #[test]
    fn concurrent_grant_writes_do_not_lose_updates() {
        let (_project, _global, config) = fresh();
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let config = config.clone();
                std::thread::spawn(move || {
                    config.add_grant(Scope::Project, GrantList::Allow, format!("read:file{i}")).unwrap();
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }

        let allow = config.project_permissions().allow;
        assert_eq!(allow.len(), 8, "every concurrent grant must survive, got {allow:?}");
        for i in 0..8 {
            assert!(allow.contains(&format!("read:file{i}")), "missing grant read:file{i} in {allow:?}");
        }
    }

    #[test]
    fn add_grant_is_idempotent_within_a_list() {
        let (_project, _global, config) = fresh();
        config.add_grant(Scope::Global, GrantList::Allow, "read:**").unwrap();
        config.add_grant(Scope::Global, GrantList::Allow, "read:**").unwrap();
        assert_eq!(config.global_permissions().allow, vec!["read:**".to_string()]);
    }

    #[test]
    fn remove_grant_only_touches_its_own_list() {
        let (_project, _global, config) = fresh();
        config.add_grant(Scope::Global, GrantList::Allow, "read:**").unwrap();
        config.add_grant(Scope::Global, GrantList::Deny, "read:**").unwrap();

        config.remove_grant(Scope::Global, GrantList::Allow, "read:**").unwrap();

        let cfg = config.global_permissions();
        assert!(cfg.allow.is_empty());
        assert_eq!(cfg.deny, vec!["read:**".to_string()]);
    }

    #[test]
    fn unknown_major_version_is_rejected() {
        let (project, _global, _config) = fresh();
        let dir = project.path().join(".mjolnir");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("permissions.yaml"), "version: 99\nallow: []\ndeny: []\n").unwrap();

        let err = Config::open_at(project.path(), _global.path().join(".mjolnir")).unwrap_err();
        assert!(matches!(err, ConfigError::UnknownVersion { found: 99, expected: 1, .. }));
    }

    #[test]
    fn empty_api_key_env_refuses_to_start() {
        let (_project, global, _config) = fresh();
        let dir = global.path().join(".mjolnir");
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
        let dir = global.path().join(".mjolnir");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("provider.yaml"), "version: 1\nprovider: anthropic\nmodel: m\napi_key_env: X\n").unwrap();

        let config = Config::open_at(_project.path(), &dir).unwrap();
        assert_eq!(config.global_provider().unwrap().extended_thinking_budget, None);
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
        assert_eq!(config.global_provider().unwrap().extended_thinking_budget, Some(16_000));
    }

    #[test]
    fn raw_api_key_field_is_rejected_by_the_schema() {
        let (_project, global, _config) = fresh();
        let dir = global.path().join(".mjolnir");
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
        assert!(!_global.path().join(".mjolnir").join("provider.yaml").exists());
        assert!(config.global_provider().is_err());
    }

    #[test]
    fn mcp_server_unknown_field_is_rejected_despite_flattened_transport() {
        // McpServer flattens McpTransport into itself; flatten + deny_unknown_fields
        // is a known serde trouble spot, so this checks the combination actually
        // still rejects a bogus field rather than silently accepting it.
        let bad = "name: fs\nkind: stdio\ncommand: fs-server\nbogus: 1\n";
        let result: Result<McpServer, _> = serde_yaml_ng::from_str(bad);
        assert!(result.is_err(), "expected bogus field to be rejected, got {result:?}");
    }

    #[test]
    fn add_mcp_server_upserts_by_name_replacing_wholesale() {
        let (_project, _global, config) = fresh();
        config
            .add_mcp_server(
                Scope::Global,
                McpServer {
                    name: "fs".into(),
                    transport: McpTransport::Stdio { command: "fs-server".into(), args: vec![] },
                    env: Default::default(),
                },
            )
            .unwrap();
        config
            .add_mcp_server(
                Scope::Global,
                McpServer {
                    name: "fs".into(),
                    transport: McpTransport::Http { url: "http://localhost:9/".into() },
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
        config.add_grant(Scope::Project, GrantList::Allow, "read:**").unwrap();
        config.add_grant(Scope::Global, GrantList::Allow, "shell:*").unwrap();

        // Hand-edit project permissions.yaml into garbage, but leave global alone.
        let dir = project.path().join(".mjolnir");
        std::fs::write(dir.join("permissions.yaml"), "not: [valid, yaml: at all").unwrap();

        let result = config.reload_all();
        let failures = result.unwrap_err();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].path, dir.join("permissions.yaml"));

        // Previous snapshot retained for the broken layer...
        assert_eq!(config.project_permissions().allow, vec!["read:**".to_string()]);
        // ...while an unrelated, still-valid layer still reloads fine.
        assert_eq!(config.global_permissions().allow, vec!["shell:*".to_string()]);
    }

    #[test]
    fn reload_all_picks_up_hand_edits_that_are_still_valid() {
        let (project, _global, config) = fresh();
        let dir = project.path().join(".mjolnir");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("permissions.yaml"), "version: 1\nallow: [\"edited:in\"]\ndeny: []\n")
            .unwrap();

        config.reload_all().unwrap();
        assert_eq!(config.project_permissions().allow, vec!["edited:in".to_string()]);
    }

    #[test]
    fn clones_share_the_same_in_memory_state() {
        let (_project, _global, config) = fresh();
        let other = config.clone();
        config.add_grant(Scope::Global, GrantList::Allow, "read:**").unwrap();
        assert_eq!(other.global_permissions().allow, vec!["read:**".to_string()]);
    }
}
