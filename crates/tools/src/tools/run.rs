//! `run` — a shell command, confined to the workspace for writes (ADR 0011).
//!
//! The command is one string, run as `sh -c`, so pipes, redirection, globs
//! and `&&` are what the model expects them to be. What contains it is not
//! a reading of that string — ADR 0004 tried classifying argv and ADR 0007
//! tried containing path-like arguments, and each left a hole the size of
//! `sh -c` — but the sandbox every process Aldwin spawns runs in: it may
//! read anything and write only inside the workspace roots and the
//! incidental paths (`crate::sandbox`).
//!
//! Two things `run` still decides for itself: its working directory is
//! resolved through the same [`Workspace`] as every other tool's path, and a
//! timeout kills the whole process group and keeps what the command had
//! already written.

use std::path::Path;
use std::process::Stdio;
use std::sync::Mutex;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::io::AsyncReadExt;

use crate::error::ToolError;
use crate::paths::Workspace;
use crate::registry::{Tool, ToolDescriptor};
use crate::sandbox;
use aldwin_core::DispatchContext;

const DEFAULT_TIMEOUT_SECS: u64 = 120;
const OUTPUT_CAP_BYTES: usize = 50 * 1024;
/// What is *held* per stream while the command runs. Only `OUTPUT_CAP_BYTES`
/// is ever shown, so holding more is memory a chatty command — `yes`, a build
/// log — can grow without bound. The slack keeps a character that straddles
/// the cap whole, and lets [`cap`] see that there was more.
const OUTPUT_KEEP_BYTES: usize = OUTPUT_CAP_BYTES + 4;

pub struct RunTool {
    descriptor: ToolDescriptor,
    workspace: Workspace,
}

/// One call's parsed input.
struct RunArgs {
    command: String,
    timeout: u64,
    /// Working directory as the model wrote it, before containment.
    cwd: Option<String>,
}

impl RunTool {
    pub fn new(workspace: Workspace) -> Self {
        Self {
            descriptor: ToolDescriptor {
                name: "run".into(),
                description: "Run a shell command (`sh -c`) in the project root, or in `cwd`. \
                              Pipes, redirection, globs and `&&` work as in any shell. \
                              The command can read anything, but it can write only inside the \
                              workspace — the roots listed in the session context — and to \
                              temporary files: a write anywhere else fails with a permission \
                              error. Do not try to route around that; say what you need written \
                              and where. \
                              A non-zero exit comes back as an error carrying the exit code and \
                              both streams; read the code before concluding anything broke, since \
                              some programs use it to report a result (`grep` exits 1 when nothing \
                              matched). \
                              Any edits you have staged this turn are reviewed by the developer \
                              before a run, since the run would see the files as they are on disk. \
                              `cwd` sets the working directory for this one call; pass it every \
                              time rather than expecting an earlier one to stick."
                    .into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "command":      { "type": "string", "description": "The shell command, e.g. \"cargo test 2>&1 | tail -20\"." },
                        "cwd":          { "type": "string",
                                          "description": "Working directory for this call. Defaults to the project root. \
                                                          Must be inside the workspace — the roots are listed in \
                                                          the session context, and a refusal names them." },
                        "timeout_secs": { "type": "integer", "minimum": 1 },
                    },
                    "required": ["command"],
                }),
                observes_disk: true,
            },
            workspace,
        }
    }
}

fn parse(input: &Value) -> Result<RunArgs, ToolError> {
    let command = input
        .get("command")
        .and_then(Value::as_str)
        .filter(|c| !c.trim().is_empty())
        .ok_or_else(|| ToolError::InvalidInput {
            tool: "run".into(),
            message: "missing \"command\" string field".into(),
        })?
        .to_string();
    let timeout = input
        .get("timeout_secs")
        .and_then(Value::as_u64)
        .filter(|s| *s > 0)
        .unwrap_or(DEFAULT_TIMEOUT_SECS);
    let cwd = input
        .get("cwd")
        .and_then(Value::as_str)
        .filter(|c| !c.is_empty())
        .map(str::to_string);
    Ok(RunArgs {
        command,
        timeout,
        cwd,
    })
}

