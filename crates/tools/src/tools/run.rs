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

use std::path::PathBuf;
use std::process::Stdio;

use async_trait::async_trait;
use mjolnir_permissions::Class;
use serde_json::{json, Value};
use tokio::io::AsyncReadExt;

use crate::error::ToolError;
use crate::gate::ApprovalGate;
use crate::registry::{PermissionRequest, Tool, ToolDescriptor, ToolSource};
use crate::sandbox;

const DEFAULT_TIMEOUT_SECS: u64 = 120;
const OUTPUT_CAP_BYTES: usize = 50 * 1024;

pub struct RunTool {
    descriptor:   ToolDescriptor,
    project_root: PathBuf,
}

/// One call's parsed input.
struct RunArgs {
    program: String,
    args:    Vec<String>,
    class:   Class,
    timeout: u64,
}

impl RunTool {
    pub fn new(project_root: PathBuf) -> Self {
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
                              fails rather than causing damage."
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
                    },
                    "required": ["program", "class"],
                }),
                edit_class: false,
                source:     ToolSource::Builtin,
            },
            project_root,
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

    Ok(RunArgs { program, args, class, timeout })
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
        execute(&self.project_root, &args).await
    }
}

/// Runs the call, confining it when it was declared a read.
///
/// The [`ToolError::ReadRefused`] arm is the one that matters: it is not an
/// error so much as a question for the developer, and the dispatcher turns it
/// into one. Nothing landed when it is returned — that is the property the
/// sandbox exists to provide, and it is what makes re-running after a yes
/// safe rather than a gamble on how far the first attempt got.
async fn execute(project_root: &PathBuf, args: &RunArgs) -> Result<String, ToolError> {
    let confine = args.class == Class::Read;

    let mut cmd = tokio::process::Command::new(&args.program);
    cmd.args(&args.args)
        .current_dir(project_root)
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

    if confine {
        for (key, value) in sandbox::read_only_env() {
            cmd.env(key, value);
        }
        let plan = sandbox::ReadOnly::build(project_root)
            .map_err(|source| ToolError::SandboxUnavailable { source })?;
        // SAFETY: `engage` is two syscalls with no allocation, which is what
        // `pre_exec` permits — see its own safety note.
        unsafe {
            cmd.pre_exec(move || plan.engage());
        }
    }

    let mut child = cmd.spawn().map_err(|source| match source.kind() {
        std::io::ErrorKind::NotFound => ToolError::ProgramNotFound { program: args.program.clone() },
        _ => ToolError::Io { path: PathBuf::from(&args.program), source },
    })?;

    let mut stdout = child.stdout.take().expect("piped");
    let mut stderr = child.stderr.take().expect("piped");

    let run = async {
        let mut out = String::new();
        let mut err = String::new();
        let (a, b) = tokio::join!(stdout.read_to_string(&mut out), stderr.read_to_string(&mut err));
        a.and(b)?;
        let status = child.wait().await?;
        Ok::<_, std::io::Error>((status, out, err))
    };

    let (status, out, err) = match tokio::time::timeout(std::time::Duration::from_secs(args.timeout), run).await {
        Ok(Ok(triple)) => triple,
        Ok(Err(source)) => return Err(ToolError::Io { path: project_root.clone(), source }),
        Err(_) => return Err(ToolError::Timeout { seconds: args.timeout }),
    };

    if confine && !status.success() {
        return Err(ToolError::ReadRefused {
            program: args.program.clone(),
            args:    args.args.clone(),
        });
    }

    let output = render(&status, &out, &err);
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

fn render(status: &std::process::ExitStatus, out: &str, err: &str) -> String {
    let mut body = String::new();
    body.push_str(&format!("exit: {}\n", status.code().map_or("signal".to_string(), |c| c.to_string())));
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
            .prefix("mjolnir-run-")
            .tempdir_in(env!("CARGO_MANIFEST_DIR"))
            .unwrap();
        let tool = RunTool::new(dir.path().to_path_buf());
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
            json!({"program": "/usr/bin/echo", "args": ["hello", "&&", "rm", "-rf", "/"], "class": "read"}),
        )
        .await
        .unwrap();

        assert!(out.contains("hello && rm -rf /"), "the operator must be inert text: {out}");
        assert!(dir.path().exists());
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
