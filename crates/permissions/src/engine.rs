use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use mjolnir_config::{Class, Config, GrantEntry, GrantList, Rung, Scope as ConfigScope};

use crate::error::PermissionError;
use crate::prompt::{Choice, ContextFileTier, PromptPayload};

/// Where a rule came from. Distinct from `mjolnir_config::Scope` because
/// session has no config-backed counterpart, and `turn` is not a scope at all
/// — a turn answer writes nothing anywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantScope {
    Session,
    Project,
    Global,
}

impl GrantScope {
    /// How a prompt names the place a rule lives, for a developer who has to
    /// go and change it.
    pub fn where_it_lives(self) -> &'static str {
        match self {
            GrantScope::Session => "this session",
            GrantScope::Project => "<project>/.mjolnir/permissions.yaml",
            GrantScope::Global => "~/.mjolnir/permissions.yaml",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveGrant {
    pub scope: GrantScope,
    pub list:  GrantList,
    pub entry: GrantEntry,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveContextFile {
    pub scope: GrantScope, // Project or Session only
    pub path:  PathBuf,
}

/// Immutable snapshot of the merged view across scopes, for the TUI's
/// permissions panel.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EffectiveView {
    pub rung:          Option<Rung>,
    pub grants:        Vec<EffectiveGrant>,
    pub context_files: Vec<EffectiveContextFile>,
}

/// What [`Engine::check`] concluded.
///
/// [`Self::Locked`] and "ask" are deliberately different answers.  A locked
/// call is refused and stays refused — no prompt is drawn, because there is no
/// answer the developer could give here that would change it (ADR 0004 §7).
/// The scope and rule are carried so the refusal can say where to go and undo
/// it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Allow,
    Locked { scope: GrantScope, rule: GrantEntry },
    Ask(PromptPayload),
}

#[derive(Default)]
struct SessionState {
    allow:         Vec<GrantEntry>,
    deny:          Vec<GrantEntry>,
    context_files: HashSet<PathBuf>,
}

/// The permission engine of ADR 0004. Owns precedence, the standing rung, and
/// the in-memory session layer; project and global layers persist through
/// `mjolnir_config::Config`, which is cheap to clone (internally `Arc`).
///
/// The order it resolves in, and why it is this order:
///
/// 1. **`edit` never resolves here at all.** It is outside the model, so it
///    always asks, and [`Engine::record`] refuses to persist anything for it.
/// 2. **Deny, across every scope.** A deny is a lock, so it is checked before
///    anything that could allow — including a narrower scope, which is exactly
///    what distinguishes a lock from a pre-answer.
/// 3. **Allow, across every scope.** Any allow that covers the call suffices;
///    allows do not compete with each other.
/// 4. **The standing rung**, narrower file winning outright.
/// 5. Otherwise, ask.
///
/// Steps 2 and 3 run before step 4, which is what "an explicit entry outranks
/// the default" means: a denied program stays denied under `write`, and an
/// allowed one runs under `ask`.
pub struct Engine {
    config:  Config,
    session: RwLock<SessionState>,
}

impl Engine {
    pub fn new(config: Config) -> Self {
        Self { config, session: RwLock::new(SessionState::default()) }
    }

    // ── Tool checks ──────────────────────────────────────────────────────

    /// Resolves one call: a program, and the class the agent declared for it.
    ///
    /// The declaration is an input here, not a trusted fact. Nothing in this
    /// function verifies it — that is the sandbox's job at execution time
    /// (ADR 0004 §4), and the split is deliberate: a policy engine that also
    /// tried to judge what a command does would be making exactly the guess
    /// the sandbox exists to avoid.
    pub fn check(&self, program: &str, declared: Class, argv: &[String]) -> Outcome {
        let ask = || {
            Outcome::Ask(PromptPayload::Tool {
                program: program.to_string(),
                argv:    argv.to_vec(),
                declared,
            })
        };

        if declared == Class::Edit {
            return ask();
        }

        if let Some((scope, rule)) = self.first_deny(program, declared) {
            return Outcome::Locked { scope, rule };
        }
        if self.any_allow(program, declared) {
            return Outcome::Allow;
        }
        if self.effective_rung().is_some_and(|r| r.covers(declared)) {
            return Outcome::Allow;
        }
        ask()
    }