#[async_trait]
impl Tool for RunTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn call(
        &self,
        _call_id: &str,
        input: Value,
        _ctx: &DispatchContext,
    ) -> Result<String, ToolError> {
        let args = parse(&input)?;
        let cwd = match &args.cwd {
            Some(dir) => self.workspace.resolve(dir)?,
            None => self.workspace.project_root(),
        };
        execute(&self.workspace, &cwd, &args).await
    }
}

async fn execute(workspace: &Workspace, cwd: &Path, args: &RunArgs) -> Result<String, ToolError> {
    let shell_args = ["-c".to_string(), args.command.clone()];
    let mut cmd = sandbox::command("/bin/sh", &shell_args, &workspace.roots())
        .map_err(|source| ToolError::Sandbox { source })?;
    cmd.current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    // Its own process group, so a timeout kills the whole tree rather than
    // just the shell we happen to hold.
    // SAFETY: `setsid` is async-signal-safe and allocates nothing.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let mut child = cmd.spawn().map_err(|source| ToolError::Io {
        path: cwd.to_path_buf(),
        source,
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

    let status =
        match tokio::time::timeout(std::time::Duration::from_secs(args.timeout), &mut run).await {
            Ok(Ok(status)) => status,
            Ok(Err(source)) => {
                return Err(ToolError::Io {
                    path: cwd.to_path_buf(),
                    source,
                })
            }
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

    let output = render(&status, &take(&out_buf), &take(&err_buf));
    if status.success() {
        Ok(output)
    } else {
        // The dispatcher's `finish` maps every `Ok` to `is_error: false`, so
        // returning the rendered output here would tell the model a command
        // that exited 2 had succeeded. See `ToolError::CommandFailed`, which
        // carries this same string through the error arm instead.
        Err(ToolError::CommandFailed { output })
    }
}

/// Kills a call's whole process group unless the command exited by itself.
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
/// stay drained or the command blocks on it.
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
    let bytes = buf
        .lock()
        .map(|mut g| std::mem::take(&mut *g))
        .unwrap_or_default();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// What a timed-out call reports: the elapsed budget *and* whatever the
/// command managed to say before it ran out.
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
        body.push_str("the command produced no output before the timeout");
    }
    body
}

