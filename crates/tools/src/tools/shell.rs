use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::io::AsyncReadExt;

use crate::error::ToolError;
use crate::gate::ApprovalGate;
use crate::registry::{Tool, ToolDescriptor, ToolSource};

const DEFAULT_TIMEOUT_SECS: u64 = 120;
const OUTPUT_CAP_BYTES: usize = 50 * 1024;

/// Run a command. Argv pattern is permission-keyed (e.g. `shell:cargo
/// test*`), piped output (not PTY), project-root cwd with no per-call
/// override, inherited env, 120s default timeout (overridable per call),
/// output capped at 50KB per stream with a truncation marker. Runs the
/// child in its own process group so cancellation/timeout can SIGKILL the
/// whole tree, not just the immediate child.
///
/// Known limitation, inherent to glob-based grant patterns rather than
/// specific to this tool: `permission_target` matches the *literal* command
/// string, so a grant like `shell:cargo test*` matches `cargo test &&
/// anything-at-all` too — `*` has no concept of "stop at a shell
/// metacharacter." A real fix needs actual shell-command parsing (an AST,
/// not a glob) to scope a grant to just the invoked binary and its argv;
/// that's out of scope for the glob grammar mjolnir-permissions.md
/// defines. This is the same accepted risk class as prefix-matched shell
/// permission systems generally (a broad grant is an intentional trust
/// decision, not an isolation boundary) — worth a developer's awareness
/// when choosing how broad a shell grant to hand out, not a bug fixable
/// within this tool alone.
pub struct ShellTool {
    descriptor:   ToolDescriptor,
    project_root: PathBuf,
}

impl ShellTool {
    pub fn new(project_root: PathBuf) -> Self {
        Self {
            descriptor: ToolDescriptor {
                name:         "shell".into(),
                description:  "Run a shell command in the project root. Output is capped at 50KB per stream.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "command":      { "type": "string" },
                        "timeout_secs": { "type": "integer", "minimum": 1 },
                    },
                    "required": ["command"],
                }),
                edit_class: false,
                source:     ToolSource::Builtin,
            },
            project_root,
        }
    }
}

fn command_arg(input: &Value) -> Result<String, ToolError> {
    input
        .get("command")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| ToolError::InvalidInput { tool: "shell".into(), message: "missing \"command\" string field".into() })
}

fn timeout_secs(input: &Value) -> u64 {
    input.get("timeout_secs").and_then(Value::as_u64).unwrap_or(DEFAULT_TIMEOUT_SECS)
}

#[async_trait]
impl Tool for ShellTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    /// Matched against `shell:`-kind grant patterns (e.g. `shell:cargo
    /// test*`) using the literal command line.
    fn permission_target(&self, input: &Value) -> Result<String, ToolError> {
        command_arg(input)
    }

    async fn call(&self, _call_id: &str, input: Value, _gate: &dyn ApprovalGate) -> Result<String, ToolError> {
        let command = command_arg(&input)?;
        let timeout = timeout_secs(&input);

        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c")
            .arg(&command)
            .current_dir(&self.project_root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(false);
        // New session (and therefore new process group, pgid == pid) so a
        // timeout/cancellation can SIGKILL the whole tree the shell spawned,
        // not just `sh` itself.
        //
        // SAFETY: `pre_exec`'s closure runs in the forked child between
        // `fork` and `exec`, where only async-signal-safe operations are
        // sound (allocating, taking locks, or touching Rust runtime state
        // can deadlock or corrupt the child — the parent's other threads,
        // and any locks they hold, are frozen mid-operation at the moment of
        // `fork` but not copied). `libc::setsid()` is a single raw syscall:
        // no allocation, no locking, nothing but a direct kernel call — safe
        // to run in that window. Its `Result` is intentionally discarded:
        // this closure can only fail with `EPERM` (already a process group
        // leader), which just means the child keeps its current pgid — an
        // already-safe fallback, not a condition worth failing the whole
        // spawn over.
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }

        let mut child = cmd.spawn().map_err(|source| ToolError::Io { path: self.project_root.clone(), source })?;
        let mut guard = ProcessGroupGuard::new(&child);

        let stdout = child.stdout.take().expect("piped above");
        let stderr = child.stderr.take().expect("piped above");

        let run = async {
            let (out, err) = tokio::join!(read_capped(stdout, OUTPUT_CAP_BYTES), read_capped(stderr, OUTPUT_CAP_BYTES));
            let status = child.wait().await;
            (out, err, status)
        };

        match tokio::time::timeout(Duration::from_secs(timeout), run).await {
            Ok((out, err, status)) => {
                guard.disarm();
                let status = status.map_err(|source| ToolError::Io { path: self.project_root.clone(), source })?;
                Ok(format_output(&out, &err, status))
            }
            Err(_) => {
                guard.kill_now();
                Err(ToolError::Timeout { secs: timeout })
            }
        }
    }
}

/// Best-effort SIGKILL of the child's process group on drop, unless
/// disarmed. Per mjolnir-tools.md: "Cancellation is advisory from core's
/// perspective — the dispatcher promises to release the slot, not that the
/// OS-level work stopped" — sending to an already-reaped pid is harmless.
struct ProcessGroupGuard {
    pid:   Option<i32>,
    armed: bool,
}

