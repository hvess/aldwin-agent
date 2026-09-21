//! `run` — the tool that replaced `shell` (ADR 0004 §1).
//!
//! The difference is not cosmetic. `shell` took one opaque string and handed
//! it to an interpreter, so a grant could never mean more than "some text
//! matched a glob": `cargo test*` also matched `cargo test && anything`,
//! because `&&` was syntax. `run` takes a **program and a list of
//! arguments** and calls `execve` directly. `&&`, `|`, `;`, backticks and
//! `$(...)` are ordinary characters with no power to chain a second command
//! onto an approved first one, because nothing ever parses them.
//!
//! What that buys is the thing the whole model needed: a call is one program,
//! so a grant can name that program, and the class the agent declares for the
//! call can be held to its word by running it where writing is impossible.
//!
//! What it costs is pipelines and redirection. `cargo test | head` is not
//! expressible here and is not meant to be — if it comes back it comes back
//! as a list of stages, each a program with its own grant, never as a string.
//! `sh` and `bash` are programs like any other: granting one is granting
//! arbitrary execution, which is now a visible act rather than the default.
//!
//! ADR 0007 brought this tool into the model the other three built-ins live
//! in: path arguments are contained by the same [`Workspace`], a call can name
//! its working directory (so "work over there" is not `bash -c 'cd … && …'`),
//! and a timeout keeps what the program had already written.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;

use async_trait::async_trait;
use aldwin_permissions::Class;
use serde_json::{json, Value};
use tokio::io::AsyncReadExt;

use crate::error::ToolError;
use crate::gate::ApprovalGate;
use crate::paths::Workspace;
use crate::registry::{PermissionRequest, Tool, ToolDescriptor, ToolSource};
use crate::sandbox;

const DEFAULT_TIMEOUT_SECS: u64 = 120;
const OUTPUT_CAP_BYTES: usize = 50 * 1024;
/// What is *held* per stream while the program runs. Only `OUTPUT_CAP_BYTES`
/// is ever shown, so holding more is memory a chatty program — `yes`, a build
/// log — can grow without bound. The slack keeps a character that straddles
/// the cap whole, and lets [`cap`] see that there was more.
const OUTPUT_KEEP_BYTES: usize = OUTPUT_CAP_BYTES + 4;

pub struct RunTool {
    descriptor: ToolDescriptor,
    workspace:  Workspace,
}

/// One call's parsed input.
struct RunArgs {
    program: String,
    args:    Vec<String>,
    class:   Class,
    timeout: u64,
    /// Working directory as the model wrote it, before containment.
    cwd:     Option<String>,
}

impl RunTool {
    pub fn new(workspace: Workspace) -> Self {
        Self {
            descriptor: ToolDescriptor {
                name:        "run".into(),
                description: "Run a program in the project root. \
                              Give the program and its arguments separately — there is no shell, so \
                              pipes, redirection, globbing and `&&` do not apply. \
                              `args` holds only what follows the program: `ls -la` is \
                              program \"ls\", args [\"-la\"]. \
                              A non-zero exit comes back as an error carrying the exit code and \
                              both streams; read the code before concluding anything broke, since \
                              some programs use it to report a result (`grep` exits 1 when nothing \
                              matched). \
                              Declare `class`: \"read\" if the call only observes, \"write\" if it may \
                              change anything. A call declared \"read\" is executed with the project \
                              read-only and the network unreachable, so an inaccurate declaration \
                              fails rather than causing damage. Declare reads as reads: \"write\" is \
                              not the safe default, it is a broader grant and it costs the developer \
                              a prompt they did not need to see. \
                              `cwd` sets the working directory for this one call; it persists no \
                              further than the call, so pass it every time rather than expecting an \
                              earlier one to stick."
                    .into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "program":      { "type": "string", "description": "The program to run, e.g. \"git\"." },
                        "args":         { "type": "array", "items": { "type": "string" }, "default": [],
                                          "description": "Arguments after the program name, one element each — \
                                                          [\"status\", \"--short\"], not [\"git\", \"status\"]. \
                                                          Do not repeat the program here." },
                        "class":        { "type": "string", "enum": ["read", "write"] },
                        "timeout_secs": { "type": "integer", "minimum": 1 },
                        "cwd":          { "type": "string",
                                          "description": "Working directory for this call. Defaults to the project root. \
                                                          Must be inside the workspace — the roots are listed in \
                                                          the session context, and a refusal names them." },
                    },
                    "required": ["program", "class"],
                }),
                edit_class: false,
                source:     ToolSource::Builtin,
            },
            workspace,
        }
    }
}

