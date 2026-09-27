//! The git shim inside the sandbox (ADR 0011, ADR 0013): a commit by the
//! confined `run` tool names Aldwin.
//!
//! Keep this the only test in its binary: it sets `PATH` and git's
//! variables, which is unsound while another test reads the environment.
#![cfg(unix)]

use std::sync::Arc;

use aldwin_core::{DispatchContext, StepId, TurnId};
use aldwin_tools::{builtin_registry, Staging, Workspace};

#[tokio::test]
async fn a_commit_made_by_run_names_aldwin() {
    let repo = tempfile::tempdir().unwrap();
    let shim = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_aldwin"), shim.path().join("git")).unwrap();
    let path = std::env::join_paths(
        std::iter::once(shim.path().to_path_buf())
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    std::env::set_var("PATH", path);
    for (key, value) in [
        ("GIT_AUTHOR_NAME", "Developer"),
        ("GIT_AUTHOR_EMAIL", "developer@example.com"),
        ("GIT_COMMITTER_NAME", "Developer"),
        ("GIT_COMMITTER_EMAIL", "developer@example.com"),
        ("GIT_CONFIG_GLOBAL", "/dev/null"),
        ("GIT_CONFIG_NOSYSTEM", "1"),
    ] {
        std::env::set_var(key, value);
    }
    for key in ["GIT_DIR", "GIT_INDEX_FILE", "GIT_WORK_TREE"] {
        std::env::remove_var(key);
    }

    let workspace = Workspace::new(repo.path());
    let registry = builtin_registry(workspace.clone(), Arc::new(Staging::new(workspace)));
    let run = registry.get("run").expect("run is a built-in");
    let (events, _events) = tokio::sync::mpsc::channel(8);
    let ctx = DispatchContext::for_testing(TurnId(1), StepId(1), events, Default::default());
    let command = [
        "command -v git",
        "git init -q",
        "git commit --allow-empty -q -m 'Add the thing'",
        "git log -1 --format=%B",
    ]
    .join(" && ");
    let output = run
        .call("call-1", serde_json::json!({ "command": command }), &ctx)
        .await
        .expect("the commit runs");

    assert!(
        output.contains(&shim.path().join("git").display().to_string()),
        "the shell reached git through the shim: {output}"
    );
    assert!(
        output.contains("Add the thing\n\nCo-Authored-By: Aldwin <noreply@aldwin.codes>"),
        "{output}"
    );
}
