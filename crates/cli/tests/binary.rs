//! The `aldwin` binary as a developer runs it: the paths that end before
//! the TUI takes the terminal, so they can be run without one.

use assert_cmd::Command;

fn aldwin() -> Command {
    Command::cargo_bin("aldwin").expect("the aldwin binary is built for its own tests")
}

#[test]
fn version_names_the_build_and_exits_cleanly() {
    let output = aldwin().arg("--version").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.starts_with("aldwin "), "{stdout}");
}

#[test]
fn an_argument_is_refused() {
    // Zero-arg binary (aldwin-cli.md): everything lives in `.aldwin/`.
    aldwin().arg("--resume").assert().failure();
}

/// A file that does not parse stops the start, names itself, and exits
/// non-zero — before anything draws.
#[test]
fn a_malformed_config_file_refuses_to_start_and_names_the_file() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let aldwin_dir = project.path().join(".aldwin");
    std::fs::create_dir(&aldwin_dir).unwrap();
    std::fs::write(aldwin_dir.join("provider.yaml"), "version: [unclosed\n").unwrap();

    let output = aldwin()
        .current_dir(project.path())
        .env("HOME", home.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.starts_with("aldwin: "), "{stderr}");
    assert!(stderr.contains("provider.yaml"), "{stderr}");
}
