//! Aldwin as `git`: every commit Aldwin makes in a developer's project names
//! it as a co-author (ADR 0013).
//!
//! At startup the binary puts a symlink to itself, named `git`, in a
//! directory of its own, and puts that directory first on its own `PATH` —
//! so every process it starts, `run`'s shell, the language server and each
//! MCP server, finds it before the real one. Started under that name, the
//! binary is the shim: it finds the real `git` further down `PATH` and
//! replaces itself with it, adding the trailer to a `git commit` and passing
//! everything else through untouched.
//!
//! Unix only, as the sandbox is: `exec` is what lets the shim leave no
//! process of its own behind, and a symlink is how it gets its name.

use std::ffi::{OsStr, OsString};
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use tempfile::TempDir;

use crate::ShimError;

/// What a commit made through the shim carries.
const TRAILER: &str = "Co-Authored-By: Aldwin <noreply@aldwin.codes>";

/// Git's global options that take the next word as their value. Every other
/// word before the subcommand that starts with `-` is a bare flag or an
/// `--option=value`. `--exec-path` is not here: bare, it prints the path
/// rather than reading one.
const VALUED_OPTIONS: [&str; 8] = [
    "-C",
    "-c",
    "--git-dir",
    "--work-tree",
    "--namespace",
    "--config-env",
    "--super-prefix",
    "--attr-source",
];

/// The shim's directory, installed at the front of this process's `PATH`.
/// Dropping it removes the directory — best effort, on a clean exit.
#[derive(Debug)]
pub struct GitShim {
    dir: TempDir,
}

impl GitShim {
    /// A private directory (`0700`) holding `git`, a symlink to `exe`.
    fn new(exe: &Path) -> io::Result<Self> {
        // Owner-only, so another user cannot decide what `git` is here. A
        // confined `run` can still replace it — temp is on the sandbox's
        // incidental list (ADR 0011) — but whatever it points at runs
        // confined too.
        let dir = tempfile::Builder::new()
            .prefix("aldwin-")
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir()?;
        std::os::unix::fs::symlink(exe, dir.path().join("git"))?;
        Ok(Self { dir })
    }

    /// `path` with this directory in front of it.
    fn ahead_of(&self, path: &OsStr) -> io::Result<OsString> {
        let dirs =
            std::iter::once(self.dir.path().to_path_buf()).chain(std::env::split_paths(path));
        std::env::join_paths(dirs).map_err(io::Error::other)
    }
}

/// Puts the shim first on this process's `PATH`, so everything it goes on
/// to start reaches `git` through it.
///
/// Call it before any thread starts: it sets `PATH`, and changing the
/// environment is only sound while nothing else can be reading it.
///
/// # Errors
///
/// [`ShimError::Install`] when the directory or the symlink cannot be
/// made, the binary cannot find itself, or the directory's path cannot sit
/// in `PATH`; [`ShimError::GitTooOld`] when the git on `PATH` is older
/// than 2.32, which has no `commit --trailer` — shimmed, every commit there
/// would fail. The caller carries on without the shim, and says so.
///
/// # Examples
///
/// ```no_run
/// // First thing in `main`, before the runtime starts a thread.
/// let git_shim = aldwin_cli::git_shim::install();
/// if let Err(e) = &git_shim {
///     eprintln!("commits will not name Aldwin: {e}");
/// }
/// ```
pub fn install() -> Result<GitShim, ShimError> {
    let exe = std::env::current_exe()?;
    // No git at all is not a reason to stay out of the way: the shim then
    // says there is none, as the shell would have.
    if let Some(git) = real_git(&std::env::var_os("PATH").unwrap_or_default(), &exe) {
        // Confined, as every process Aldwin starts is (ADR 0011); the
        // workspace is not read yet, so the working directory stands for it.
        let roots = [std::env::current_dir()?];
        let version =
            aldwin_tools::sandbox::std_command(&git.to_string_lossy(), &["--version"], &roots)
                .and_then(|mut git| git.output())?;
        let version = String::from_utf8_lossy(&version.stdout).trim().to_string();
        if !takes_trailers(&version) {
            return Err(ShimError::GitTooOld { version });
        }
    }
    let shim = GitShim::new(&exe)?;
    let path = shim.ahead_of(&std::env::var_os("PATH").unwrap_or_default())?;
    std::env::set_var("PATH", path);
    Ok(shim)
}

