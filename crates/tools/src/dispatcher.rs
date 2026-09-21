use std::sync::Arc;

use aldwin_core::{DispatchContext, ToolCall, ToolDefinition, ToolResult};
use aldwin_permissions::{Class, Engine, Outcome, PromptPayload, PromptResponse};
use async_trait::async_trait;

use crate::error::ToolError;
use crate::registry::{PermissionRequest, Registry};

/// Implements core's `ToolDispatcher`. Owns the dispatch flow of ADR 0004:
/// resolve name -> tool, ask the tool what it wants to do, weigh that against
/// the permission engine, run it, and — if a read declaration turned out to
/// be wrong — come back to the developer with the one question that is worth
/// asking at that point.
///
/// `edit_class: true` tools skip the permission path entirely: editing is
/// outside the model, and its approval gate lives inside the tool's own
/// future (see `gate.rs`).
pub struct Dispatcher {
    registry:    Registry,
    permissions: Arc<Engine>,
    /// Serialises the prompt-and-record half of `check` across the tool
    /// calls of a step, which `aldwin-core`'s `dispatch_tools` drives
    /// concurrently (`future::join_all`) — see `check`'s own doc comment for
    /// the bug that makes this necessary. Held only while a prompt is
    /// genuinely outstanding, so calls the engine can already answer never
    /// touch it.
    prompt_gate: tokio::sync::Mutex<()>,
}

/// Whether a call may proceed, and how.
enum Verdict {
    Run,
    Refused(ToolError),
}

impl Dispatcher {
    pub fn new(registry: Registry, permissions: Arc<Engine>) -> Self {
        Self { registry, permissions, prompt_gate: tokio::sync::Mutex::new(()) }
    }

    /// Weighs one call against the engine, prompting if it has to.
    ///
    /// The check happens twice on the prompt path, either side of
    /// `prompt_gate`, and that is the whole point: a step's tool calls are
    /// dispatched concurrently, so with one shared check every call in the
    /// step reached the engine before the developer had answered anything,
    /// and each one independently got "ask" back. Answering the first prompt
    /// with a grant that plainly covered the rest changed nothing for them,
    /// because their outcome was already decided — the developer was asked
    /// again for every queued call the answer had just covered. That is the
    /// reported "permissions don't appear to count properly when commands are
    /// queued".
    ///
    /// Taking the gate before prompting makes the queued calls wait, and
    /// re-checking after acquiring it is what lets the grant the developer
    /// just made actually apply.
    async fn check(&self, request: &PermissionRequest, call_id: &str, ctx: &DispatchContext) -> Verdict {
        match self.permissions.check(&request.program, request.class, &request.argv) {
            Outcome::Allow => return Verdict::Run,
            Outcome::Locked { scope, .. } => {
                return Verdict::Refused(ToolError::Locked {
                    program:        request.program.clone(),
                    where_it_lives: scope.where_it_lives(),
                })
            }
            Outcome::Ask(_) => {}
        }

        let _gate = self.prompt_gate.lock().await;
        let payload = match self.permissions.check(&request.program, request.class, &request.argv) {
            Outcome::Allow => return Verdict::Run,
            Outcome::Locked { scope, .. } => {
                return Verdict::Refused(ToolError::Locked {
                    program:        request.program.clone(),
                    where_it_lives: scope.where_it_lives(),
                })
            }
            Outcome::Ask(payload) => payload,
        };

        self.ask(payload, request.program.clone(), request.class, call_id, ctx).await
    }

    /// Draws one prompt and records the answer. `class` is what the answer
    /// gets written against — the class of the call that raised it, which is
    /// what the eight rows qualify themselves by.
    async fn ask(
        &self,
        payload: PromptPayload,
        program: String,
        class:   Class,
        call_id: &str,
        ctx:     &DispatchContext,
    ) -> Verdict {
        let value = serde_json::to_value(&payload).expect("PromptPayload always serialises");
        let response_value = ctx.request_prompt(call_id.to_string(), value).await;

        let choice = match serde_json::from_value::<PromptResponse>(response_value) {
            Ok(PromptResponse::Tool { choice } | PromptResponse::WriteAttempt { choice }) => choice,
            // The tool path never raises an Edit or ContextFile prompt, so a
            // well-behaved caller cannot produce this; a malformed answer
            // still must not panic.
            Ok(PromptResponse::ContextFile { .. }) | Err(_) => {
                return Verdict::Refused(ToolError::MalformedPromptResponse)
            }
        };

        if let Err(e) = self.permissions.record(&program, class, choice) {
            return Verdict::Refused(e.into());
        }
        if choice.is_allow() {
            Verdict::Run
        } else {
            Verdict::Refused(ToolError::Denied)
        }
    }

