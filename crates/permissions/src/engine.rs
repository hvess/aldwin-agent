use aldwin_config::{Class, Config, GrantEntry};

/// Where a lock came from — which file the refusal points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockScope {
    Project,
    Global,
}

impl LockScope {
    /// How a refusal names the place a rule lives, for a developer who has to
    /// go and change it.
    pub fn where_it_lives(self) -> &'static str {
        match self {
            LockScope::Project => "<project>/.aldwin/permissions.yaml",
            LockScope::Global => "~/.aldwin/permissions.yaml",
        }
    }
}

/// What [`Locks::check`] concluded. There are two answers and no third: a
/// call either runs or is locked. "Ask" left with ADR 0009 — the sandbox
/// answers what a prompt used to, and the review answers the rest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Allow,
    Locked { scope: LockScope, rule: GrantEntry },
}

/// The deny lists of both scopes, read live through `Config` (which is cheap
/// to clone — internally `Arc` — so a `/reload-config` is seen on the next
/// check without anything being swapped).
pub struct Locks {
    config: Config,
}

impl Locks {
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    /// Resolves one call: a program, and the class the agent declared for it.
    ///
    /// The declaration is an input here, not a trusted fact. Nothing in this
    /// function verifies it — that is the sandbox's job at execution time
    /// (ADR 0004 §4, which ADR 0009 keeps), and the split is deliberate: a
    /// policy engine that also tried to judge what a command does would be
    /// making exactly the guess the sandbox exists to avoid.
    ///
    /// The nearest scope's lock is the one named, because it is the one a
    /// developer is most likely to be able to change. Which one is found does
    /// not change the answer — every deny is a lock.
    pub fn check(&self, program: &str, declared: Class) -> Outcome {
        let project = self.config.project_permissions();
        if let Some(rule) = project.deny.iter().find(|e| e.denies(program, declared)) {
            return Outcome::Locked {
                scope: LockScope::Project,
                rule: rule.clone(),
            };
        }
        let global = self.config.global_permissions();
        if let Some(rule) = global.deny.iter().find(|e| e.denies(program, declared)) {
            return Outcome::Locked {
                scope: LockScope::Global,
                rule: rule.clone(),
            };
        }
        Outcome::Allow
    }

    /// Every lock in force, nearest scope first — what a developer glancing
    /// at "what can this agent not do" is asking.
    pub fn all(&self) -> Vec<(LockScope, GrantEntry)> {
        let mut out: Vec<(LockScope, GrantEntry)> = self
            .config
            .project_permissions()
            .deny
            .into_iter()
            .map(|e| (LockScope::Project, e))
            .collect();
        out.extend(
            self.config
                .global_permissions()
                .deny
                .into_iter()
                .map(|e| (LockScope::Global, e)),
        );
        out
    }

    /// The config behind the locks — for a test that has to write a deny
    /// the way the developer would.
    #[doc(hidden)]
    pub fn config_for_tests(&self) -> &Config {
        &self.config
    }

    /// The keys `permissions.yaml` still parses but nothing reads — `allow:`
    /// and `default:` from the previous model — so the developer can be told
    /// once, at startup, that the file says something the product no longer
    /// does. Returns the scopes that carry one.
    pub fn stale_keys(&self) -> Vec<LockScope> {
        let mut out = Vec::new();
        let project = self.config.project_permissions();
        if project.default.is_some() || !project.allow.is_empty() {
            out.push(LockScope::Project);
        }
        let global = self.config.global_permissions();
        if global.default.is_some() || !global.allow.is_empty() {
            out.push(LockScope::Global);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aldwin_config::{GrantList, Rung, Scope};

    fn locks() -> (tempfile::TempDir, tempfile::TempDir, Locks) {
        let project = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let config = Config::open_at(project.path(), global.path().join(".aldwin")).unwrap();
        (project, global, Locks::new(config))
    }

    /// The floor is inverted from ADR 0004: with nothing written, everything
    /// runs. The sandbox and the review are the gates now, not this crate.
    #[test]
    fn nothing_is_locked_in_a_fresh_project() {
        let (_p, _g, l) = locks();
        assert_eq!(l.check("git", Class::Read), Outcome::Allow);
        assert_eq!(l.check("rm", Class::Write), Outcome::Allow);
        assert!(l.all().is_empty());
    }

    #[test]
    fn a_deny_is_a_lock_at_every_class_above_it() {
        let (_p, _g, l) = locks();
        l.config
            .add_grant(
                Scope::Project,
                GrantList::Deny,
                GrantEntry::classed("git", Class::Write),
            )
            .unwrap();

        assert_eq!(
            l.check("git", Class::Read),
            Outcome::Allow,
            "a write deny leaves reads alone"
        );
        assert!(matches!(
            l.check("git", Class::Write),
            Outcome::Locked {
                scope: LockScope::Project,
                ..
            }
        ));

        l.config
            .add_grant(Scope::Global, GrantList::Deny, GrantEntry::program("curl"))
            .unwrap();
        assert!(
            matches!(
                l.check("curl", Class::Read),
                Outcome::Locked {
                    scope: LockScope::Global,
                    ..
                }
            ),
            "a bare program deny locks every class"
        );
    }

    /// An allow used to outrank the rung and a deny outranked both. With no
    /// allows consulted, a deny is simply a deny — and the previous model's
    /// `allow:` cannot lift it.
    #[test]
    fn a_stale_allow_does_not_lift_a_lock() {
        let (_p, _g, l) = locks();
        l.config
            .add_grant(
                Scope::Project,
                GrantList::Allow,
                GrantEntry::classed("curl", Class::Write),
            )
            .unwrap();
        l.config
            .add_grant(Scope::Global, GrantList::Deny, GrantEntry::program("curl"))
            .unwrap();
        assert!(matches!(
            l.check("curl", Class::Write),
            Outcome::Locked { .. }
        ));
    }

    /// The nearest lock is the one named, so the refusal points at the file
    /// the developer can most easily change.
    #[test]
    fn the_nearest_scope_is_the_one_named() {
        let (_p, _g, l) = locks();
        l.config
            .add_grant(Scope::Global, GrantList::Deny, GrantEntry::program("rm"))
            .unwrap();
        l.config
            .add_grant(Scope::Project, GrantList::Deny, GrantEntry::program("rm"))
            .unwrap();
        assert!(matches!(
            l.check("rm", Class::Read),
            Outcome::Locked {
                scope: LockScope::Project,
                ..
            }
        ));
        assert_eq!(l.all().len(), 2);
    }

    /// A file from the previous model still loads, and the keys nothing
    /// reads are reported rather than silently accepted.
    #[test]
    fn stale_keys_are_reported_not_honoured() {
        let (_p, _g, l) = locks();
        assert!(l.stale_keys().is_empty());
        l.config.ensure_permissions(Scope::Project).unwrap();
        l.config
            .set_default_rung(Scope::Project, Rung::Write)
            .unwrap();
        assert_eq!(l.stale_keys(), vec![LockScope::Project]);
        assert_eq!(
            l.check("anything", Class::Write),
            Outcome::Allow,
            "the rung neither allows nor denies now"
        );
    }
}
