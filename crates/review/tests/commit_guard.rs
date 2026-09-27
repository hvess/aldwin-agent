//! The `PreToolUse` guard in `.claude/hooks/commit-guard.sh`, run as Claude
//! Code runs it: the tool call's JSON on stdin, a deny decision or nothing on
//! stdout.
//!
//! Stage 10's other half: git's pre-commit hook runs the gate, and the guard
//! refuses the plain spellings of the ways around it; a spelling built to get
//! past it is out of scope (Decision 16). Both failures are tested: letting a
//! plain way around through (the dangerous one), and refusing an ordinary
//! commit, e.g. one whose message contains `-n`.

use std::path::PathBuf;

use assert_cmd::Command;

fn guard() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.claude/hooks/commit-guard.sh")
}

/// Whether the guard refuses `command`.
fn refuses(command: &str) -> bool {
    let input = serde_json::json!({ "tool_input": { "command": command } });
    let assert = Command::new(guard())
        .write_stdin(input.to_string())
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    if stdout.trim().is_empty() {
        return false;
    }
    let decision: serde_json::Value = serde_json::from_str(&stdout).expect("the guard prints JSON");
    decision["hookSpecificOutput"]["permissionDecision"] == "deny"
}

#[test]
fn every_way_around_the_hook_is_refused() {
    for command in [
        "git commit -nm x",
        "git commit -n -m x",
        "git commit --no-verify -m x",
        "git commit \"--no-verify\" -m x",
        "git -C . commit -n -m x",
        "git -c user.name=x commit -n",
        "cd x && git commit -n -m y",
        "git add -A\ngit commit -n -m x",
        "git -c core.hooksPath=/dev/null commit -m x",
        "git -c \"core.hooksPath=/dev/null\" commit -m x",
        "git cherry-pick abc",
        "git -C . rebase -i main",
        "git revert HEAD",
        "git am < p.patch",
        "rm .git/aldwin-review/x.json",
        "git commit -n -m \"unbalanced",
        // git takes an unambiguous abbreviation of a long option.
        "git commit --no-veri -m x",
        "git commit --no-verif -m x",
        // A command run by another shell, or substituted, is checked too.
        "bash -c 'git commit --no-verify -m x'",
        "sh -c \"git commit -n -m x\"",
        "bash -lc 'cd x && git commit -n -m y'",
        "bash -c 'git cherry-pick abc'",
        "`git commit -n -m x`",
        "echo $(git commit -n -m x)",
        "eval git commit -n -m x",
        // The gate knows an agent by AGENT; clearing it would commit as the
        // developer.
        "AGENT= git commit -m x",
        "env -u AGENT git commit -m x",
        "env --unset=AGENT git commit -m x",
        "unset AGENT; git commit -m x",
        "unset FOO AGENT && git commit -m x",
        "env -i git commit -m x",
        "env - git commit -m x",
        "export AGENT=; git commit -m x",
        // Builtins that assign it an empty value.
        "read AGENT </dev/null; git commit -m x",
        "printf -v AGENT '' && git commit -m x",
        "mapfile -t AGENT </dev/null; git commit -m x",
        // The shell joins quotes and expands braces before it runs.
        "unset AG''ENT; git commit -m x",
        "unset \"AG\"ENT; git commit -m x",
        "unset {AGENT,X}; git commit -m x",
        // Clearing the whole environment clears it too.
        "env -iu FOO git commit -m x",
        "env -0i git commit -m x",
        "env --ignore-env git commit -m x",
        "env -u FOO -i git commit -m x",
        "exec -c git commit -m x",
        // A command wrapped onto a second line is still one command.
        "git commit \\\n  --no-verify -m x",
        "git commit \\\n  -n -m x",
        "git commit -m x \\\n  --no-verify",
        // A global option, valued or unknown, must not hide the subcommand.
        "git --config-env x=HOME commit --no-verify -m hi",
        "git --attr-source HEAD commit -n -m hi",
        "git --super-prefix sub/ commit -n -m hi",
        "git --unknown-global value commit -n -m hi",
        "git --exec-path commit -n -m hi",
        // A merge commit runs the same gate through pre-merge-commit.
        "git merge --no-verify feature",
        "git pull --no-verify origin main",
        "git merge --no-verif feature",
        "git -C . merge --no-verify feature",
        // Git config keys ignore case.
        "git -c core.hookspath=/dev/null commit -m x",
        "git -c CORE.HOOKSPATH=/dev/null commit -m x",
        // Un-exporting the gate's variable hides it from git.
        "export -n AGENT; git commit -m x",
        "declare +x AGENT; git commit -m x",
        "typeset +x AGENT; git commit -m x",
        // A command-line alias hides the subcommand.
        "git -c alias.ci=commit ci -n -m x",
        "git -c 'alias.ci=commit -n' ci -m x",
        // A saved alias hides it in a later command.
        "git config alias.ci 'commit --no-verify'",
        "git config --global Alias.ci commit",
        // The shell joins a quoted split back into the setting.
        "git -c core.hooks''Path=/dev/null commit -m x",
        "git -c 'core.'hooksPath=/dev/null commit -m x",
    ] {
        assert!(refuses(command), "let through: {command:?}");
    }
}

#[test]
fn an_ordinary_commit_is_let_through_whatever_its_message_says() {
    for command in [
        "git commit -m \"x\"",
        "git commit -m \"fix the -n flag\"",
        "git commit -m \"mention --no-verify in prose\"",
        "git commit -m 'drop -n'",
        "git commit -m -n",
        "git commit -am \"a -n b\"",
        "git commit --message=-n",
        "git commit -F- <<EOF\nfix -n\nEOF",
        "git commit --amend --no-edit",
        "git commit -m x && git log -n 5",
        "git commit -m \"unbalanced",
        "git commit --no-edit --amend",
        "git commit --no-verbose -m x",
        "git --config-env x=HOME commit -m hi",
        // An agent's usual message form: a heredoc inside `$(…)`.
        "git commit -m \"$(cat <<'EOF'\nfix: handle -n in messages\n\nCo-Authored-By: Aldwin <noreply@aldwin.codes>\nEOF\n)\"",
    ] {
        assert!(!refuses(command), "refused: {command:?}");
    }
}

#[test]
fn commands_that_are_not_commits_are_let_through() {
    for command in [
        "git status",
        "git log -n 5",
        "git --no-pager log -n 3",
        "git merge -n feature",
        "git merge --no-verify-signatures feature",
        "git pull origin main",
        "export PATH=$PATH:/opt/bin; cargo test",
        "declare -a list; echo ok",
        "git -c user.name=x log -n 1",
        "cargo test -n",
        "echo \"git commit -n\"",
        "bash -c 'cargo test'",
        "env FOO=1 cargo test",
        "echo $AGENT",
        "echo \"${AGENT}\"",
        "MY_AGENT=x cargo run",
        "AGENT_HOME=x cargo run",
        "env FOO=1 cargo test -- --ignored",
        "cargo test \\\n  -p aldwin-tools",
    ] {
        assert!(!refuses(command), "refused: {command:?}");
    }
}
