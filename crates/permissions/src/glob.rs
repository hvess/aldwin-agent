//! Grant-pattern matching. Per mjolnir-permissions.md's Pattern vocabulary,
//! both tool-arg globs (`shell:cargo test*`) and path globs (`read:./**`) are
//! the same grammar: `*` matches any run of characters, including none and
//! including path separators; everything else matches literally. Exact match
//! is simply a pattern with no `*`. Consecutive `*`s (`**`) behave exactly
//! like a single `*` under this algorithm, so no special-casing is needed.

/// Two-pointer greedy match with backtracking to the most recent `*` on a
/// mismatch — the standard O(n*m)-worst-case algorithm for a `*`-only glob.
pub fn glob_match(pattern: &str, target: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let target: Vec<char> = target.chars().collect();

    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None; // (pattern index after '*', target index at time of '*')

    while ti < target.len() {
        if pi < pattern.len() && pattern[pi] != '*' && pattern[pi] == target[ti] {
            pi += 1;
            ti += 1;
        } else if pi < pattern.len() && pattern[pi] == '*' {
            star = Some((pi + 1, ti));
            pi += 1;
        } else if let Some((sp, st)) = star {
            pi = sp;
            ti = st + 1;
            star = Some((sp, ti));
        } else {
            return false;
        }
    }
    while pi < pattern.len() && pattern[pi] == '*' {
        pi += 1;
    }
    pi == pattern.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match_has_no_wildcards() {
        assert!(glob_match("cargo test", "cargo test"));
        assert!(!glob_match("cargo test", "cargo tests"));
    }

    #[test]
    fn trailing_star_matches_any_suffix() {
        assert!(glob_match("cargo test*", "cargo test"));
        assert!(glob_match("cargo test*", "cargo test -- foo::bar"));
        assert!(!glob_match("cargo test*", "cargo install foo"));
    }

    #[test]
    fn double_star_behaves_like_single_star() {
        assert!(glob_match("./**", "./src/main.rs"));
        assert!(glob_match("./**", "./"));
    }

    #[test]
    fn star_in_the_middle_matches_across_the_gap() {
        assert!(glob_match("git *push", "git force push"));
        assert!(!glob_match("git *push", "git force pull"));
    }

    #[test]
    fn empty_pattern_only_matches_empty_target() {
        assert!(glob_match("", ""));
        assert!(!glob_match("", "x"));
        assert!(glob_match("*", ""));
        assert!(glob_match("*", "anything"));
    }
}

#[cfg(test)]
mod adr_0001_tests {
    use super::*;

    /// The `all` access tier writes `shell:*`. If `*` did not match a
    /// command with arguments, that tier would silently grant nothing.
    #[test]
    fn a_bare_star_matches_any_command() {
        assert!(glob_match("*", "cargo test -p gateway"));
        assert!(glob_match("*", "ls"));
        assert!(glob_match("*", ""));
    }

    /// The program unit `cargo *` matches any cargo invocation with
    /// arguments, and deliberately not a bare `cargo` — the trailing space
    /// is literal, so an argument-less command re-prompts rather than
    /// slipping through.
    #[test]
    fn a_program_glob_matches_that_programs_commands_only() {
        assert!(glob_match("cargo *", "cargo test -p gateway"));
        assert!(glob_match("cargo *", "cargo build"));
        assert!(!glob_match("cargo *", "cargo"), "fails closed on a bare program name");
        assert!(!glob_match("cargo *", "git status"), "must not leak to another program");
    }

    /// `read:**` is what the `read` tier writes.
    #[test]
    fn a_double_star_matches_any_path() {
        assert!(glob_match("**", "./crates/tui/src/ui.rs"));
        assert!(glob_match("**", "main.rs"));
    }
}