fn render(status: &std::process::ExitStatus, out: &str, err: &str) -> String {
    let mut body = format!(
        "exit: {}\n",
        status
            .code()
            .map_or("signal".to_string(), |c| c.to_string())
    );
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
    format!(
        "{}\n[truncated at {OUTPUT_CAP_BYTES} bytes]\n",
        &text[..end]
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{confinement_or_explicit_skip, dispatch_context, scratch_dir};

    fn tool() -> (tempfile::TempDir, RunTool) {
        let dir = scratch_dir();
        let tool = RunTool::new(Workspace::new(dir.path()));
        (dir, tool)
    }

    async fn call(tool: &RunTool, input: Value) -> Result<String, ToolError> {
        let (ctx, _events, _pending) = dispatch_context();
        tool.call("c1", input, &ctx).await
    }

    /// The whole of ADR 0011 in one assertion: a command may say it writes
    /// anywhere, and what it can actually write is the workspace.
    #[tokio::test]
    async fn a_run_writing_outside_the_workspace_fails_and_changes_nothing() {
        if !confinement_or_explicit_skip() {
            return;
        }
        let (_d, tool) = tool();
        let outside = scratch_dir();
        let victim = outside.path().join("victim.txt");
        std::fs::write(&victim, "original\n").unwrap();

        let err = call(
            &tool,
            json!({"command": format!("echo changed > {}", victim.display())}),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err, ToolError::CommandFailed { .. }),
            "got {err:?}"
        );
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "original\n");
    }

    #[tokio::test]
    async fn a_run_writing_inside_the_workspace_succeeds() {
        let (dir, tool) = tool();
        call(
            &tool,
            json!({"command": "mkdir -p out && printf built > out/artifact"}),
        )
        .await
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("out/artifact")).unwrap(),
            "built"
        );
    }

    /// A second declared root is workspace on the same terms as the first —
    /// reachable as a `cwd`, and writable.
    #[tokio::test]
    async fn a_declared_second_root_is_reachable_and_writable() {
        let project = scratch_dir();
        let sibling = scratch_dir();
        let tool = RunTool::new(Workspace::with_roots(
            project.path(),
            vec![sibling.path().to_path_buf()],
        ));
        call(
            &tool,
            json!({"command": "printf here > notes.txt", "cwd": sibling.path().to_str().unwrap()}),
        )
        .await
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(sibling.path().join("notes.txt")).unwrap(),
            "here"
        );
    }

    #[tokio::test]
    async fn a_shell_pipeline_works() {
        let (dir, tool) = tool();
        std::fs::write(dir.path().join("words.txt"), "b\na\nc\na\n").unwrap();
        let out = call(
            &tool,
            json!({"command": "sort words.txt | uniq -c | grep ' a' && echo done"}),
        )
        .await
        .unwrap();
        assert!(out.contains("2 a"), "{out}");
        assert!(out.contains("done"), "{out}");
    }

    #[tokio::test]
    async fn a_call_runs_in_the_working_directory_it_names() {
        let (dir, tool) = tool();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/here.txt"), "found").unwrap();
        let out = call(&tool, json!({"command": "cat here.txt", "cwd": "sub"}))
            .await
            .unwrap();
        assert!(out.contains("found"), "got {out}");
    }

    #[tokio::test]
    async fn a_working_directory_outside_the_workspace_is_refused() {
        let (_d, tool) = tool();
        for cwd in ["/etc", "../.."] {
            let err = call(&tool, json!({"command": "pwd", "cwd": cwd}))
                .await
                .unwrap_err();
            assert!(
                matches!(err, ToolError::PathEscapesWorkspace { .. }),
                "{cwd}: got {err:?}"
            );
        }
    }

    /// A symlink is an ordinary git blob, so a checkout can ship `out ->
    /// /elsewhere`; naming it as a `cwd` must not start the command there.
    #[tokio::test]
    async fn a_symlinked_working_directory_that_escapes_is_refused() {
        let (dir, tool) = tool();
        let outside = scratch_dir();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("out")).unwrap();
        let err = call(&tool, json!({"command": "pwd", "cwd": "out"}))
            .await
            .unwrap_err();
        assert!(
            matches!(err, ToolError::PathEscapesWorkspace { .. }),
            "got {err:?}"
        );
    }

    /// The refusal names the roots, so the model can ask for one rather than
    /// guessing at a path it can reach.
    #[tokio::test]
    async fn the_refusal_names_the_reachable_roots() {
        let (dir, tool) = tool();
        let message = call(&tool, json!({"command": "pwd", "cwd": "/etc"}))
            .await
            .unwrap_err()
            .to_string();
        assert!(
            message.contains(&dir.path().canonicalize().unwrap().display().to_string()),
            "got {message}"
        );
        assert!(message.contains("permissions.yaml"), "{message}");
    }

    #[tokio::test]
    async fn a_call_without_a_command_is_refused() {
        let (_d, tool) = tool();
        for input in [json!({}), json!({"command": "  "}), json!({"command": 3})] {
            let err = call(&tool, input).await.unwrap_err();
            assert!(matches!(err, ToolError::InvalidInput { .. }), "{err:?}");
        }
    }

    /// Seen in a real transcript: a failed command came back `is_error:
    /// false`, and the model was told its broken call had worked.
    #[tokio::test]
    async fn a_non_zero_exit_is_an_error_and_keeps_the_whole_output() {
        let (_d, tool) = tool();
        let err = call(&tool, json!({"command": "ls no-such-entry"}))
            .await
            .expect_err("a command that exits non-zero has not succeeded");
        let ToolError::CommandFailed { output } = &err else {
            panic!("expected CommandFailed, got {err:?}");
        };
        assert!(
            output.contains("exit: 2"),
            "the exit code survives: {output}"
        );
        assert!(output.contains("stderr:"), "and so does stderr: {output}");
        assert_eq!(err.to_string(), *output);
    }

    #[tokio::test]
    async fn a_zero_exit_is_still_a_plain_success() {
        let (_d, tool) = tool();
        let out = call(&tool, json!({"command": "ls -a"})).await.unwrap();
        assert!(out.starts_with("exit: 0"), "{out}");
    }

    /// A timeout used to report only that it had timed out, dropping
    /// everything the command had already written.
    #[tokio::test]
    async fn a_timeout_keeps_what_the_command_already_produced() {
        let (_d, tool) = tool();
        let err = call(
            &tool,
            json!({"command": "echo progress-so-far; sleep 30", "timeout_secs": 1}),
        )
        .await
        .unwrap_err();
        let message = err.to_string();
        assert!(matches!(err, ToolError::Timeout { .. }), "got {err:?}");
        assert!(message.contains("progress-so-far"), "{message}");
    }

    /// `setsid` exists so a timeout takes the whole tree: a timed-out
    /// `sh -c` used to leave whatever it had started running.
    #[tokio::test]
    async fn a_timeout_kills_the_grandchildren_too() {
        let (dir, tool) = tool();
        let marker = dir.path().join("still-alive");
        let command = format!("(sleep 2; touch {}) & sleep 30", marker.display());
        let _ = call(&tool, json!({"command": command, "timeout_secs": 1})).await;
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        assert!(
            !marker.exists(),
            "the backgrounded grandchild outlived the timeout"
        );
    }

    /// Only a timeout killed the group once. A call dropped mid-run — the
    /// developer cancelling the turn — left its grandchildren running.
    #[tokio::test]
    async fn a_cancelled_call_kills_the_grandchildren_too() {
        let (dir, tool) = tool();
        let marker = dir.path().join("still-alive");
        let command = format!("(sleep 2; touch {}) & sleep 30", marker.display());
        let input = json!({"command": command, "timeout_secs": 30});
        let cancelled =
            tokio::time::timeout(std::time::Duration::from_secs(1), call(&tool, input)).await;
        assert!(
            cancelled.is_err(),
            "the call should still have been running"
        );

        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        assert!(
            !marker.exists(),
            "the backgrounded grandchild outlived the cancelled call"
        );
    }

    #[tokio::test]
    async fn output_past_the_cap_is_read_but_not_held() {
        let flood = vec![b'x'; 4 * OUTPUT_KEEP_BYTES];
        let buf = Mutex::new(Vec::new());
        drain(&mut flood.as_slice(), &buf).await.unwrap();
        assert_eq!(buf.lock().unwrap().len(), OUTPUT_KEEP_BYTES);
        assert!(cap(&take(&buf)).ends_with("bytes]\n"));
    }

    /// Output was lossy-decoded one 8 KiB read at a time once, so a
    /// multibyte character straddling a read boundary became U+FFFD.
    #[tokio::test]
    async fn a_multibyte_character_across_a_read_boundary_survives() {
        let (dir, tool) = tool();
        let mut text = "a".repeat(8191);
        text.push('€');
        text.push_str("tail");
        std::fs::write(dir.path().join("u.txt"), &text).unwrap();
        let out = call(&tool, json!({"command": "cat u.txt"})).await.unwrap();
        assert!(out.contains("€tail"), "the character must arrive whole");
        assert!(!out.contains('\u{FFFD}'));
    }
}
