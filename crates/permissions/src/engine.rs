use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use mjolnir_config::{Config, GrantList, Scope as ConfigScope};

use crate::error::PermissionError;
use crate::grant::{Decision, GrantKey};
use crate::prompt::{ContextFileTier, PromptPayload, ToolTier};

/// Where a grant or context-file approval came from, for display in
/// [`EffectiveView`]. Distinct from `mjolnir_config::Scope` because session
/// has no config-backed counterpart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantScope {
    Session,
    Project,
    Global,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveGrant {
    pub scope:    GrantScope,
    pub kind:     String,
    pub pattern:  String,
    pub decision: Decision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveContextFile {
    pub scope: GrantScope, // Project or Session only
    pub path:  PathBuf,
}

/// Immutable snapshot of the merged view across all three scopes, with
/// per-grant scope attribution, for the TUI's permissions panel.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EffectiveView {
    pub tool_grants:   Vec<EffectiveGrant>,
    pub context_files: Vec<EffectiveContextFile>,
}

/// Result of [`Engine::check_tool`] / [`Engine::check_context_file`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckOutcome {
    Allow,
    Deny,
    PromptRequired(PromptPayload),
}

impl From<Decision> for CheckOutcome {
    fn from(decision: Decision) -> Self {
        match decision {
            Decision::Allow => CheckOutcome::Allow,
            Decision::Deny => CheckOutcome::Deny,
        }
    }
}

#[derive(Default)]
struct SessionState {
    tool_grants:   Vec<(GrantKey, Decision)>,
    context_files: HashSet<PathBuf>,
}

/// Default-deny permission engine — see mjolnir-permissions.md. Owns scope
/// precedence, pattern matching, and the in-memory shape of allow/deny lists.
/// Session-scope decisions live only in this struct; project/global persist
/// through `mjolnir_config::Config`, which is cheap to clone (internally
/// `Arc`) so `Engine` just holds one.
pub struct Engine {
    config:  Config,
    session: RwLock<SessionState>,
}

impl Engine {
    pub fn new(config: Config) -> Self {
        Self { config, session: RwLock::new(SessionState::default()) }
    }

    // ── Tool checks ──────────────────────────────────────────────────────

    /// Checks a tool invocation. `target` is the string grant patterns are
    /// matched against — the assembled argv for shell-shaped tools, a file
    /// path for path-shaped ones. `edit_class` tools are never allowlistable:
    /// this always returns `PromptRequired(PromptPayload::Edit)` regardless
    /// of any persisted grant, without consulting allow/deny lists at all.
    pub fn check_tool(&self, kind: &str, target: &str, edit_class: bool) -> CheckOutcome {
        if edit_class {
            return CheckOutcome::PromptRequired(PromptPayload::Edit { kind: kind.to_string() });
        }

        {
            let session = self.session.read().expect("session lock poisoned");
            if let Some(decision) = session_scope_decision(&session.tool_grants, kind, target) {
                return decision.into();
            }
        }

        let project = self.config.project_permissions();
        if let Some(decision) = list_scope_decision(&project.allow, &project.deny, kind, target) {
            return decision.into();
        }

        let global = self.config.global_permissions();
        if let Some(decision) = list_scope_decision(&global.allow, &global.deny, kind, target) {
            return decision.into();
        }

        CheckOutcome::PromptRequired(PromptPayload::Tool { kind: kind.to_string(), target: target.to_string() })
    }

    /// Records the developer's answer to a tool four-tier prompt.
    /// `pattern` is what gets persisted (usually, but not necessarily, an
    /// exact-match glob of `target` from the matching `check_tool` call —
    /// the caller decides how coarse the grant should be). Refuses
    /// `edit_class` kinds outright: Edit is never allowlistable at any tier,
    /// including `Once`.
    pub fn record_tool_decision(
        &self,
        kind:       &str,
        pattern:    &str,
        edit_class: bool,
        decision:   Decision,
        tier:       ToolTier,
    ) -> Result<(), PermissionError> {
        if edit_class {
            return Err(PermissionError::EditNotAllowlistable);
        }

        match tier {
            ToolTier::Once => {}
            ToolTier::Session => {
                let key = GrantKey::new(kind, pattern);
                let mut session = self.session.write().expect("session lock poisoned");
                session.tool_grants.retain(|(g, _)| g != &key);
                session.tool_grants.push((key, decision));
            }
            ToolTier::Project => self.persist_grant(ConfigScope::Project, kind, pattern, decision)?,
            ToolTier::Always => self.persist_grant(ConfigScope::Global, kind, pattern, decision)?,
        }
        Ok(())
    }