fn parse(input: &Value) -> Result<RunArgs, ToolError> {
    let invalid = |message: &str| ToolError::InvalidInput { tool: "run".into(), message: message.into() };

    let program = input
        .get("program")
        .and_then(Value::as_str)
        .filter(|p| !p.is_empty())
        .ok_or_else(|| invalid("missing \"program\" string field"))?
        .to_string();

    // A program is a name, not a command line. Rejecting a whitespace-bearing
    // program is what stops `{"program": "git status"}` from quietly becoming
    // a grant for a program called `git status` that no entry will ever match
    // — and, worse, from reading as though the argv split had happened.
    if program.split_whitespace().count() != 1 {
        return Err(invalid("\"program\" names one program; put its arguments in \"args\""));
    }

    let args = match input.get("args") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| v.as_str().map(str::to_string).ok_or_else(|| invalid("every entry in \"args\" must be a string")))
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => return Err(invalid("\"args\" must be an array of strings")),
    };

    let class = match input.get("class").and_then(Value::as_str) {
        Some("read") => Class::Read,
        Some("write") => Class::Write,
        _ => return Err(invalid("\"class\" must be \"read\" or \"write\" — say what this call does")),
    };

    let timeout = input
        .get("timeout_secs")
        .and_then(Value::as_u64)
        .filter(|s| *s > 0)
        .unwrap_or(DEFAULT_TIMEOUT_SECS);

    let cwd = input.get("cwd").and_then(Value::as_str).filter(|c| !c.is_empty()).map(str::to_string);

    Ok(RunArgs { program, args, class, timeout, cwd })
}

/// Which of a call's arguments name a path we are pointing the program at,
/// and therefore have to be inside the workspace.
///
/// This cannot be exact, and pretending otherwise would rebuild the shipped
/// classification table ADR 0004 rejected — argv meaning is the program's,
/// not ours. What it can be is *sound in the direction that matters*: an
/// argument is checked when it is an absolute path, or when it climbs with
/// `..`. Everything else is relative without climbing, so it resolves under
/// a working directory that is itself already contained, and needs no check
/// of its own.
///
/// That leaves globs and patterns alone (`--include=*.kts`, `*/build/*`
/// never resolve to a real path outside a root) while catching the shape
/// that actually escaped in the observed session: `find ~
/// -iname …`, and `rm -rf /…/proton-calendar`.
///
/// **The hole, stated rather than papered over.** A path inside a string
/// argument is invisible here — `bash -c 'cd /elsewhere && …'` is one
/// argument that neither starts with `/` nor climbs. Granting a shell is
/// already granting arbitrary execution (ADR 0004 §1) and this does not
/// change that; it narrows what every *other* program can be pointed at, and
/// `cwd` removes the main reason to reach for a shell at all.
fn path_like(arg: &str) -> Option<&str> {
    let candidate = match arg.split_once('=') {
        // `--flag=/some/path` — check the value, not the whole token. The
        // same for a bare `key=value` (`dd of=/etc/x`): taken whole, the key
        // reads as a directory name, so `of=../../x` normalized to `x` and
        // passed as contained.
        Some((key, value)) if !key.contains('/') => value,
        // `-C/elsewhere`, `-f/etc/x` — a value glued to a short flag. Skipping
        // every `-…` token let these through to `execve` unchecked.
        _ if arg.starts_with('-') && !arg.starts_with("--") && arg.len() > 2 && arg.is_char_boundary(2) => &arg[2..],
        _ => arg,
    };
    if candidate.is_empty() || candidate.starts_with('-') {
        return None;
    }
    let climbs = Path::new(candidate).components().any(|c| matches!(c, std::path::Component::ParentDir));
    if climbs || (candidate.starts_with('/') && first_component_exists(candidate)) {
        return Some(candidate);
    }
    None
}

/// Whether an absolute-looking argument points into the real filesystem.
///
/// `sed -n /pattern/p` and `grep /api/v1` carry arguments that start with
/// `/` and are not paths. What separates them from `/etc/passwd` is that
/// nothing called `/pattern` exists: an argument whose first component is
/// absent from `/` is not pointing at anything, and creating it would need
/// write access to `/` itself. One `stat`, no table of programs.
fn first_component_exists(candidate: &str) -> bool {
    match Path::new(candidate).components().nth(1) {
        Some(first) => Path::new("/").join(first.as_os_str()).exists(),
        None => true, // the argument is `/` itself
    }
}