    /// The second half of ADR 0004 §4. A call declared a read has come back
    /// refused, which means the sandbox stopped it and **nothing landed**.
    /// The developer is asked whether to allow it as a write; a yes re-runs
    /// it unconfined, which is safe precisely because the first attempt could
    /// not have half-finished.
    async fn offer_as_write(
        &self,
        program: &str,
        args:    &[String],
        call:    &ToolCall,
        ctx:     &DispatchContext,
    ) -> Option<ToolResult> {
        let payload = PromptPayload::WriteAttempt {
            program: program.to_string(),
            argv:    args.to_vec(),
        };
        match self.ask(payload, program.to_string(), Class::Write, &call.id, ctx).await {
            Verdict::Run => None,
            Verdict::Refused(e) => Some(error_result(&call.id, e)),
        }
    }
}

#[async_trait]
impl aldwin_core::ToolDispatcher for Dispatcher {
    async fn dispatch(&self, call: ToolCall, ctx: &DispatchContext) -> ToolResult {
        let Some(tool) = self.registry.get(&call.name) else {
            return error_result(&call.id, ToolError::UnknownTool { name: call.name });
        };
        let descriptor = tool.descriptor().clone();

        if descriptor.edit_class {
            return finish(&call.id, tool.call(&call.id, call.input, ctx).await);
        }

        let request = match tool.permission(&call.input) {
            Ok(request) => request,
            Err(e) => return error_result(&call.id, e),
        };
        if let Verdict::Refused(e) = self.check(&request, &call.id, ctx).await {
            return error_result(&call.id, e);
        }

        match tool.call(&call.id, call.input.clone(), ctx).await {
            // A read declaration that did not survive contact with the
            // sandbox. Ask, and on a yes run it again as the write it was.
            Err(ToolError::ReadRefused { program, args }) => {
                if let Some(refusal) = self.offer_as_write(&program, &args, &call, ctx).await {
                    return refusal;
                }
                let input = as_write(call.input);
                finish(&call.id, tool.call(&call.id, input, ctx).await)
            }
            // There is no enforcement primitive on this platform, so the read
            // declaration cannot be honoured — which ADR 0004 §4 answers with
            // "every call asks", not with an error. It *was* an error until
            // ADR 0007, and the cost was not one failed call: the model saw
            // two reads fail, concluded the read path was broken, and spent
            // the remaining 69 calls of that session declaring `ls`, `grep`
            // and `cat` as writes. A class nobody can use safely is a class
            // nobody uses.
            //
            // Same question as above, same guarantee behind it — nothing ran,
            // because the sandbox refused to be built rather than refusing
            // mid-call.
            //
            // Unlike a refused read — a rare event, worth a question every
            // time — this happens on *every* read-declared call where there
            // is no sandbox. So the engine is consulted first, under the
            // prompt gate: once the developer has allowed this program's
            // writes at any tier, the answer stands and nothing is asked
            // again. Asking unconditionally made "always allow" a no-op and
            // would have taught the model, again, to stop declaring reads.
            Err(ToolError::SandboxUnavailable { program, args, .. }) => {
                let _gate = self.prompt_gate.lock().await;
                match self.permissions.check(&program, Class::Write, &args) {
                    Outcome::Allow => {}
                    Outcome::Locked { scope, .. } => {
                        return error_result(
                            &call.id,
                            ToolError::Locked { program: program.clone(), where_it_lives: scope.where_it_lives() },
                        );
                    }
                    Outcome::Ask(_) => {
                        if let Some(refusal) = self.offer_as_write(&program, &args, &call, ctx).await {
                            return refusal;
                        }
                    }
                }
                drop(_gate);
                let input = as_write(call.input);
                finish(&call.id, tool.call(&call.id, input, ctx).await)
            }
            other => finish(&call.id, other),
        }
    }