    fn persist_grant(
        &self,
        scope:    ConfigScope,
        kind:     &str,
        pattern:  &str,
        decision: Decision,
    ) -> Result<(), PermissionError> {
        let entry = GrantKey::new(kind, pattern).to_string();
        let list = match decision {
            Decision::Allow => GrantList::Allow,
            Decision::Deny => GrantList::Deny,
        };
        self.config.add_grant(scope, list, entry)?;
        Ok(())
    }

    // ── Context-file checks ──────────────────────────────────────────────

    /// Checks a CLAUDE.md / AGENTS.md candidate. Path-keyed only, no content
    /// hash — see mjolnir-permissions.md's Decisions. There is no deny
    /// list: decline simply persists nothing, so absence re-prompts.
    pub fn check_context_file(&self, path: &Path) -> CheckOutcome {
        if self.config.project_context_files().approved.iter().any(|p| p == path) {
            return CheckOutcome::Allow;
        }

        let session = self.session.read().expect("session lock poisoned");
        if session.context_files.contains(path) {
            return CheckOutcome::Allow;
        }
        drop(session);

        CheckOutcome::PromptRequired(PromptPayload::ContextFile { path: path.to_path_buf() })
    }

    /// Records the developer's answer to a context-file two-tier prompt.
    /// `approve: false` (decline) persists nothing at any tier.
    pub fn record_context_file_decision(
        &self,
        path:    &Path,
        approve: bool,
        tier:    Option<ContextFileTier>,
    ) -> Result<(), PermissionError> {
        if !approve {
            return Ok(());
        }
        match tier {
            Some(ContextFileTier::Project) => {
                self.config.add_context_file(path.to_path_buf())?;
            }
            Some(ContextFileTier::Session) | None => {
                self.session.write().expect("session lock poisoned").context_files.insert(path.to_path_buf());
            }
        }
        Ok(())
    }

    // ── Effective view ───────────────────────────────────────────────────

    pub fn effective_view(&self) -> EffectiveView {
        let mut tool_grants = Vec::new();
        {
            let session = self.session.read().expect("session lock poisoned");
            tool_grants.extend(session.tool_grants.iter().map(|(g, d)| EffectiveGrant {
                scope:    GrantScope::Session,
                kind:     g.kind.clone(),
                pattern:  g.pattern.clone(),
                decision: *d,
            }));
        }

        let project = self.config.project_permissions();
        tool_grants.extend(collect_grants(GrantScope::Project, &project.allow, Decision::Allow));
        tool_grants.extend(collect_grants(GrantScope::Project, &project.deny, Decision::Deny));

        let global = self.config.global_permissions();
        tool_grants.extend(collect_grants(GrantScope::Global, &global.allow, Decision::Allow));
        tool_grants.extend(collect_grants(GrantScope::Global, &global.deny, Decision::Deny));

        let mut context_files: Vec<EffectiveContextFile> = self
            .config
            .project_context_files()
            .approved
            .into_iter()
            .map(|path| EffectiveContextFile { scope: GrantScope::Project, path })
            .collect();
        {
            let session = self.session.read().expect("session lock poisoned");
            context_files.extend(
                session
                    .context_files
                    .iter()
                    .cloned()
                    .map(|path| EffectiveContextFile { scope: GrantScope::Session, path }),
            );
        }

        EffectiveView { tool_grants, context_files }
    }
}

fn session_scope_decision(grants: &[(GrantKey, Decision)], kind: &str, target: &str) -> Option<Decision> {
    let mut allow_matched = false;
    let mut deny_matched = false;
    for (grant, decision) in grants {
        if grant.matches(kind, target) {
            match decision {
                Decision::Deny => deny_matched = true,
                Decision::Allow => allow_matched = true,
            }
        }
    }
    resolve(allow_matched, deny_matched)
}

fn list_scope_decision(allow: &[String], deny: &[String], kind: &str, target: &str) -> Option<Decision> {
    resolve(list_matches(allow, kind, target), list_matches(deny, kind, target))
}