#[async_trait]
impl Tool for RunTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn permission(&self, input: &Value) -> Result<PermissionRequest, ToolError> {
        let args = parse(input)?;
        Ok(PermissionRequest { program: args.program, class: args.class, argv: args.args })
    }

    async fn call(&self, _call_id: &str, input: Value, _gate: &dyn ApprovalGate) -> Result<String, ToolError> {
        let args = parse(&input)?;

        // Containment first, before anything is spawned. The working
        // directory is resolved as a path in its own right, then every
        // path-like argument is checked against *it* rather than against the
        // project root — `cat notes.md` with `cwd` set elsewhere means the
        // file next to that cwd.
        let cwd = match &args.cwd {
            Some(dir) => self.workspace.resolve(dir)?,
            None => self.workspace.project_root(),
        };
        let roots = self.workspace.roots();
        for arg in &args.args {
            let Some(candidate) = path_like(arg) else { continue };
            if let Err(refusal) = self.workspace.resolve_against(&cwd, candidate) {
                // `/dev/null`, `$TMPDIR` and the rest of the sandbox's
                // incidental list are not an escape: the sandbox already
                // treats them as writable under a *read*, and refusing them
                // as arguments broke `grep x file /dev/null` under any class.
                let incidental = crate::paths::resolved_form(&cwd, candidate)
                    .is_some_and(|resolved| sandbox::is_incidental(&resolved, &roots));
                if !incidental {
                    return Err(refusal);
                }
            }
        }

        execute(&self.workspace, &cwd, &args).await
    }
}

/// Runs the call, confining it when it was declared a read.
///
/// The [`ToolError::ReadRefused`] arm is the one that matters: it is not an
/// error so much as a question for the developer, and the dispatcher turns it
/// into one. Nothing landed when it is returned — that is the property the
/// sandbox exists to provide, and it is what makes re-running after a yes
/// safe rather than a gamble on how far the first attempt got.
async fn execute(workspace: &Workspace, cwd: &Path, args: &RunArgs) -> Result<String, ToolError> {
    let confine = args.class == Class::Read;

    // The sandbox is built first, because on some platforms confining a call
    // changes *what is spawned* rather than what happens after the fork.
    // Every root is read-only, not just the project root: a workspace whose
    // second root stayed writable under a read declaration would be a
    // read-only guarantee with a hole in exactly the place ADR 0007 widened.
    let plan = if confine {
        Some(sandbox::ReadOnly::build(&workspace.roots()).map_err(|source| ToolError::SandboxUnavailable {
            program: args.program.clone(),
            args:    args.args.clone(),
            source,
        })?)
    } else {
        None
    };

    // Linux hands back the program untouched and confines in the child;
    // macOS hands back `sandbox-exec -p <profile> -- <program>`. Asking for
    // the command line *before* stdio and `setsid` are configured is what
    // keeps a wrapping backend from having to rebuild — and silently
    // discard — settings that `Command` exposes no getter for.
    let (program, argv) = match &plan {
        Some(plan) => plan.command_line(&args.program, &args.args),
        None => (args.program.clone(), args.args.clone()),
    };

    let mut cmd = tokio::process::Command::new(&program);
    cmd.args(&argv)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    // Its own process group, so a timeout kills the whole tree rather than
    // just the process we happen to hold.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    if let Some(plan) = plan {
        for (key, value) in sandbox::read_only_env() {
            cmd.env(key, value);
        }
        // Installed after `setsid`: `pre_exec` hooks run in the order they
        // were added, and the confinement has to be the last thing the child
        // does before `exec`.
        plan.install(&mut cmd);
    }

    let mut child = cmd.spawn().map_err(|source| match source.kind() {
        std::io::ErrorKind::NotFound => ToolError::ProgramNotFound { program: args.program.clone() },
        _ => ToolError::Io { path: PathBuf::from(&args.program), source },
    })?;

    // `setsid` made the child its own group leader, so its pid is the group
    // id.
    let group = child.id();
    let mut stdout = child.stdout.take().expect("piped");
    let mut stderr = child.stderr.take().expect("piped");

    // Accumulate into buffers the reading future only borrows, so a timeout
    // can still read what arrived before it fired — which is precisely the
    // diagnostic a developer needs when a long command is what went wrong.
    //
    // Bytes, decoded once at the end: decoding each 8 KiB read on its own
    // turns any multibyte character that straddles a read boundary into
    // U+FFFD, in output the model is about to reason over.
    let out_buf = Mutex::new(Vec::<u8>::new());
    let err_buf = Mutex::new(Vec::<u8>::new());

    let run = async {
        let (a, b) = tokio::join!(drain(&mut stdout, &out_buf), drain(&mut stderr, &err_buf));
        a.and(b)?;
        child.wait().await
    };
    tokio::pin!(run);

    // Declared after `run` so it drops *first*: the group is killed while
    // the child is still unreaped, which is what keeps its pid — the group
    // id — from having been handed to something else.
    let mut group = KillGroupOnDrop(group);

    let status = match tokio::time::timeout(std::time::Duration::from_secs(args.timeout), &mut run).await {
        Ok(Ok(status)) => status,
        Ok(Err(source)) => return Err(ToolError::Io { path: cwd.to_path_buf(), source }),
        Err(_) => {
            drop(group);
            let out = take(&out_buf);
            let err = take(&err_buf);
            return Err(ToolError::Timeout {
                seconds: args.timeout,
                partial: render_partial(&out, &err),
            });
        }
    };
    // It exited on its own. Anything it deliberately left running stays.
    group.0 = None;

    let out = take(&out_buf);
    let err = take(&err_buf);

    // A read declaration only *fails* when the sandbox refused it — not
    // whenever the program exits non-zero. Conflating the two reported
    // `grep`'s "nothing matched" (exit 1, no stderr) as a refused read and
    // asked the developer to re-run a write that was never attempted, which
    // taught the model to stop declaring reads at all. A denial reaches the
    // child as an ordinary permission error, so it is the *evidence on
    // stderr* — or death by signal — that distinguishes them. A denial this
    // misses is bounded: it falls through as an ordinary failed command with
    // its own error in view, which the model can read and re-declare from.
    if confine && !status.success() && looks_like_denial(&err, &status) {
        return Err(ToolError::ReadRefused {
            program: args.program.clone(),
            args:    args.args.clone(),
        });
    }

    let mut output = render(&status, &out, &err);
    if confine && !status.success() {
        // Reached only when the failure carried no evidence of a denial. It
        // may still have been one — a program that swallows EACCES and exits
        // 1 looks exactly like this — and without saying so the model has no
        // way to know the call ran somewhere writing was impossible.
        output.push_str(
            "note: this call was declared a read, so it ran with the workspace read-only and the network \
             unreachable. If it needed to write or connect, that is why it failed — declare it \"write\".\n",
        );
    }
    if status.success() {
        Ok(output)
    } else {
        // The dispatcher's `finish` maps every `Ok` to `is_error: false`, so
        // returning the rendered output here would tell the model a command
        // that exited 2 had succeeded — which is exactly what it did, and the
        // model then guessed at a different program rather than fixing its
        // call. See `ToolError::CommandFailed`, which carries this same
        // string through the error arm instead.
        Err(ToolError::CommandFailed { output })
    }
}