    /// The first deny covering this call, searched narrowest scope outwards.
    /// Which one is found does not change the answer — every deny is a lock —
    /// only which file the refusal points at, and the nearest one is the one a
    /// developer is most likely to be able to change.
    fn first_deny(&self, program: &str, class: Class) -> Option<(GrantScope, GrantEntry)> {
        let session = self.session.read().expect("session lock poisoned");
        if let Some(e) = session.deny.iter().find(|e| e.denies(program, class)) {
            return Some((GrantScope::Session, e.clone()));
        }
        drop(session);

        let project = self.config.project_permissions();
        if let Some(e) = project.deny.iter().find(|e| e.denies(program, class)) {
            return Some((GrantScope::Project, e.clone()));
        }

        let global = self.config.global_permissions();
        global
            .deny
            .iter()
            .find(|e| e.denies(program, class))
            .map(|e| (GrantScope::Global, e.clone()))
    }

    fn any_allow(&self, program: &str, class: Class) -> bool {
        let session = self.session.read().expect("session lock poisoned");
        if session.allow.iter().any(|e| e.allows(program, class)) {
            return true;
        }
        drop(session);

        self.config.project_permissions().allow.iter().any(|e| e.allows(program, class))
            || self.config.global_permissions().allow.iter().any(|e| e.allows(program, class))
    }

    /// The standing rung in force here: the project file's if it states one,
    /// otherwise the global file's (ADR 0004 §6, narrower wins outright).
    ///
    /// `None` — neither file states one — is not the same as `Ask`, but it
    /// behaves identically at a check. It is kept distinct so the permissions
    /// panel can show "not set" rather than asserting a rung the developer
    /// never chose.
    pub fn effective_rung(&self) -> Option<Rung> {
        self.config.project_permissions().default.or(self.config.global_permissions().default)
    }

    /// Records the developer's answer to the eight-row prompt.
    ///
    /// `class` is the class of the call that raised the prompt, which is what
    /// rows 2–4 and 6–7 qualify themselves by. Refuses `edit` outright at
    /// every row, including the two that persist nothing: `edit` is outside
    /// the model, and a turn-scoped `edit` grant would be the first step back
    /// towards making it grantable.
    pub fn record(&self, program: &str, class: Class, choice: Choice) -> Result<(), PermissionError> {
        if class == Class::Edit {
            return Err(PermissionError::EditNotGrantable);
        }

        let list = if choice.is_allow() { GrantList::Allow } else { GrantList::Deny };
        let entry = GrantEntry::classed(program, class);

        match choice {
            Choice::AllowOnce | Choice::DenyOnce => Ok(()),
            Choice::AllowSession | Choice::DenySession => {
                let mut session = self.session.write().expect("session lock poisoned");
                let target =
                    if list == GrantList::Allow { &mut session.allow } else { &mut session.deny };
                target.retain(|e| e.program != entry.program);
                target.push(entry);
                Ok(())
            }
            Choice::AllowProject | Choice::DenyProject => {
                self.config.add_grant(ConfigScope::Project, list, entry)?;
                Ok(())
            }
            Choice::AllowEverywhere => {
                self.config.add_grant(ConfigScope::Global, GrantList::Allow, entry)?;
                Ok(())
            }
            // Row 8 is the blunt one: the whole program, every class.
            Choice::NeverAllow => {
                self.config.add_grant(
                    ConfigScope::Global,
                    GrantList::Deny,
                    GrantEntry::program(program),
                )?;
                Ok(())
            }
        }
    }

    /// Sets a scope's standing rung. Not reachable from a prompt — a per-call
    /// moment is the wrong place to change the standing answer for everything
    /// (ADR 0004 §6) — so this is for first run, the permissions panel, and a
    /// hand edit.
    pub fn set_rung(&self, scope: ConfigScope, rung: Rung) -> Result<(), PermissionError> {
        self.config.set_default_rung(scope, rung)?;
        Ok(())
    }

    // ── Context-file checks ──────────────────────────────────────────────