    fn definitions(&self) -> Vec<ToolDefinition> {
        self.registry.definitions()
    }
}

/// Re-declares a call as a write, for the re-run after the developer allowed
/// it. The tool re-parses its own input, so the class has to change in the
/// input rather than beside it.
fn as_write(mut input: serde_json::Value) -> serde_json::Value {
    if let Some(map) = input.as_object_mut() {
        map.insert("class".to_string(), serde_json::Value::String("write".into()));
    }
    input
}

fn finish(call_id: &str, outcome: Result<String, ToolError>) -> ToolResult {
    match outcome {
        Ok(content) => ToolResult { call_id: call_id.to_string(), content, is_error: false },
        Err(e) => error_result(call_id, e),
    }
}

fn error_result(call_id: &str, err: ToolError) -> ToolResult {
    ToolResult { call_id: call_id.to_string(), content: err.to_string(), is_error: true }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Tool, ToolDescriptor, ToolSource};
    use crate::test_support::dispatch_context;
    use aldwin_config::Config;
    use aldwin_core::{Event, ToolDispatcher as _};
    use aldwin_permissions::{Choice, GrantEntry, GrantList};
    use async_trait::async_trait;
    use serde_json::{json, Value};

    /// A stand-in for `run`: it takes a program and a declared class the same
    /// way, and can be told to fail its first read-declared attempt, which is
    /// what the sandbox does to a call that tried to write.
    struct FakeRun {
        descriptor:   ToolDescriptor,
        /// Number of `call`s that have happened, so a test can see the re-run.
        calls:        std::sync::atomic::AtomicUsize,
        /// When set, a `read`-declared call comes back as `ReadRefused`.
        refuses_read: bool,
        /// When set, a `read`-declared call comes back as
        /// `SandboxUnavailable` — what `run` returns on a platform with no
        /// enforcement primitive.
        no_sandbox:   bool,
    }

    #[async_trait]
    impl Tool for FakeRun {
        fn descriptor(&self) -> &ToolDescriptor {
            &self.descriptor
        }

        fn permission(&self, input: &Value) -> Result<PermissionRequest, ToolError> {
            Ok(PermissionRequest {
                program: input.get("program").and_then(Value::as_str).unwrap_or_default().to_string(),
                class:   match input.get("class").and_then(Value::as_str) {
                    Some("write") => Class::Write,
                    _ => Class::Read,
                },
                argv: Vec::new(),
            })
        }

        async fn call(&self, _id: &str, input: Value, _gate: &dyn crate::gate::ApprovalGate) -> Result<String, ToolError> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let declared = input.get("class").and_then(Value::as_str).unwrap_or("read");
            if self.no_sandbox && declared == "read" {
                return Err(ToolError::SandboxUnavailable {
                    program: input.get("program").and_then(Value::as_str).unwrap_or_default().to_string(),
                    args:    Vec::new(),
                    source:  std::io::Error::new(std::io::ErrorKind::Unsupported, "no enforcement here"),
                });
            }
            if self.refuses_read && declared == "read" {
                return Err(ToolError::ReadRefused {
                    program: input.get("program").and_then(Value::as_str).unwrap_or_default().to_string(),
                    args:    Vec::new(),
                });
            }
            Ok(format!("ran as {declared}"))
        }
    }

    fn fake_run(name: &str, edit_class: bool, refuses_read: bool) -> Arc<FakeRun> {
        Arc::new(FakeRun {
            descriptor:   ToolDescriptor {
                name:         name.into(),
                description:  "fake".into(),
                input_schema: json!({}),
                edit_class,
                source:       ToolSource::Builtin,
            },
            calls:        std::sync::atomic::AtomicUsize::new(0),
            refuses_read,
            no_sandbox:   false,
        })
    }

    fn fake_run_without_a_sandbox(name: &str) -> Arc<FakeRun> {
        Arc::new(FakeRun {
            descriptor:   ToolDescriptor {
                name:         name.into(),
                description:  "fake".into(),
                input_schema: json!({}),
                edit_class:   false,
                source:       ToolSource::Builtin,
            },
            calls:        std::sync::atomic::AtomicUsize::new(0),
            refuses_read: false,
            no_sandbox:   true,
        })
    }

    fn engine() -> (tempfile::TempDir, Arc<Engine>) {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::open_at(dir.path(), dir.path().join("global")).unwrap();
        (dir, Arc::new(Engine::new(config)))
    }

    fn call_of(program: &str, class: &str) -> ToolCall {
        ToolCall { id: "c1".into(), name: "run".into(), input: json!({"program": program, "class": class}) }
    }

    /// Answers the next prompt with `choice`, asserting on the payload.
    async fn answer(
        events:  &mut tokio::sync::mpsc::Receiver<Event>,
        pending: &aldwin_core::PendingMap,
        choice:  Choice,
    ) -> PromptPayload {
        let Some(Event::PromptRequested { call_id, payload }) = events.recv().await else {
            panic!("expected a prompt");
        };
        let payload: PromptPayload = serde_json::from_value(payload).unwrap();
        let response = match payload {
            PromptPayload::WriteAttempt { .. } => PromptResponse::WriteAttempt { choice },
            _ => PromptResponse::Tool { choice },
        };
        let Some(aldwin_core::PendingReply::Prompt(tx)) = pending.lock().unwrap().remove(&call_id) else {
            panic!("expected a pending Prompt entry for {call_id}");
        };
        tx.send(serde_json::to_value(response).unwrap()).unwrap();
        payload
    }

    #[tokio::test]
    async fn unknown_tool_returns_a_structured_error() {
        let (_d, permissions) = engine();
        let dispatcher = Dispatcher::new(Registry::new(), permissions);
        let (ctx, _e, _p) = dispatch_context();

        let result = dispatcher
            .dispatch(ToolCall { id: "c1".into(), name: "nope".into(), input: json!({}) }, &ctx)
            .await;
        assert!(result.is_error);
        assert!(result.content.contains("no such tool"));
    }

    #[tokio::test]
    async fn a_granted_program_runs_without_prompting() {
        let mut registry = Registry::new();
        registry.register(fake_run("run", false, false)).unwrap();
        let (_d, permissions) = engine();
        permissions.record("git", Class::Read, Choice::AllowSession).unwrap();

        let dispatcher = Dispatcher::new(registry, permissions);
        let (ctx, _e, _p) = dispatch_context();

        let result = dispatcher.dispatch(call_of("git", "read"), &ctx).await;
        assert!(!result.is_error, "{}", result.content);
    }

    /// A read grant is not a write grant, even for the same program — the
    /// distinction the old model could not express at all.
    #[tokio::test]
    async fn a_read_grant_does_not_cover_the_same_programs_writes() {
        let mut registry = Registry::new();
        registry.register(fake_run("run", false, false)).unwrap();
        let (_d, permissions) = engine();
        permissions.record("git", Class::Read, Choice::AllowSession).unwrap();

        let dispatcher = Dispatcher::new(registry, permissions);
        let (ctx, mut events, pending) = dispatch_context();

        let call = dispatcher.dispatch(call_of("git", "write"), &ctx);
        let resolve = answer(&mut events, &pending, Choice::DenyOnce);
        let (result, payload) = tokio::join!(call, resolve);

        assert!(matches!(payload, PromptPayload::Tool { declared: Class::Write, .. }), "{payload:?}");
        assert!(result.is_error);
    }

    /// A lock is refused outright, with no prompt at all — because there is
    /// no answer at a prompt that could lift it. The message names the file.
    #[tokio::test]
    async fn a_locked_program_is_refused_without_drawing_a_prompt() {
        let mut registry = Registry::new();
        registry.register(fake_run("run", false, false)).unwrap();
        let (_d, permissions) = engine();
        permissions.record("curl", Class::Write, Choice::NeverAllow).unwrap();

        let dispatcher = Dispatcher::new(registry, permissions);
        let (ctx, mut events, _p) = dispatch_context();

        let result = dispatcher.dispatch(call_of("curl", "write"), &ctx).await;
        assert!(result.is_error);
        assert!(result.content.contains("permissions.yaml"), "must say where the rule lives: {}", result.content);
        assert!(events.try_recv().is_err(), "a lock must not offer a way to say yes");
    }

    #[tokio::test]
    async fn an_answer_at_the_prompt_is_what_persists() {
        let mut registry = Registry::new();
        registry.register(fake_run("run", false, false)).unwrap();
        let (_d, permissions) = engine();
        let dispatcher = Dispatcher::new(registry, permissions.clone());
        let (ctx, mut events, pending) = dispatch_context();

        let call = dispatcher.dispatch(call_of("cargo", "write"), &ctx);
        let resolve = answer(&mut events, &pending, Choice::AllowProject);
        let (result, _) = tokio::join!(call, resolve);

        assert!(!result.is_error, "{}", result.content);
        let grants = permissions.effective_view().grants;
        assert!(
            grants.iter().any(|g| g.list == GrantList::Allow
                && g.entry == GrantEntry::classed("cargo", Class::Write)),
            "{grants:?}"
        );
    }

    /// ADR 0004 §4, end to end through the dispatcher: the read declaration
    /// is refused by the sandbox, the developer is asked, and a yes re-runs
    /// the same call as a write.
    #[tokio::test]
    async fn a_refused_read_becomes_a_question_and_then_a_re_run() {
        let tool = fake_run("run", false, true);
        let mut registry = Registry::new();
        registry.register(tool.clone()).unwrap();
        let (_d, permissions) = engine();
        permissions.record("rm", Class::Read, Choice::AllowSession).unwrap();

        let dispatcher = Dispatcher::new(registry, permissions);
        let (ctx, mut events, pending) = dispatch_context();

        let call = dispatcher.dispatch(call_of("rm", "read"), &ctx);
        let resolve = answer(&mut events, &pending, Choice::AllowOnce);
        let (result, payload) = tokio::join!(call, resolve);

        assert!(
            matches!(payload, PromptPayload::WriteAttempt { ref program, .. } if program == "rm"),
            "the second prompt must be the write question: {payload:?}"
        );
        assert!(!result.is_error, "{}", result.content);
        assert_eq!(result.content, "ran as write", "the re-run must be declared honestly");
        assert_eq!(tool.calls.load(std::sync::atomic::Ordering::SeqCst), 2, "attempt, then re-run");
    }

    /// ADR 0004 §4, as built by ADR 0007 §6: where a read cannot be
    /// enforced, **every call asks**. This was a flat error. On macOS that
    /// failed every read-declared call, and in the session that surfaced it
    /// the model declared `read` twice, saw both fail, and declared the next
    /// 69 calls `write` — `ls` and `grep` among them.
    #[tokio::test]
    async fn a_read_that_cannot_be_enforced_becomes_a_question_not_an_error() {
        let tool = fake_run_without_a_sandbox("run");
        let mut registry = Registry::new();
        registry.register(tool.clone()).unwrap();
        let (_d, permissions) = engine();
        permissions.record("ls", Class::Read, Choice::AllowSession).unwrap();

        let dispatcher = Dispatcher::new(registry, permissions);
        let (ctx, mut events, pending) = dispatch_context();

        let call = dispatcher.dispatch(call_of("ls", "read"), &ctx);
        let resolve = answer(&mut events, &pending, Choice::AllowOnce);
        let (result, payload) = tokio::join!(call, resolve);

        assert!(
            matches!(payload, PromptPayload::WriteAttempt { ref program, .. } if program == "ls"),
            "an unenforceable read must raise the write question: {payload:?}"
        );
        assert!(!result.is_error, "the model must not be handed an error for this: {}", result.content);
        assert_eq!(result.content, "ran as write");
        assert_eq!(tool.calls.load(std::sync::atomic::Ordering::SeqCst), 2, "attempt, then re-run");
    }

    /// Audit: the question was asked unconditionally, so on a platform with
    /// no sandbox "allow ls writes for this session" changed nothing and the
    /// very next `ls` asked again.
    #[tokio::test]
    async fn an_unenforceable_read_is_not_asked_about_once_writes_are_allowed() {
        let tool = fake_run_without_a_sandbox("run");
        let mut registry = Registry::new();
        registry.register(tool.clone()).unwrap();
        let (_d, permissions) = engine();
        permissions.record("ls", Class::Write, Choice::AllowSession).unwrap();

        let dispatcher = Dispatcher::new(registry, permissions);
        let (ctx, mut events, _pending) = dispatch_context();

        let result = dispatcher.dispatch(call_of("ls", "read"), &ctx).await;

        assert!(!result.is_error, "{}", result.content);
        assert_eq!(result.content, "ran as write");
        assert!(
            !matches!(events.try_recv(), Ok(Event::PromptRequested { .. })),
            "a standing grant must not be asked about again"
        );
    }

    #[tokio::test]
    async fn an_unenforceable_read_the_developer_declines_does_not_run_unconfined() {
        let tool = fake_run_without_a_sandbox("run");
        let mut registry = Registry::new();
        registry.register(tool.clone()).unwrap();
        let (_d, permissions) = engine();
        permissions.record("ls", Class::Read, Choice::AllowSession).unwrap();

        let dispatcher = Dispatcher::new(registry, permissions);
        let (ctx, mut events, pending) = dispatch_context();

        let call = dispatcher.dispatch(call_of("ls", "read"), &ctx);
        let resolve = answer(&mut events, &pending, Choice::DenyOnce);
        let (result, _) = tokio::join!(call, resolve);

        assert!(result.is_error);
        assert_eq!(tool.calls.load(std::sync::atomic::Ordering::SeqCst), 1, "a no means it never runs unconfined");
    }

    #[tokio::test]
    async fn a_refused_read_that_the_developer_declines_does_not_re_run() {
        let tool = fake_run("run", false, true);
        let mut registry = Registry::new();
        registry.register(tool.clone()).unwrap();
        let (_d, permissions) = engine();
        permissions.record("rm", Class::Read, Choice::AllowSession).unwrap();

        let dispatcher = Dispatcher::new(registry, permissions);
        let (ctx, mut events, pending) = dispatch_context();

        let call = dispatcher.dispatch(call_of("rm", "read"), &ctx);
        let resolve = answer(&mut events, &pending, Choice::DenyOnce);
        let (result, _) = tokio::join!(call, resolve);

        assert!(result.is_error);
        assert_eq!(tool.calls.load(std::sync::atomic::Ordering::SeqCst), 1, "no re-run without a yes");
    }

    /// The reported bug: "permissions don't appear to count properly when
    /// commands are queued." Both calls of a step are dispatched
    /// concurrently, so both used to reach the engine before the developer
    /// had answered anything and both were told to ask — the grant made in
    /// answer to the first could not affect the second, whose outcome was
    /// already fixed. Now the second waits on `prompt_gate` and re-checks.
    #[tokio::test]
    async fn a_grant_answered_for_one_queued_call_covers_the_others() {
        let mut registry = Registry::new();
        registry.register(fake_run("run", false, false)).unwrap();
        let (_d, permissions) = engine();
        let dispatcher = Dispatcher::new(registry, permissions);
        let (ctx, mut events, pending) = dispatch_context();

        let first = dispatcher.dispatch(
            ToolCall { id: "c1".into(), name: "run".into(), input: json!({"program": "git", "class": "read"}) },
            &ctx,
        );
        let second = dispatcher.dispatch(
            ToolCall { id: "c2".into(), name: "run".into(), input: json!({"program": "git", "class": "read"}) },
            &ctx,
        );
        let resolve = answer(&mut events, &pending, Choice::AllowSession);

        // Bounded, because the pre-fix failure mode is not a wrong answer but
        // a hang: the second call raised its own prompt, and with only one
        // answer sent, `join!` would wait on it forever.
        let joined = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            tokio::join!(first, second, resolve)
        });
        let (first, second, _) = joined
            .await
            .expect("the queued call must resolve from the grant already made, not wait on a second prompt");

        assert!(!first.is_error, "{}", first.content);
        assert!(!second.is_error, "{}", second.content);
        assert!(events.try_recv().is_err(), "the queued call must not raise a second prompt");
    }

    #[tokio::test]
    async fn an_edit_class_tool_never_reaches_the_permission_path() {
        let mut registry = Registry::new();
        registry.register(fake_run("edit", true, false)).unwrap();
        let (_d, permissions) = engine();

        let dispatcher = Dispatcher::new(registry, permissions);
        let (ctx, _e, _p) = dispatch_context();

        // Nothing is granted and nothing will answer a prompt. If the
        // dispatcher ran the permission path for this tool, this would hang
        // awaiting an answer that never comes.
        let result = dispatcher
            .dispatch(ToolCall { id: "c1".into(), name: "edit".into(), input: json!({"program": "edit", "class": "read"}) }, &ctx)
            .await;
        assert!(!result.is_error);
    }
}