/// Kills a call's whole process group unless the program exited by itself.
///
/// `kill_on_drop` reaches only the process we hold, so without this a
/// timed-out `sh -c 'slow-thing'` left `slow-thing` running — and so did a
/// call whose future was dropped because the developer cancelled the turn.
struct KillGroupOnDrop(Option<u32>);

impl Drop for KillGroupOnDrop {
    fn drop(&mut self) {
        if let Some(pgid) = self.0 {
            // SAFETY: a plain syscall on the id of a group we created.
            unsafe {
                libc::killpg(pgid as libc::pid_t, libc::SIGKILL);
            }
        }
    }
}

/// Reads a stream to EOF, appending as it goes so a cancelled read still
/// leaves everything received so far in the buffer. Past
/// [`OUTPUT_KEEP_BYTES`] it keeps reading and stops keeping: the pipe has to
/// stay drained or the program blocks on it.
async fn drain<R>(reader: &mut R, buf: &Mutex<Vec<u8>>) -> std::io::Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut chunk = [0u8; 8192];
    loop {
        let n = reader.read(&mut chunk).await?;
        if n == 0 {
            return Ok(());
        }
        if let Ok(mut guard) = buf.lock() {
            let room = OUTPUT_KEEP_BYTES.saturating_sub(guard.len());
            guard.extend_from_slice(&chunk[..n.min(room)]);
        }
    }
}