/// When this process was started as `git` — through the shim — runs the
/// real git in its place, and returns only if that could not be done, with
/// the status to exit with. `None` when this process is Aldwin.
///
/// # Examples
///
/// ```no_run
/// fn main() -> std::process::ExitCode {
///     if let Some(status) = aldwin_cli::git_shim::intercept() {
///         return status;
///     }
///     std::process::ExitCode::SUCCESS
/// }
/// ```
pub fn intercept() -> Option<ExitCode> {
    let mut args = std::env::args_os();
    let argv0 = args.next()?;
    (Path::new(&argv0).file_name() == Some(OsStr::new("git")))
        .then(|| exec_real_git(args.collect()))
}

/// Replaces this process with the real git. Returns only on failure: 127
/// when there is no git to run, as a shell says of a missing command, and
/// 126 when there is one and it would not start.
fn exec_real_git(args: Vec<OsString>) -> ExitCode {
    let shim = match std::env::current_exe().and_then(|exe| exe.canonicalize()) {
        Ok(shim) => shim,
        Err(e) => {
            eprintln!("git: Aldwin could not find its own binary to step past it: {e}");
            return ExitCode::from(127);
        }
    };
    let path = std::env::var_os("PATH").unwrap_or_default();
    let Some(git) = real_git(&path, &shim) else {
        eprintln!("git: there is no git on PATH other than Aldwin's own.");
        return ExitCode::from(127);
    };
    let error = Command::new(&git).args(with_trailer(args)).exec();
    eprintln!("git: {} could not be started: {error}", git.display());
    ExitCode::from(126)
}

/// The first `git` on `path` that is an executable file and not a build of
/// Aldwin. Any build, not only `shim` itself: a session started inside
/// another session's `run`, from a different build, puts a second shim on
/// `PATH`, and two shims that each skipped only themselves would hand a
/// commit back and forth forever.
fn real_git(path: &OsStr, shim: &Path) -> Option<PathBuf> {
    std::env::split_paths(path)
        .filter_map(|dir| dir.join("git").canonicalize().ok())
        .find(|target| {
            let executable = target
                .metadata()
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0);
            executable && target.file_name() != shim.file_name()
        })
}

/// Whether `git --version`'s answer names a git with `commit --trailer`
/// (2.32 or later). An answer that cannot be read is taken as new enough: a
/// git that prints something unexpected is far likelier a newer one than one
/// from before 2021.
fn takes_trailers(version: &str) -> bool {
    let mut numbers = version
        .split_whitespace()
        .nth(2)
        .unwrap_or_default()
        .split('.')
        .map(|n| n.parse::<u32>().ok());
    match (numbers.next().flatten(), numbers.next().flatten()) {
        (Some(major), Some(minor)) => (major, minor) >= (2, 32),
        _ => true,
    }
}

/// `args` — git's, without its own name — with the trailer inserted right
/// after the subcommand when that subcommand is `commit`, and unchanged
/// otherwise.
fn with_trailer(mut args: Vec<OsString>) -> Vec<OsString> {
    let mut at = 0;
    while let Some(arg) = args.get(at).and_then(|a| a.to_str()) {
        if VALUED_OPTIONS.contains(&arg) {
            at += 2;
        } else if arg.starts_with('-') {
            at += 1;
        } else {
            break;
        }
    }
    if args.get(at).is_some_and(|a| a == "commit") && !empty_message(&args[at + 1..]) {
        args.splice(at + 1..at + 1, ["--trailer".into(), TRAILER.into()]);
    }
    args
}