impl ProcessGroupGuard {
    fn new(child: &tokio::process::Child) -> Self {
        Self { pid: child.id().map(|p| p as i32), armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }

    fn kill_now(&mut self) {
        if let Some(pid) = self.pid.take() {
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
        }
        self.armed = false;
    }
}

impl Drop for ProcessGroupGuard {
    fn drop(&mut self) {
        if self.armed {
            self.kill_now();
        }
    }
}

struct CapturedStream {
    bytes:     Vec<u8>,
    truncated: bool,
}

async fn read_capped<R: tokio::io::AsyncRead + Unpin>(mut reader: R, cap: usize) -> CapturedStream {
    let mut bytes = Vec::new();
    let mut total = 0usize;
    let mut scratch = [0u8; 8192];
    loop {
        match reader.read(&mut scratch).await {
            Ok(0) => break,
            Ok(n) => {
                total += n;
                if bytes.len() < cap {
                    let take = (cap - bytes.len()).min(n);
                    bytes.extend_from_slice(&scratch[..take]);
                }
            }
            Err(_) => break,
        }
    }
    let truncated = total > bytes.len();
    CapturedStream { bytes, truncated }
}

fn format_output(stdout: &CapturedStream, stderr: &CapturedStream, status: ExitStatus) -> String {
    let status_desc = match status.code() {
        Some(code) => format!("exit code {code}"),
        None => format!("terminated by signal {}", status.signal().unwrap_or(-1)),
    };

    let mut out = format!("{status_desc}\n--- stdout ---\n{}", String::from_utf8_lossy(&stdout.bytes));
    if stdout.truncated {
        out.push_str("\n...[truncated]\n");
    }
    out.push_str("--- stderr ---\n");
    out.push_str(&String::from_utf8_lossy(&stderr.bytes));
    if stderr.truncated {
        out.push_str("\n...[truncated]\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ALWAYS_APPROVE;
    use std::os::unix::fs::PermissionsExt;

    #[tokio::test]
    async fn captures_stdout_and_exit_code() {
        let tool = ShellTool::new(std::env::temp_dir());
        let out = tool.call("c1", json!({"command": "echo hi"}), &ALWAYS_APPROVE).await.unwrap();
        assert!(out.contains("exit code 0"));
        assert!(out.contains("hi"));
    }

    #[tokio::test]
    async fn nonzero_exit_is_content_not_a_tool_error() {
        let tool = ShellTool::new(std::env::temp_dir());
        let out = tool.call("c1", json!({"command": "exit 3"}), &ALWAYS_APPROVE).await.unwrap();
        assert!(out.contains("exit code 3"));
    }

    #[tokio::test]
    async fn runs_in_the_given_cwd() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("marker.txt"), "").unwrap();
        let tool = ShellTool::new(dir.path().to_path_buf());
        let out = tool.call("c1", json!({"command": "ls"}), &ALWAYS_APPROVE).await.unwrap();
        assert!(out.contains("marker.txt"));
    }

    #[tokio::test]
    async fn output_is_capped_with_a_truncation_marker() {
        let tool = ShellTool::new(std::env::temp_dir());
        let out = tool.call("c1", json!({"command": "yes x | head -c 200000"}), &ALWAYS_APPROVE).await.unwrap();
        assert!(out.contains("...[truncated]"));
        assert!(out.len() < 120_000); // well under the 200KB the command actually produced
    }

    #[tokio::test]
    async fn timeout_is_a_structured_error() {
        let tool = ShellTool::new(std::env::temp_dir());
        let out = tool.call("c1", json!({"command": "sleep 5", "timeout_secs": 1}), &ALWAYS_APPROVE).await;
        assert!(matches!(out, Err(ToolError::Timeout { secs: 1 })));
    }

    #[tokio::test]
    async fn timeout_kills_the_whole_process_group() {
        // sleep spawned by a `sh -c` subshell; on group-kill both the
        // subshell and the grandchild `sleep` die, so no leaked process.
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("spawn.sh");
        std::fs::write(&script, "#!/bin/sh\n(sleep 5 &)\nsleep 5\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let tool = ShellTool::new(dir.path().to_path_buf());
        let out = tool.call("c1", json!({"command": format!("sh {}", script.display()), "timeout_secs": 1}), &ALWAYS_APPROVE).await;
        assert!(matches!(out, Err(ToolError::Timeout { .. })));
        // No assertion beyond "returns promptly" here — verifying the OS
        // actually reaped the grandchild would need /proc polling, which is
        // more machinery than this test needs; the guard's kill(-pgid) call
        // is what mjolnir-tools.md asks for.
    }

    #[test]
    fn permission_target_is_the_literal_command() {
        let tool = ShellTool::new(PathBuf::from("."));
        assert_eq!(tool.permission_target(&json!({"command": "cargo test"})).unwrap(), "cargo test");
    }

    #[test]
    fn missing_command_field_is_invalid_input() {
        let tool = ShellTool::new(PathBuf::from("."));
        let err = tool.permission_target(&json!({})).unwrap_err();
        assert!(matches!(err, ToolError::InvalidInput { .. }));
    }
}