    /// Checks a CLAUDE.md / AGENTS.md candidate. Path-keyed only, no content
    /// hash. There is no deny list: declining persists nothing, so absence
    /// re-prompts.
    pub fn check_context_file(&self, path: &Path) -> Outcome {
        if self.config.project_context_files().approved.iter().any(|p| p == path) {
            return Outcome::Allow;
        }
        if self.session.read().expect("session lock poisoned").context_files.contains(path) {
            return Outcome::Allow;
        }
        Outcome::Ask(PromptPayload::ContextFile { path: path.to_path_buf() })
    }

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
                self.session
                    .write()
                    .expect("session lock poisoned")
                    .context_files
                    .insert(path.to_path_buf());
            }
        }
        Ok(())
    }

    // ── Effective view ───────────────────────────────────────────────────

    pub fn effective_view(&self) -> EffectiveView {
        let mut grants = Vec::new();
        let mut context_files = Vec::new();

        {
            let session = self.session.read().expect("session lock poisoned");
            for (list, entries) in
                [(GrantList::Allow, &session.allow), (GrantList::Deny, &session.deny)]
            {
                grants.extend(entries.iter().map(|e| EffectiveGrant {
                    scope: GrantScope::Session,
                    list,
                    entry: e.clone(),
                }));
            }
            context_files.extend(session.context_files.iter().map(|p| EffectiveContextFile {
                scope: GrantScope::Session,
                path:  p.clone(),
            }));
        }

        for (scope, cfg) in [
            (GrantScope::Project, self.config.project_permissions()),
            (GrantScope::Global, self.config.global_permissions()),
        ] {
            for (list, entries) in [(GrantList::Allow, &cfg.allow), (GrantList::Deny, &cfg.deny)] {
                grants.extend(
                    entries.iter().map(|e| EffectiveGrant { scope, list, entry: e.clone() }),
                );
            }
        }

        context_files.extend(self.config.project_context_files().approved.iter().map(|p| {
            EffectiveContextFile { scope: GrantScope::Project, path: p.clone() }
        }));

        EffectiveView { rung: self.effective_rung(), grants, context_files }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> (tempfile::TempDir, tempfile::TempDir, Engine) {
        let project = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let config = Config::open_at(project.path(), global.path().join(".mjolnir")).unwrap();
        (project, global, Engine::new(config))
    }

    fn check(e: &Engine, program: &str, class: Class) -> Outcome {
        e.check(program, class, &[])
    }

    fn asks(outcome: &Outcome) -> bool {
        matches!(outcome, Outcome::Ask(_))
    }

    // ── The floor ────────────────────────────────────────────────────────

    #[test]
    fn nothing_runs_in_a_fresh_project() {
        let (_p, _g, e) = engine();
        assert!(asks(&check(&e, "git", Class::Read)));
        assert!(asks(&check(&e, "rm", Class::Write)));
    }

    #[test]
    fn edit_asks_under_every_rung_and_every_grant() {
        let (_p, _g, e) = engine();
        e.set_rung(ConfigScope::Global, Rung::Write).unwrap();
        e.record("edit", Class::Write, Choice::AllowEverywhere).unwrap();

        assert!(asks(&check(&e, "edit", Class::Edit)));
    }

    #[test]
    fn edit_cannot_be_recorded_at_any_row_including_the_ones_that_persist_nothing() {
        let (_p, _g, e) = engine();
        for choice in Choice::ORDER {
            assert!(
                matches!(e.record("edit", Class::Edit, choice), Err(PermissionError::EditNotGrantable)),
                "{choice:?} must refuse an edit-class call"
            );
        }
    }

    // ── The rung ─────────────────────────────────────────────────────────

    #[test]
    fn the_read_rung_runs_reads_and_asks_for_writes() {
        let (_p, _g, e) = engine();
        e.set_rung(ConfigScope::Global, Rung::Read).unwrap();

        assert_eq!(check(&e, "rg", Class::Read), Outcome::Allow);
        assert!(asks(&check(&e, "rg", Class::Write)));
    }

    #[test]
    fn the_write_rung_runs_both() {
        let (_p, _g, e) = engine();
        e.set_rung(ConfigScope::Global, Rung::Write).unwrap();

        assert_eq!(check(&e, "cargo", Class::Read), Outcome::Allow);
        assert_eq!(check(&e, "cargo", Class::Write), Outcome::Allow);
    }

    /// ADR 0004 §6: the narrower file wins outright, in both directions. A
    /// project may be locked down without touching the global file...
    #[test]
    fn a_project_rung_narrows_a_wider_global_one() {
        let (_p, _g, e) = engine();
        e.set_rung(ConfigScope::Global, Rung::Write).unwrap();
        e.set_rung(ConfigScope::Project, Rung::Read).unwrap();

        assert_eq!(check(&e, "cargo", Class::Read), Outcome::Allow);
        assert!(asks(&check(&e, "cargo", Class::Write)), "the project's narrower rung must win");
    }

    /// ...and opened up without loosening every project, which is the case
    /// that most-restrictive-wins would have made unreachable.
    #[test]
    fn a_project_rung_widens_a_narrower_global_one() {
        let (_p, _g, e) = engine();
        e.set_rung(ConfigScope::Global, Rung::Read).unwrap();
        e.set_rung(ConfigScope::Project, Rung::Write).unwrap();

        assert_eq!(check(&e, "cargo", Class::Write), Outcome::Allow);
    }

    /// A project that has never answered the question must fall through, not
    /// override the global file with a rung serde invented for it.
    #[test]
    fn a_project_that_states_no_rung_falls_through_to_global() {
        let (_p, _g, e) = engine();
        e.set_rung(ConfigScope::Global, Rung::Read).unwrap();

        assert_eq!(e.effective_rung(), Some(Rung::Read));
        assert_eq!(check(&e, "rg", Class::Read), Outcome::Allow);
    }

    // ── Entries outrank the rung, in both directions ─────────────────────

    #[test]
    fn an_allow_entry_runs_under_the_ask_rung() {
        let (_p, _g, e) = engine();
        e.record("git", Class::Read, Choice::AllowProject).unwrap();

        assert_eq!(e.effective_rung(), None);
        assert_eq!(check(&e, "git", Class::Read), Outcome::Allow);
        assert!(asks(&check(&e, "git", Class::Write)), "the grant was for reads only");
    }

    #[test]
    fn a_deny_entry_still_blocks_under_the_write_rung() {
        let (_p, _g, e) = engine();
        e.set_rung(ConfigScope::Global, Rung::Write).unwrap();
        e.record("curl", Class::Write, Choice::NeverAllow).unwrap();

        assert!(matches!(check(&e, "curl", Class::Write), Outcome::Locked { .. }));
    }

    // ── Class arithmetic ─────────────────────────────────────────────────

    #[test]
    fn a_write_grant_covers_reads_but_a_read_grant_does_not_cover_writes() {
        let (_p, _g, e) = engine();
        e.record("cargo", Class::Write, Choice::AllowProject).unwrap();
        e.record("git", Class::Read, Choice::AllowProject).unwrap();

        assert_eq!(check(&e, "cargo", Class::Read), Outcome::Allow);
        assert_eq!(check(&e, "cargo", Class::Write), Outcome::Allow);
        assert_eq!(check(&e, "git", Class::Read), Outcome::Allow);
        assert!(asks(&check(&e, "git", Class::Write)));
    }

    /// Deny is asymmetric with allow on purpose: forbidding a program's reads
    /// forbids its writes too, because permitting writing while forbidding
    /// reading describes no coherent posture.
    #[test]
    fn denying_reads_denies_writes_too_but_denying_writes_leaves_reads_alone() {
        let (_p, _g, e) = engine();
        e.record("npm", Class::Write, Choice::DenyProject).unwrap();
        e.record("ssh", Class::Read, Choice::DenyProject).unwrap();

        assert!(asks(&check(&e, "npm", Class::Read)), "npm may still read");
        assert!(matches!(check(&e, "npm", Class::Write), Outcome::Locked { .. }));
        assert!(matches!(check(&e, "ssh", Class::Read), Outcome::Locked { .. }));
        assert!(matches!(check(&e, "ssh", Class::Write), Outcome::Locked { .. }));
    }

    // ── Deny is a lock, not a pre-answer ─────────────────────────────────

    /// The decision that separates this model from the one it replaced. A
    /// session is the narrowest persisted scope and the developer's own
    /// deliberate space — and it still cannot reach past a global deny.
    #[test]
    fn a_session_allow_cannot_override_a_global_deny() {
        let (_p, _g, e) = engine();
        e.record("curl", Class::Write, Choice::NeverAllow).unwrap();
        e.record("curl", Class::Write, Choice::AllowSession).unwrap();

        match check(&e, "curl", Class::Write) {
            Outcome::Locked { scope, rule } => {
                assert_eq!(scope, GrantScope::Global);
                assert_eq!(rule, GrantEntry::program("curl"));
            }
            other => panic!("a deny must be a lock, got {other:?}"),
        }
    }

    #[test]
    fn a_project_allow_cannot_override_a_global_deny() {
        let (_p, _g, e) = engine();
        e.record("curl", Class::Write, Choice::NeverAllow).unwrap();
        e.record("curl", Class::Write, Choice::AllowProject).unwrap();

        assert!(matches!(check(&e, "curl", Class::Write), Outcome::Locked { .. }));
    }

    /// A locked call is refused outright rather than prompted, because there
    /// is no answer the developer could give at the prompt that would change
    /// it — offering one would be the pre-answer shape ADR 0004 §7 rejected.
    #[test]
    fn a_locked_call_draws_no_prompt_and_names_where_the_rule_lives() {
        let (_p, _g, e) = engine();
        e.record("curl", Class::Write, Choice::NeverAllow).unwrap();

        let outcome = check(&e, "curl", Class::Write);
        assert!(!asks(&outcome));
        let Outcome::Locked { scope, .. } = outcome else { panic!("expected a lock") };
        assert_eq!(scope.where_it_lives(), "~/.mjolnir/permissions.yaml");
    }

    // ── What each row writes ─────────────────────────────────────────────

    #[test]
    fn the_two_once_rows_persist_nothing_anywhere() {
        let (_p, _g, e) = engine();
        e.record("git", Class::Write, Choice::AllowOnce).unwrap();
        e.record("npm", Class::Write, Choice::DenyOnce).unwrap();

        assert!(e.effective_view().grants.is_empty());
        assert!(asks(&check(&e, "git", Class::Write)), "allow-once must not outlive its call");
    }

    #[test]
    fn a_session_row_never_reaches_disk() {
        let (project, _g, e) = engine();
        e.record("git", Class::Read, Choice::AllowSession).unwrap();

        assert_eq!(check(&e, "git", Class::Read), Outcome::Allow);
        assert!(
            !project.path().join(".mjolnir/permissions.yaml").exists(),
            "a session answer must not create a project file"
        );
    }

    /// Row 8 is deliberately blunter than rows 2–4 and 6–7: the whole
    /// program, every class, in the global file.
    #[test]
    fn never_allow_locks_the_whole_program_not_just_its_class() {
        let (_p, _g, e) = engine();
        e.record("curl", Class::Write, Choice::NeverAllow).unwrap();

        assert!(matches!(check(&e, "curl", Class::Read), Outcome::Locked { .. }));
        assert!(matches!(check(&e, "curl", Class::Write), Outcome::Locked { .. }));
    }

    #[test]
    fn the_four_allow_rows_land_in_the_scope_they_name() {
        let (_p, _g, e) = engine();
        e.record("a", Class::Read, Choice::AllowSession).unwrap();
        e.record("b", Class::Read, Choice::AllowProject).unwrap();
        e.record("c", Class::Read, Choice::AllowEverywhere).unwrap();

        let view = e.effective_view();
        let scope_of = |program: &str| {
            view.grants.iter().find(|g| g.entry.program == program).expect("granted").scope
        };
        assert_eq!(scope_of("a"), GrantScope::Session);
        assert_eq!(scope_of("b"), GrantScope::Project);
        assert_eq!(scope_of("c"), GrantScope::Global);
    }

    #[test]
    fn the_prompt_payload_carries_the_declaration_and_the_argv() {
        let (_p, _g, e) = engine();
        let argv = vec!["status".to_string(), "--short".to_string()];

        match e.check("git", Class::Read, &argv) {
            Outcome::Ask(PromptPayload::Tool { program, argv: seen, declared }) => {
                assert_eq!(program, "git");
                assert_eq!(seen, argv);
                assert_eq!(declared, Class::Read);
            }
            other => panic!("expected a tool prompt, got {other:?}"),
        }
    }
}