/// Whether a `commit`'s arguments give its message explicitly and every part
/// of it is empty — `-m ""`, `--message=`. Git aborts such a commit, and
/// with a trailer it would not: the trailer alone would become the message
/// (the developer's call, 2026-09-27, is to keep git's abort). A message
/// typed into an editor cannot be seen from here.
fn empty_message(args: &[OsString]) -> bool {
    // `commit`'s short flags that take a value, and those whose value is
    // optional and so only ever the rest of the cluster.
    const VALUED: &str = "mFCct";
    const OPTIONAL: &str = "Su";
    // A loop rather than a chain: a message can be the word after its flag,
    // which the loop takes with `words.next()`.
    let (mut given, mut all_empty) = (false, true);
    let mut words = args.iter().map(|a| a.to_string_lossy());
    while let Some(word) = words.next() {
        let message = if word == "--" {
            break;
        } else if let Some(value) = word.strip_prefix("--message=") {
            Some(value.to_string())
        } else if word == "--message" {
            Some(words.next().unwrap_or_default().into_owned())
        } else if let Some(flags) = word.strip_prefix('-').filter(|f| !f.starts_with('-')) {
            // A short cluster: its first flag that takes a value takes the
            // rest of the cluster — or, for a required value, the next word
            // when it ends the cluster. Only `m`'s value is a message.
            flags
                .char_indices()
                .find(|&(_, flag)| VALUED.contains(flag) || OPTIONAL.contains(flag))
                .and_then(|(i, flag)| {
                    let rest = &flags[i + 1..];
                    let value = if rest.is_empty() && VALUED.contains(flag) {
                        words.next().unwrap_or_default().into_owned()
                    } else {
                        rest.to_string()
                    };
                    (flag == 'm').then_some(value)
                })
        } else {
            None
        };
        if let Some(message) = message {
            given = true;
            all_empty &= message.trim().is_empty();
        }
    }
    given && all_empty
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shimmed, a git before 2.32 would fail every commit on
    /// `--trailer`; `install` stays out of the way of one instead.
    #[test]
    fn only_a_git_with_commit_trailers_is_shimmed() {
        assert!(takes_trailers("git version 2.55.0\n"));
        assert!(takes_trailers("git version 2.32.0"));
        assert!(takes_trailers("git version 3.0.1"));
        assert!(takes_trailers("git version 2.39.5 (Apple Git-154)"));
        assert!(!takes_trailers("git version 2.31.8"));
        assert!(!takes_trailers("git version 2.25.1"));
        assert!(!takes_trailers("git version 1.9.5"));
        assert!(takes_trailers("something else entirely"));
    }

    /// With a trailer git would commit an empty message instead of aborting,
    /// leaving the trailer as the whole message.
    #[test]
    fn an_explicitly_empty_message_gets_no_trailer_so_git_still_aborts() {
        for args in [
            &["commit", "-m", ""][..],
            &["commit", "-m", "  "],
            &["commit", "--message="],
            &["commit", "--message", ""],
            &["commit", "-am", ""],
            &["commit", "-m", "", "-m", ""],
        ] {
            assert_eq!(rewritten(args), args.to_vec(), "{args:?}");
        }
    }

    #[test]
    fn a_message_with_anything_in_it_still_gets_the_trailer() {
        assert_eq!(
            rewritten(&["commit", "-m", "", "-m", "body"]),
            after(&["commit"], &["-m", "", "-m", "body"])
        );
        assert_eq!(rewritten(&["commit", "-mx"]), after(&["commit"], &["-mx"]));
        // `-C` takes `m` as the commit to reuse; there is no message flag.
        assert_eq!(
            rewritten(&["commit", "-Cm", ""]),
            after(&["commit"], &["-Cm", ""])
        );
        assert_eq!(rewritten(&["commit"]), after(&["commit"], &[]));
        assert_eq!(
            rewritten(&["commit", "-F", "msg.txt"]),
            after(&["commit"], &["-F", "msg.txt"])
        );
    }

    fn rewritten(args: &[&str]) -> Vec<String> {
        with_trailer(args.iter().map(OsString::from).collect())
            .into_iter()
            .map(|a| a.into_string().unwrap())
            .collect()
    }

    fn after(prefix: &[&str], rest: &[&str]) -> Vec<String> {
        prefix
            .iter()
            .copied()
            .chain(["--trailer", TRAILER])
            .chain(rest.iter().copied())
            .map(String::from)
            .collect()
    }

    #[test]
    fn a_commit_gets_the_trailer_right_after_the_subcommand() {
        assert_eq!(
            rewritten(&["commit", "-m", "x"]),
            after(&["commit"], &["-m", "x"])
        );
    }

    #[test]
    fn global_options_before_the_subcommand_are_stepped_over() {
        for prefix in [
            &["-C", "dir"][..],
            &["-c", "k=v"],
            &["--git-dir=x"],
            &["--git-dir", "x"],
            &["--work-tree", "w", "--no-pager"],
            &[
                "-p",
                "--bare",
                "--literal-pathspecs",
                "--no-replace-objects",
            ],
            &["--config-env", "k=ENV", "-C", "a", "-C", "b"],
        ] {
            let args: Vec<&str> = prefix.iter().copied().chain(["commit", "-a"]).collect();
            let mut commit = prefix.to_vec();
            commit.push("commit");
            assert_eq!(rewritten(&args), after(&commit, &["-a"]), "{prefix:?}");
        }
    }

    #[test]
    fn a_value_that_looks_like_the_subcommand_is_not_it() {
        let args = ["-C", "commit", "log"];
        assert_eq!(rewritten(&args), args);
    }

    #[test]
    fn every_other_subcommand_passes_through_unchanged() {
        for args in [
            &["log", "-1"][..],
            &["status"],
            &["commit-tree", "HEAD^{tree}"],
            &["cherry-pick", "abc"],
            &["-C", "dir", "log", "commit"],
            &["help", "commit"],
        ] {
            assert_eq!(rewritten(args), args, "{args:?}");
        }
    }

    #[test]
    fn git_with_no_subcommand_is_left_alone() {
        assert!(rewritten(&[]).is_empty());
        assert_eq!(rewritten(&["--version"]), ["--version"]);
        assert_eq!(rewritten(&["-C"]), ["-C"]);
    }

    #[test]
    fn the_shim_is_a_private_directory_first_on_path_and_gone_when_dropped() {
        let exe = tempfile::NamedTempFile::new().unwrap();
        let shim = GitShim::new(exe.path()).unwrap();
        let dir = shim.dir.path().to_path_buf();

        let mode = dir.metadata().unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700, "{mode:o}");
        assert_eq!(std::fs::read_link(dir.join("git")).unwrap(), exe.path());

        let path = shim
            .ahead_of(OsStr::new("/usr/local/bin:/usr/bin"))
            .unwrap();
        let dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
        assert_eq!(
            dirs,
            [dir.clone(), "/usr/local/bin".into(), "/usr/bin".into()]
        );

        drop(shim);
        assert!(!dir.exists());
    }

    #[test]
    fn the_real_git_is_the_first_on_path_that_is_not_a_build_of_aldwin() {
        let root = tempfile::tempdir().unwrap();
        let exec = |path: &Path| {
            std::fs::write(path, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        let dir = |name: &str| {
            let dir = root.path().join(name);
            std::fs::create_dir(&dir).unwrap();
            dir
        };

        let this = root.path().join("aldwin");
        exec(&this);
        let other_build = dir("other").join("aldwin");
        exec(&other_build);
        let shim = dir("shim");
        std::os::unix::fs::symlink(&this, shim.join("git")).unwrap();
        let nested = dir("nested");
        std::os::unix::fs::symlink(&other_build, nested.join("git")).unwrap();
        let empty = dir("empty");
        let not_executable = dir("plain");
        std::fs::write(not_executable.join("git"), "").unwrap();
        let real = dir("real");
        exec(&real.join("git"));

        let path = std::env::join_paths([&shim, &nested, &empty, &not_executable, &real]).unwrap();
        let this = this.canonicalize().unwrap();
        assert_eq!(
            real_git(&path, &this),
            Some(real.join("git").canonicalize().unwrap())
        );

        let path = std::env::join_paths([&shim, &nested]).unwrap();
        assert_eq!(real_git(&path, &this), None);
    }
}