fn take(buf: &Mutex<Vec<u8>>) -> String {
    let bytes = buf.lock().map(|mut g| std::mem::take(&mut *g)).unwrap_or_default();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Whether a failed read-declared call looks like the sandbox stopping it,
/// rather than the program reporting an ordinary non-zero result.
///
/// Deliberately evidence-based and deliberately not a table of programs.
/// Death by signal counts: a process killed rather than exiting did not
/// choose its status.
fn looks_like_denial(stderr: &str, status: &std::process::ExitStatus) -> bool {
    if status.code().is_none() {
        return true;
    }
    let lowered = stderr.to_lowercase();
    ["permission denied", "read-only file system", "operation not permitted", "network is unreachable", "eacces", "eperm"]
        .iter()
        .any(|needle| lowered.contains(needle))
}

/// What a timed-out call reports: the elapsed budget *and* whatever the
/// program managed to say before it ran out.
fn render_partial(out: &str, err: &str) -> String {
    let mut body = String::new();
    if !out.is_empty() {
        body.push_str("partial stdout:\n");
        body.push_str(&cap(out));
    }
    if !err.is_empty() {
        body.push_str("partial stderr:\n");
        body.push_str(&cap(err));
    }
    if body.is_empty() {
        body.push_str("the program produced no output before the timeout");
    }
    body
}

fn render(status: &std::process::ExitStatus, out: &str, err: &str) -> String {
    let mut body = format!("exit: {}\n", status.code().map_or("signal".to_string(), |c| c.to_string()));
    if !out.is_empty() {
        body.push_str("stdout:\n");
        body.push_str(&cap(out));
    }
    if !err.is_empty() {
        body.push_str("stderr:\n");
        body.push_str(&cap(err));
    }
    body
}

fn cap(text: &str) -> String {
    if text.len() <= OUTPUT_CAP_BYTES {
        return text.to_string();
    }
    let mut end = OUTPUT_CAP_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n[truncated at {OUTPUT_CAP_BYTES} bytes]\n", &text[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::dispatch_context;

    /// Not `tempfile::tempdir()`: `/tmp` is on the sandbox's incidental-write
    /// list, so a project placed there would let a confined write succeed and
    /// the read-enforcement tests would pass without testing anything.
    fn tool() -> (tempfile::TempDir, RunTool) {
        let dir = tempfile::Builder::new()
            .prefix("aldwin-run-")
            .tempdir_in(env!("CARGO_MANIFEST_DIR"))
            .unwrap();
        let tool = RunTool::new(Workspace::new(dir.path()));
        (dir, tool)
    }

    async fn call(tool: &RunTool, input: Value) -> Result<String, ToolError> {
        let (ctx, _events, _pending) = dispatch_context();
        tool.call("c1", input, &ctx).await
    }

    /// The defect this pins, seen in a real transcript: the model sent
    /// `program: "ls", args: ["ls", "-la"]`, which ran `ls ls -la` and
    /// exited 2 — and the result came back `is_error: false`, so the model
    /// was told its broken call had worked. It then guessed a different
    /// program rather than fixing the arguments, costing a second approval.
    #[tokio::test]
    async fn a_non_zero_exit_is_an_error_and_keeps_the_whole_output() {
        let (_d, tool) = tool();
        let err = call(&tool, json!({"program": "ls", "args": ["no-such-entry"], "class": "write"}))
            .await
            .expect_err("a command that exits non-zero has not succeeded");

        let ToolError::CommandFailed { output } = &err else {
            panic!("expected CommandFailed, got {err:?}");
        };
        assert!(output.contains("exit: 2"), "the exit code survives: {output}");
        assert!(output.contains("stderr:"), "and so does stderr: {output}");
        assert_eq!(err.to_string(), *output, "Display carries the whole rendering, so nothing is lost");
    }

    /// The other half: a command that succeeds must stay a success, with no
    /// error flag and its stdout intact.
    #[tokio::test]
    async fn a_zero_exit_is_still_a_plain_success() {
        let (_d, tool) = tool();
        let out = call(&tool, json!({"program": "ls", "args": ["-a"], "class": "write"}))
            .await
            .expect("a command that exits 0 succeeded");
        assert!(out.starts_with("exit: 0"), "{out}");
    }

    /// Non-zero is reported as what it is, not interpreted. `grep` exits 1
    /// when it matched nothing, which is a result rather than a fault — the
    /// model is told the code and `run`'s description tells it to read it.
    #[tokio::test]
    async fn a_program_that_reports_by_exit_code_still_carries_its_code() {
        let (dir, tool) = tool();
        std::fs::write(dir.path().join("haystack.txt"), "alpha\n").unwrap();
        let err = call(&tool, json!({"program": "grep", "args": ["needle", "haystack.txt"], "class": "write"}))
            .await
            .expect_err("grep exits 1 on no match");
        assert!(err.to_string().contains("exit: 1"), "{err}");
    }

    /// The model supplied `args: ["ls", "-la"]` for `program: "ls"` because
    /// nothing in the schema said `args` excludes the program name — the
    /// `program` field had a description and `args` had none at all.
    #[test]
    fn the_args_schema_says_the_program_is_not_repeated() {
        let (_d, tool) = tool();
        let schema = &tool.descriptor().input_schema;
        let args = &schema["properties"]["args"];
        let description = args["description"].as_str().expect("args carries a description");
        assert!(
            description.contains("Do not repeat the program"),
            "the schema has to say it, not only the prose: {description}"
        );
    }

    #[test]
    fn the_permission_request_names_the_program_not_the_command_line() {
        let (_d, tool) = tool();
        let request = tool
            .permission(&json!({"program": "git", "args": ["status", "--short"], "class": "read"}))
            .unwrap();

        assert_eq!(request.program, "git");
        assert_eq!(request.class, Class::Read);
        assert_eq!(request.argv, vec!["status", "--short"]);
    }

    /// The reason `shell` could not carry a class: its grant was a glob over
    /// a whole command line, and a glob cannot stop at a metacharacter. Here
    /// there is no line to chain onto — the operator is just an argument.
    #[tokio::test]
    async fn shell_operators_are_arguments_not_syntax() {
        let (dir, tool) = tool();
        let out = call(
            &tool,
            json!({"program": "echo", "args": ["hello", "&&", "rm", "-rf", "everything"], "class": "read"}),
        )
        .await
        .unwrap();

        assert!(out.contains("hello && rm -rf everything"), "the operator must be inert text: {out}");
        assert!(dir.path().exists());
    }

    /// The same call with a real path outside the workspace does not even
    /// reach `execve` now — containment refuses it first (ADR 0007). This is
    /// the gap the tool had: `read`, `edit` and `explain` all checked their
    /// path arguments and `run`, alone, did not.
    #[tokio::test]
    async fn a_path_argument_outside_the_workspace_is_refused_before_the_program_runs() {
        let (_d, tool) = tool();
        let err = call(&tool, json!({"program": "echo", "args": ["/etc/passwd"], "class": "read"}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::PathEscapesWorkspace { .. }), "got {err:?}");
    }

    /// ...and the refusal names the roots, so the model can ask for one
    /// rather than quietly routing the work through a shell.
    #[tokio::test]
    async fn the_refusal_names_the_reachable_roots() {
        let (dir, tool) = tool();
        let err = call(&tool, json!({"program": "echo", "args": ["/etc/passwd"], "class": "read"}))
            .await
            .unwrap_err();
        let message = err.to_string();
        assert!(message.contains(&dir.path().canonicalize().unwrap().display().to_string()), "got {message}");
        assert!(message.contains("permissions.yaml"), "it must name the real mechanism: {message}");
        assert!(!message.contains("--root"), "there is no such flag: {message}");
    }

    /// A relative argument that climbs out is caught by the same check.
    #[tokio::test]
    async fn a_relative_argument_that_climbs_out_is_refused() {
        let (_d, tool) = tool();
        let err = call(&tool, json!({"program": "echo", "args": ["../../etc/passwd"], "class": "read"}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::PathEscapesWorkspace { .. }), "got {err:?}");
    }

    /// Globs and flags are not paths and must not be mistaken for them —
    /// containment that rejected `--include=*.kts` would be containment
    /// nobody could run a search under.
    #[test]
    fn patterns_and_flags_are_not_treated_as_paths() {
        assert_eq!(path_like("--include=*.kts"), None);
        assert_eq!(path_like("-la"), None);
        assert_eq!(path_like("*/node_modules/*"), None);
        assert_eq!(path_like("src/main.rs"), None, "relative and not climbing: the cwd already bounds it");
        assert_eq!(path_like("/etc/passwd"), Some("/etc/passwd"));
        assert_eq!(path_like("../../etc"), Some("../../etc"));
        assert_eq!(path_like("--path=/etc"), Some("/etc"), "the value of a flag is still a path");
        assert_eq!(path_like("-C/etc"), Some("/etc"), "so is a value glued to a short flag");
        assert_eq!(path_like("-f../../x"), Some("../../x"));
        assert_eq!(path_like("/no-such-top-level-dir/p"), None, "a sed or grep pattern, not a path");
        assert_eq!(path_like("of=/etc/passwd"), Some("/etc/passwd"), "`dd` spells its paths key=value");
        assert_eq!(path_like("name=value"), None);
    }

    /// Audit: taken whole, `of=../../x` reads as a directory called `of=..`
    /// followed by one climb, which normalizes to `x` inside the cwd — while
    /// `dd` writes two levels up.
    #[tokio::test]
    async fn a_climbing_value_behind_a_bare_key_is_contained_too() {
        let (_d, tool) = tool();
        let err = call(&tool, json!({"program": "echo", "args": ["of=../../../../escaped"], "class": "write"}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::PathEscapesWorkspace { .. }), "got {err:?}");
    }

    /// Audit: everything a program wrote was held until it exited and capped
    /// only when rendered, so `yes` could grow the buffer until the timeout.
    #[tokio::test]
    async fn output_past_the_cap_is_read_but_not_held() {
        let flood = vec![b'x'; 4 * OUTPUT_KEEP_BYTES];
        let buf = Mutex::new(Vec::new());
        drain(&mut flood.as_slice(), &buf).await.unwrap();

        assert_eq!(buf.lock().unwrap().len(), OUTPUT_KEEP_BYTES);
        assert!(cap(&take(&buf)).ends_with("bytes]\n"), "and the rendering still says it was cut");
    }

    /// Audit: only a timeout killed the group. A call dropped mid-run — the
    /// developer cancelling the turn — left its grandchildren running.
    #[tokio::test]
    async fn a_cancelled_call_kills_the_grandchildren_too() {
        let (dir, tool) = tool();
        let marker = dir.path().join("still-alive");
        let script = format!("(sleep 2; touch {}) & sleep 30", marker.display());
        let input = json!({"program": "/bin/sh", "args": ["-c", script], "class": "write", "timeout_secs": 30});
        let cancelled = tokio::time::timeout(std::time::Duration::from_secs(1), call(&tool, input)).await;
        assert!(cancelled.is_err(), "the call should still have been running");

        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        assert!(!marker.exists(), "the backgrounded grandchild outlived the cancelled call");
    }

    /// A second declared root is reachable, which is the half of ADR 0007
    /// that keeps the boundary usable rather than merely strict.
    #[tokio::test]
    async fn a_declared_second_root_is_reachable() {
        let project = tempfile::tempdir().unwrap();
        let sibling = tempfile::tempdir().unwrap();
        std::fs::write(sibling.path().join("notes.txt"), "hello from over here").unwrap();
        let tool = RunTool::new(Workspace::with_roots(project.path(), vec![sibling.path().to_path_buf()]));

        let target = sibling.path().join("notes.txt");
        let out = call(&tool, json!({"program": "/bin/cat", "args": [target.to_str().unwrap()], "class": "read"}))
            .await
            .unwrap();
        assert!(out.contains("hello from over here"), "got {out}");
    }

    /// `cwd` is the other half: the reason 46% of the observed session's
    /// calls were `bash -c 'cd … && …'` was that there was no way to say
    /// this.
    #[tokio::test]
    async fn a_call_runs_in_the_working_directory_it_names() {
        let (dir, tool) = tool();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/here.txt"), "found").unwrap();

        let out = call(&tool, json!({"program": "/bin/cat", "args": ["here.txt"], "class": "read", "cwd": "sub"}))
            .await
            .unwrap();
        assert!(out.contains("found"), "got {out}");
    }

    /// The defect that taught the model to stop declaring reads. `grep`
    /// exits 1 when nothing matched — a *result*, not a refusal — and the
    /// old code turned any non-zero exit under `class: read` into
    /// `ReadRefused`, which prompted the developer to allow a write that had
    /// never been attempted. `run`'s own description warns about this exit
    /// code two paragraphs above where it happened.
    #[tokio::test]
    async fn a_read_that_merely_exits_non_zero_is_not_a_refused_read() {
        let (dir, tool) = tool();
        std::fs::write(dir.path().join("haystack.txt"), "nothing of interest").unwrap();

        let err = call(&tool, json!({"program": "grep", "args": ["needle", "haystack.txt"], "class": "read"}))
            .await
            .unwrap_err();

        assert!(
            !matches!(err, ToolError::ReadRefused { .. }),
            "a clean no-match must not read as a sandbox refusal: {err:?}"
        );
        assert!(matches!(err, ToolError::CommandFailed { .. }), "got {err:?}");
    }

    #[test]
    fn a_denial_is_told_from_an_ordinary_failure_by_its_evidence() {
        use std::os::unix::process::ExitStatusExt;
        let exit_one = std::process::ExitStatus::from_raw(1 << 8);

        assert!(!looks_like_denial("", &exit_one), "grep's silent exit 1 is a result");
        assert!(!looks_like_denial("no such file or directory", &exit_one));
        assert!(looks_like_denial("mkdir: cannot create directory: Read-only file system", &exit_one));
        assert!(looks_like_denial("touch: /x: Permission denied", &exit_one));

        // Killed rather than exited: it did not choose its status.
        let killed = std::process::ExitStatus::from_raw(9);
        assert!(looks_like_denial("", &killed));
    }

    /// A timeout used to report only that it had timed out, dropping
    /// everything the program had already written — which is exactly the
    /// diagnostic a 30-minute command needs.
    #[tokio::test]
    async fn a_timeout_keeps_what_the_program_already_produced() {
        let (_d, tool) = tool();
        let err = call(
            &tool,
            json!({
                "program": "/bin/sh",
                "args": ["-c", "echo progress-so-far; sleep 30"],
                "class": "write",
                "timeout_secs": 1
            }),
        )
        .await
        .unwrap_err();

        let message = err.to_string();
        assert!(matches!(err, ToolError::Timeout { .. }), "got {err:?}");
        assert!(message.contains("timed out"), "got {message}");
        assert!(message.contains("progress-so-far"), "the partial output must survive: {message}");
    }

    /// Audit: the sandbox exempts `/dev/null` for writes while containment
    /// refused it as an argument, so this failed under either class.
    #[tokio::test]
    async fn an_incidental_path_is_a_legitimate_argument() {
        let (dir, tool) = tool();
        std::fs::write(dir.path().join("h.txt"), "needle\n").unwrap();
        let out = call(&tool, json!({"program": "grep", "args": ["needle", "h.txt", "/dev/null"], "class": "write"}))
            .await
            .unwrap();
        assert!(out.contains("needle"), "got {out}");
    }

    /// Audit: every `-…` token was skipped, so a path glued to a short flag
    /// reached `execve` unchecked.
    #[tokio::test]
    async fn a_path_glued_to_a_short_flag_is_contained_too() {
        let (_d, tool) = tool();
        let err = call(&tool, json!({"program": "grep", "args": ["-f/etc/hostname", "x"], "class": "write"}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::PathEscapesWorkspace { .. }), "got {err:?}");
    }

    /// Audit: output was lossy-decoded one 8 KiB read at a time, so a
    /// multibyte character straddling a read boundary became U+FFFD.
    #[tokio::test]
    async fn a_multibyte_character_across_a_read_boundary_survives() {
        let (dir, tool) = tool();
        let mut text = "a".repeat(8191);
        text.push('€');
        text.push_str("tail");
        std::fs::write(dir.path().join("u.txt"), &text).unwrap();

        let out = call(&tool, json!({"program": "/bin/cat", "args": ["u.txt"], "class": "write"})).await.unwrap();
        assert!(out.contains("€tail"), "the character must arrive whole");
        assert!(!out.contains('\u{FFFD}'));
    }

    /// Audit: a read-only run that fails *without* denial evidence came back
    /// as a bare failure, with nothing telling the model where it had run.
    #[tokio::test]
    async fn a_failed_read_says_it_ran_read_only() {
        let (dir, tool) = tool();
        std::fs::write(dir.path().join("h.txt"), "nothing").unwrap();
        let err = call(&tool, json!({"program": "grep", "args": ["needle", "h.txt"], "class": "read"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("declared a read"), "got {err}");
    }

    /// `setsid` exists so a timeout takes the whole tree, and until the
    /// audit only the direct child was killed — a timed-out `sh -c` left
    /// whatever it had started running.
    #[tokio::test]
    async fn a_timeout_kills_the_grandchildren_too() {
        let (dir, tool) = tool();
        let marker = dir.path().join("still-alive");
        let script = format!("(sleep 2; touch {}) & sleep 30", marker.display());
        let _ = call(&tool, json!({"program": "/bin/sh", "args": ["-c", script], "class": "write", "timeout_secs": 1}))
            .await;
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        assert!(!marker.exists(), "the backgrounded grandchild outlived the timeout");
    }

    #[tokio::test]
    async fn a_working_directory_outside_the_workspace_is_refused() {
        let (_d, tool) = tool();
        let err = call(&tool, json!({"program": "/bin/pwd", "class": "read", "cwd": "/etc"})).await.unwrap_err();
        assert!(matches!(err, ToolError::PathEscapesWorkspace { .. }), "got {err:?}");
    }

    #[tokio::test]
    async fn a_program_with_arguments_baked_into_it_is_refused() {
        let (_d, tool) = tool();
        let err = tool.permission(&json!({"program": "git status", "class": "read"})).unwrap_err();
        assert!(matches!(err, ToolError::InvalidInput { .. }));
    }

    /// The declaration is mandatory. A call that does not say what it does
    /// cannot be weighed against a grant that is written in those terms.
    #[tokio::test]
    async fn a_call_without_a_class_is_refused() {
        let (_d, tool) = tool();
        let err = tool.permission(&json!({"program": "ls"})).unwrap_err();
        assert!(matches!(err, ToolError::InvalidInput { .. }));
        let err = tool.permission(&json!({"program": "ls", "class": "edit"})).unwrap_err();
        assert!(matches!(err, ToolError::InvalidInput { .. }), "edit is not a class a call may declare");
    }

    #[tokio::test]
    async fn an_unknown_program_says_so_by_name() {
        let (_d, tool) = tool();
        let err = call(&tool, json!({"program": "no-such-program-anywhere", "class": "read"})).await.unwrap_err();
        assert!(matches!(err, ToolError::ProgramNotFound { .. }), "{err:?}");
    }

    /// The end-to-end shape of ADR 0004 §4, through the real tool: a call
    /// that declares itself a read and then tries to write comes back as a
    /// question, and the file it reached for is untouched.
    #[tokio::test]
    async fn a_read_declaration_that_writes_comes_back_as_a_question() {
        if !sandbox::availability().enforcing() {
            return;
        }
        let (dir, tool) = tool();
        let victim = dir.path().join("untouched.txt");
        std::fs::write(&victim, "original\n").unwrap();

        let err = call(&tool, json!({"program": "/usr/bin/rm", "args": ["untouched.txt"], "class": "read"}))
            .await
            .unwrap_err();

        assert!(matches!(err, ToolError::ReadRefused { .. }), "{err:?}");
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "original\n");
    }

    /// ...and the same call, declared honestly, is permitted to do it. The
    /// sandbox is not a second permission layer; it only holds a *read*
    /// declaration to its word.
    #[tokio::test]
    async fn the_same_call_declared_a_write_is_not_confined() {
        let (dir, tool) = tool();
        let victim = dir.path().join("doomed.txt");
        std::fs::write(&victim, "original\n").unwrap();

        call(&tool, json!({"program": "/usr/bin/rm", "args": ["doomed.txt"], "class": "write"})).await.unwrap();
        assert!(!victim.exists(), "a declared write is allowed to write");
    }

    #[tokio::test]
    async fn a_read_still_reads() {
        let (dir, tool) = tool();
        std::fs::write(dir.path().join("a.txt"), "contents\n").unwrap();

        let out = call(&tool, json!({"program": "/usr/bin/cat", "args": ["a.txt"], "class": "read"})).await.unwrap();
        assert!(out.contains("contents"), "{out}");
    }
}