fn list_matches(list: &[String], kind: &str, target: &str) -> bool {
    list.iter().any(|entry| GrantKey::parse(entry).map(|g| g.matches(kind, target)).unwrap_or(false))
}

/// Within a scope, deny beats allow.
fn resolve(allow_matched: bool, deny_matched: bool) -> Option<Decision> {
    if deny_matched {
        Some(Decision::Deny)
    } else if allow_matched {
        Some(Decision::Allow)
    } else {
        None
    }
}

fn collect_grants(scope: GrantScope, list: &[String], decision: Decision) -> Vec<EffectiveGrant> {
    list.iter()
        .filter_map(|entry| GrantKey::parse(entry).ok())
        .map(|g| EffectiveGrant { scope, kind: g.kind, pattern: g.pattern, decision })
        .collect()
}

// ── Tests ────────────────────────────────────────────────────────────────
//
// Covers this crate's real failure modes per mjolnir-permissions.md's
// Pitfalls: default-deny as the floor, session overriding both directions,
// deny-wins within a scope, the edit_class flag (not tool name) driving
// enforcement, session persistence never leaking to project storage, and
// context-file decline never being recorded.

#[cfg(test)]
mod tests {
    use super::*;
    use mjolnir_config::Config;
    use tempfile::tempdir;

    fn fresh_engine() -> (tempfile::TempDir, tempfile::TempDir, Engine) {
        let project = tempdir().unwrap();
        let global = tempdir().unwrap();
        let config = Config::open_at(project.path(), global.path().join(".mjolnir")).unwrap();
        (project, global, Engine::new(config))
    }

    #[test]
    fn unconfigured_tool_prompts_by_default() {
        let (_p, _g, engine) = fresh_engine();
        assert_eq!(
            engine.check_tool("shell", "cargo test", false),
            CheckOutcome::PromptRequired(PromptPayload::Tool {
                kind:   "shell".into(),
                target: "cargo test".into(),
            })
        );
    }

    #[test]
    fn project_allow_grant_is_honoured() {
        let (_p, _g, engine) = fresh_engine();
        engine.record_tool_decision("shell", "cargo test*", false, Decision::Allow, ToolTier::Project).unwrap();
        assert_eq!(engine.check_tool("shell", "cargo test -- foo", false), CheckOutcome::Allow);
        assert_eq!(engine.check_tool("shell", "cargo install foo", false), CheckOutcome::PromptRequired(PromptPayload::Tool {
            kind: "shell".into(), target: "cargo install foo".into(),
        }));
    }

    #[test]
    fn once_tier_never_persists() {
        let (_p, _g, engine) = fresh_engine();
        engine.record_tool_decision("shell", "cargo test*", false, Decision::Allow, ToolTier::Once).unwrap();
        assert_eq!(
            engine.check_tool("shell", "cargo test", false),
            CheckOutcome::PromptRequired(PromptPayload::Tool { kind: "shell".into(), target: "cargo test".into() })
        );
    }

    #[test]
    fn session_allow_overrides_global_deny() {
        let (_p, _g, engine) = fresh_engine();
        engine.record_tool_decision("shell", "cargo test*", false, Decision::Deny, ToolTier::Always).unwrap();
        assert_eq!(engine.check_tool("shell", "cargo test", false), CheckOutcome::Deny);

        engine.record_tool_decision("shell", "cargo test*", false, Decision::Allow, ToolTier::Session).unwrap();
        assert_eq!(engine.check_tool("shell", "cargo test", false), CheckOutcome::Allow);
    }

    #[test]
    fn session_deny_overrides_global_allow() {
        let (_p, _g, engine) = fresh_engine();
        engine.record_tool_decision("shell", "cargo test*", false, Decision::Allow, ToolTier::Always).unwrap();
        engine.record_tool_decision("shell", "cargo test*", false, Decision::Deny, ToolTier::Session).unwrap();
        assert_eq!(engine.check_tool("shell", "cargo test", false), CheckOutcome::Deny);
    }

    #[test]
    fn deny_beats_allow_within_the_same_scope() {
        let (_p, _g, engine) = fresh_engine();
        engine.record_tool_decision("shell", "cargo *", false, Decision::Allow, ToolTier::Project).unwrap();
        engine.record_tool_decision("shell", "cargo install*", false, Decision::Deny, ToolTier::Project).unwrap();
        assert_eq!(engine.check_tool("shell", "cargo install foo", false), CheckOutcome::Deny);
        assert_eq!(engine.check_tool("shell", "cargo test", false), CheckOutcome::Allow);
    }

