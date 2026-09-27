//! The git shim against real git (ADR 0013): the test-built binary, reached
//! through a symlink named `git`, commits in a scratch repository.
#![cfg(unix)]

use std::path::{Path, PathBuf};

use assert_cmd::assert::Assert;
use assert_cmd::Command;

const TRAILER: &str = "Co-Authored-By: Aldwin <noreply@aldwin.codes>";

/// A scratch repository, and a directory holding the shim.
struct Scratch {
    repo: tempfile::TempDir,
    shim: tempfile::TempDir,
}

impl Scratch {
    fn new() -> Self {
        let scratch = Self {
            repo: tempfile::tempdir().unwrap(),
            shim: tempfile::tempdir().unwrap(),
        };
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_aldwin"), scratch.shim_git()).unwrap();
        scratch.git(&["init", "-q"]).success();
        scratch
    }

    fn shim_git(&self) -> PathBuf {
        self.shim.path().join("git")
    }

    /// `git args` through the shim, with the shim first on `PATH` as a
    /// session puts it.
    fn git(&self, args: &[&str]) -> Assert {
        let path = std::env::join_paths(
            std::iter::once(self.shim.path().to_path_buf())
                .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
        )
        .unwrap();
        isolated(Command::new(self.shim_git()), self.repo.path())
            .env("PATH", path)
            .args(args)
            .assert()
    }

    /// The stdout of `git args` through the shim, which must succeed.
    fn out(&self, args: &[&str]) -> String {
        String::from_utf8(self.git(args).success().get_output().stdout.clone()).unwrap()
    }
}

/// A git that depends on nothing of the machine it runs on: a fixed
/// identity, no global or system config, and none of the variables a hook
/// running these tests would have set.
fn isolated(mut cmd: Command, dir: &Path) -> Command {
    cmd.current_dir(dir)
        .env("GIT_AUTHOR_NAME", "Developer")
        .env("GIT_AUTHOR_EMAIL", "developer@example.com")
        .env("GIT_COMMITTER_NAME", "Developer")
        .env("GIT_COMMITTER_EMAIL", "developer@example.com")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_DIR")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_WORK_TREE");
    cmd
}

#[test]
fn a_commit_through_the_shim_names_aldwin_once() {
    let scratch = Scratch::new();
    scratch
        .git(&["commit", "--allow-empty", "-q", "-m", "Add the thing"])
        .success();
    assert_eq!(
        scratch.out(&["log", "-1", "--format=%B"]),
        format!("Add the thing\n\n{TRAILER}\n\n")
    );
}

/// Git aborts a commit with an empty message; with the trailer added it
/// would commit one whose whole message is the trailer. Through the shim it
/// still aborts, and nothing is committed.
#[test]
fn an_empty_message_still_aborts_the_commit() {
    let scratch = Scratch::new();
    scratch
        .git(&["commit", "--allow-empty", "-q", "-m", ""])
        .failure();
    scratch
        .git(&["rev-parse", "--verify", "-q", "HEAD"])
        .failure();
}

#[test]
fn a_message_that_already_names_aldwin_is_not_given_it_twice() {
    let scratch = Scratch::new();
    let message = format!("Add the thing\n\n{TRAILER}");
    scratch
        .git(&["-C", ".", "commit", "--allow-empty", "-q", "-m", &message])
        .success();
    let last = scratch.out(&["log", "-1", "--format=%B"]);
    assert_eq!(last.matches(TRAILER).count(), 1);
}

#[test]
fn anything_but_a_commit_passes_through_unchanged() {
    let scratch = Scratch::new();
    let through_shim = scratch.out(&["rev-parse", "--is-inside-work-tree"]);
    let direct = isolated(Command::new("git"), scratch.repo.path())
        .args(["rev-parse", "--is-inside-work-tree"])
        .assert()
        .success();
    assert_eq!(
        through_shim.as_bytes(),
        direct.get_output().stdout.as_slice()
    );

    // `commit-tree` makes a commit without `git commit`: no trailer.
    let tree = scratch.out(&["write-tree"]);
    let commit = scratch.out(&["commit-tree", tree.trim(), "-m", "plumbing"]);
    let message = scratch.out(&["cat-file", "commit", commit.trim()]);
    assert!(!message.contains(TRAILER));
}

#[test]
fn a_shim_with_no_real_git_behind_it_says_so_and_exits_127() {
    let scratch = Scratch::new();
    let assert = isolated(Command::new(scratch.shim_git()), scratch.repo.path())
        .env("PATH", scratch.shim.path())
        .arg("status")
        .assert()
        .code(127);
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(stderr.contains("no git on PATH"), "{stderr}");
}