    #[test]
    fn project_scope_wins_over_global_scope() {
        let (_p, _g, engine) = fresh_engine();
        engine.record_tool_decision("shell", "cargo *", false, Decision::Allow, ToolTier::Always).unwrap();
        engine.record_tool_decision("shell", "cargo *", false, Decision::Deny, ToolTier::Project).unwrap();
        assert_eq!(engine.check_tool("shell", "cargo test", false), CheckOutcome::Deny);
    }

    #[test]
    fn edit_class_always_prompts_regardless_of_grants() {
        let (_p, _g, engine) = fresh_engine();
        engine.record_tool_decision("edit", "**", false, Decision::Allow, ToolTier::Always).unwrap();
        // Same kind, but this call is made as edit_class — must still prompt,
        // proving enforcement is keyed to the flag, not the tool name.
        assert_eq!(
            engine.check_tool("edit", "./src/main.rs", true),
            CheckOutcome::PromptRequired(PromptPayload::Edit { kind: "edit".into() })
        );
    }

    #[test]
    fn edit_class_decisions_cannot_be_persisted_at_any_tier() {
        let (_p, _g, engine) = fresh_engine();
        for tier in [ToolTier::Once, ToolTier::Session, ToolTier::Project, ToolTier::Always] {
            let result = engine.record_tool_decision("edit", "./src/main.rs", true, Decision::Allow, tier);
            assert!(matches!(result, Err(PermissionError::EditNotAllowlistable)));
        }
    }

    #[test]
    fn session_grant_does_not_leak_into_project_storage() {
        let (project, _g, engine) = fresh_engine();
        engine.record_tool_decision("shell", "cargo test*", false, Decision::Allow, ToolTier::Session).unwrap();
        let path = project.path().join(".mjolnir").join("permissions.yaml");
        assert!(!path.exists(), "session-tier grant must not touch disk");
    }

    #[test]
    fn context_file_prompts_until_approved() {
        let (_p, _g, engine) = fresh_engine();
        let path = PathBuf::from("./CLAUDE.md");
        assert_eq!(engine.check_context_file(&path), CheckOutcome::PromptRequired(PromptPayload::ContextFile { path: path.clone() }));

        engine.record_context_file_decision(&path, true, Some(ContextFileTier::Session)).unwrap();
        assert_eq!(engine.check_context_file(&path), CheckOutcome::Allow);
    }

    #[test]
    fn context_file_project_tier_persists_across_engines() {
        let (project, global, engine) = fresh_engine();
        let path = PathBuf::from("./CLAUDE.md");
        engine.record_context_file_decision(&path, true, Some(ContextFileTier::Project)).unwrap();

        let reopened = Config::open_at(project.path(), global.path().join(".mjolnir")).unwrap();
        let other = Engine::new(reopened);
        assert_eq!(other.check_context_file(&path), CheckOutcome::Allow);
    }

    #[test]
    fn context_file_decline_persists_nothing() {
        let (_p, _g, engine) = fresh_engine();
        let path = PathBuf::from("./CLAUDE.md");
        engine.record_context_file_decision(&path, false, None).unwrap();
        assert_eq!(engine.check_context_file(&path), CheckOutcome::PromptRequired(PromptPayload::ContextFile { path }));
    }

    #[test]
    fn effective_view_attributes_grants_to_their_scope() {
        let (_p, _g, engine) = fresh_engine();
        engine.record_tool_decision("shell", "a*", false, Decision::Allow, ToolTier::Session).unwrap();
        engine.record_tool_decision("shell", "b*", false, Decision::Allow, ToolTier::Project).unwrap();
        engine.record_tool_decision("shell", "c*", false, Decision::Deny, ToolTier::Always).unwrap();

        let view = engine.effective_view();
        let scope_of = |pattern: &str| {
            view.tool_grants.iter().find(|g| g.pattern == pattern).map(|g| g.scope)
        };
        assert_eq!(scope_of("a*"), Some(GrantScope::Session));
        assert_eq!(scope_of("b*"), Some(GrantScope::Project));
        assert_eq!(scope_of("c*"), Some(GrantScope::Global));
    }
}
