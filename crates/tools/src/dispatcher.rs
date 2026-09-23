use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use aldwin_core::{DispatchContext, Event, ReviewComment, ReviewDecision, ReviewOutcome, ToolCall, ToolDefinition, ToolResult};
use aldwin_permissions::{Locks, Outcome};
use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::error::ToolError;
use crate::registry::{Registry, ToolSource};
use crate::staging::Staging;

/// Implements core's `ToolDispatcher`. Owns the dispatch flow of ADR 0009:
/// resolve name -> tool, refuse a locked program, run it — and, at the two
/// moments a staged changeset would otherwise be observed without one, open
/// the review.
///
/// There is no prompt in this flow. A read declaration is held to its word
/// by the sandbox; a wrong one comes back to the model as an error it can
/// re-declare from, not to the developer as a question.
pub struct Dispatcher {
    registry: Registry,
    locks:    Arc<Locks>,
    staging:  Arc<Staging>,
    /// Serialises the review across the concurrent calls of a step and the
    /// turn's end, so one changeset is never reviewed twice at once.
    review:   tokio::sync::Mutex<()>,
    /// Where a read cannot be enforced, the call runs unconfined and the
    /// developer is told so — once. Said at the first such call rather than
    /// at startup, so a session that never runs anything never hears it.
    said_unconfined: AtomicBool,
    /// Where the one-time notice goes. `None` in tests that build a
    /// dispatcher without a session.
    notices: Option<mpsc::Sender<Event>>,
}

impl Dispatcher {
    pub fn new(registry: Registry, locks: Arc<Locks>, staging: Arc<Staging>) -> Self {
        Self { registry, locks, staging, review: tokio::sync::Mutex::new(()), said_unconfined: AtomicBool::new(false), notices: None }
    }

    /// Where the dispatcher's own notices — today only the unconfined-run
    /// warning — reach the developer.
    pub fn with_notices(mut self, notices: mpsc::Sender<Event>) -> Self {
        self.notices = Some(notices);
        self
    }

    /// Whether any of a step's calls would see the disk. `run` executes a
    /// program over the real tree; an MCP tool runs in its own process. The
    /// other built-ins read through the staging overlay, or nothing at all.
    fn observes_disk(&self, calls: &[ToolCall]) -> bool {
        calls.iter().any(|call| {
            call.name == "run"
                || self.registry.get(&call.name).is_some_and(|t| matches!(t.descriptor().source, ToolSource::Mcp { .. }))
        })
    }

    /// Opens the review over what is staged and acts on the decision.
    async fn review(&self, ctx: &DispatchContext) -> Reviewed {
        let _one_at_a_time = self.review.lock().await;
        if self.staging.is_empty() {
            return Reviewed::Proceed;
        }
        let changeset = self.staging.changeset();
        match ctx.review(changeset).await {
            Some(ReviewDecision::Approve) => {
                let written = self.staging.write_all().await;
                for (path, why) in &written.skipped {
                    let _ = ctx.review_closed(ReviewOutcome::Discarded { files: vec![path.clone()] }).await;
                    self.say(format!("{path} was not written: {why}")).await;
                }
                ctx.review_closed(ReviewOutcome::Saved { files: written.files, comments_resolved: written.comments_resolved }).await;
                Reviewed::Proceed
            }
            Some(ReviewDecision::Comment { comments }) => {
                self.staging.note_comments(comments.len());
                ctx.review_closed(ReviewOutcome::Commented { comments: comments.len() }).await;
                Reviewed::Reason(render_comments(&comments))
            }
            // Said in the developer's voice: at the end of a turn this text
            // *is* the next turn's message, and the TUI echoes it as theirs
            // (`Event::FollowUp`) — the discard is something they did.
            Some(ReviewDecision::Discard) => {
                let files = self.staging.discard();
                ctx.review_closed(ReviewOutcome::Discarded { files }).await;
                Reviewed::Reason("I discarded the staged changes; nothing was written. Do not stage the same edits again unless I ask.".into())
            }
            None => Reviewed::Gone,
        }
    }

    async fn say(&self, message: String) {
        if let Some(notices) = &self.notices {
            let _ = notices.send(Event::Notice { message }).await;
        }
    }
}

/// How a review ended, from the model's side.
enum Reviewed {
    /// Written, or nothing was staged: the model may go on.
    Proceed,
    /// A message for the model — the developer's comments, or that they
    /// discarded the changes — meaning the work is not done.
    Reason(String),
    /// Nobody answered: the session is ending under the review. Nothing was
    /// written and the staging area keeps what it had. A *cancel* never
    /// lands here — the agent drops the review future instead
    /// (`Agent::await_or_cancel`) — so this is shutdown, and the two hooks
    /// treat it differently: a step's calls must not run, and a turn must
    /// not start a next one.
    Gone,
}

/// The developer's comments as one message to the model: where, then what.
fn render_comments(comments: &[ReviewComment]) -> String {
    comments
        .iter()
        .map(|c| {
            // A comment with no path is what the developer typed into the
            // review's field — about the change as a whole, not a line.
            if c.path.is_empty() {
                return c.text.clone();
            }
            let lines = if c.lines.0 == c.lines.1 { format!("line {}", c.lines.0) } else { format!("lines {}–{}", c.lines.0, c.lines.1) };
            format!("On {}, {lines}:\n{}", c.path, c.text)
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[async_trait]
impl aldwin_core::ToolDispatcher for Dispatcher {
    async fn dispatch(&self, call: ToolCall, ctx: &DispatchContext) -> ToolResult {
        let Some(tool) = self.registry.get(&call.name) else {
            return error_result(&call.id, ToolError::UnknownTool { name: call.name });
        };

        match tool.permission(&call.input) {
            Ok(None) => {}
            Ok(Some(request)) => {
                if let Outcome::Locked { scope, .. } = self.locks.check(&request.program, request.class) {
                    return error_result(&call.id, ToolError::Locked { program: request.program, where_it_lives: scope.where_it_lives() });
                }
            }
            Err(e) => return error_result(&call.id, e),
        }

        match tool.call(&call.id, call.input.clone(), ctx).await {
            // There is no enforcement primitive on this platform, so the
            // read declaration cannot be honoured. ADR 0009 §3: the call runs
            // unconfined and the developer is told once. Nothing has run yet
            // — the sandbox refused to be built — so this is a first run,
            // not a retry.
            Err(ToolError::SandboxUnavailable { source, .. }) => {
                if !self.said_unconfined.swap(true, Ordering::SeqCst) {
                    self.say(format!("Runs are not sandboxed on this system ({source}); a call declared a read runs with the tree writable."))
                        .await;
                }
                finish(&call.id, tool.call(&call.id, as_write(call.input), ctx).await)
            }
            other => finish(&call.id, other),
        }
    }

    fn definitions(&self) -> Vec<ToolDefinition> {
        self.registry.definitions()
    }

    /// A staged changeset is reviewed before any call that would see the
    /// disk without it — a test run over unapproved edits would otherwise
    /// need the edits written first, which is the one thing that must not
    /// happen without the review.
    async fn before_step(&self, calls: &[ToolCall], ctx: &DispatchContext) -> Option<String> {
        if self.staging.is_empty() || !self.observes_disk(calls) {
            return None;
        }
        match self.review(ctx).await {
            Reviewed::Proceed => None,
            Reviewed::Reason(text) => Some(text),
            Reviewed::Gone => Some("the review was not answered; nothing was written and nothing ran".into()),
        }
    }

    /// The model has stopped. Whatever is staged is reviewed now, and a
    /// comment starts the next turn.
    async fn turn_ending(&self, ctx: &DispatchContext) -> Option<String> {
        match self.review(ctx).await {
            Reviewed::Proceed | Reviewed::Gone => None,
            Reviewed::Reason(text) => Some(text),
        }
    }
}

/// Re-declares a call as a write, for the unconfined run on a platform with
/// no sandbox. The tool re-parses its own input, so the class has to change
/// in the input rather than beside it.
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
    use crate::registry::{PermissionRequest, Tool, ToolDescriptor};
    use crate::test_support::dispatch_context;
    use aldwin_config::{Config, GrantEntry, GrantList, Scope};
    use aldwin_core::{ChangedFile, Changeset, PendingReply, ToolDispatcher as _};
    use aldwin_permissions::Class;
    use serde_json::{json, Value};

    /// A stand-in for `run`: it takes a program and a declared class the same
    /// way, and can be told its sandbox is missing.
    struct FakeRun {
        descriptor: ToolDescriptor,
        calls:      std::sync::atomic::AtomicUsize,
        no_sandbox: bool,
    }

    #[async_trait]
    impl Tool for FakeRun {
        fn descriptor(&self) -> &ToolDescriptor {
            &self.descriptor
        }
        fn permission(&self, input: &Value) -> Result<Option<PermissionRequest>, ToolError> {
            Ok(Some(PermissionRequest {
                program: input.get("program").and_then(Value::as_str).unwrap_or_default().to_string(),
                class:   match input.get("class").and_then(Value::as_str) {
                    Some("write") => Class::Write,
                    _ => Class::Read,
                },
                argv: Vec::new(),
            }))
        }
        async fn call(&self, _id: &str, input: Value, _ctx: &DispatchContext) -> Result<String, ToolError> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let declared = input.get("class").and_then(Value::as_str).unwrap_or("read");
            if self.no_sandbox && declared == "read" {
                return Err(ToolError::SandboxUnavailable {
                    program: "x".into(),
                    args:    Vec::new(),
                    source:  std::io::Error::new(std::io::ErrorKind::Unsupported, "no enforcement here"),
                });
            }
            Ok(format!("ran as {declared}"))
        }
    }

    fn fake_run(name: &str, no_sandbox: bool) -> Arc<FakeRun> {
        Arc::new(FakeRun {
            descriptor: ToolDescriptor { name: name.into(), description: "fake".into(), input_schema: json!({}), source: ToolSource::Builtin },
            calls:      std::sync::atomic::AtomicUsize::new(0),
            no_sandbox,
        })
    }

    fn locks() -> (tempfile::TempDir, Arc<Locks>) {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::open_at(dir.path(), dir.path().join("global")).unwrap();
        (dir, Arc::new(Locks::new(config)))
    }

    fn call_of(program: &str, class: &str) -> ToolCall {
        ToolCall { id: "c1".into(), name: "run".into(), input: json!({"program": program, "class": class}) }
    }

    fn dispatcher(registry: Registry, locks: Arc<Locks>) -> (Dispatcher, Arc<Staging>) {
        let staging = Arc::new(Staging::new());
        (Dispatcher::new(registry, locks, staging.clone()), staging)
    }

    #[tokio::test]
    async fn unknown_tool_returns_a_structured_error() {
        let (_d, locks) = locks();
        let (dispatcher, _) = dispatcher(Registry::new(), locks);
        let (ctx, _e, _p) = dispatch_context();
        let result = dispatcher.dispatch(ToolCall { id: "c1".into(), name: "nope".into(), input: json!({}) }, &ctx).await;
        assert!(result.is_error);
        assert!(result.content.contains("no such tool"));
    }

    /// ADR 0009 §1: nothing is granted and nothing asks. A fresh project
    /// runs what it is asked to.
    #[tokio::test]
    async fn a_call_runs_without_a_grant_and_without_a_prompt() {
        let mut registry = Registry::new();
        registry.register(fake_run("run", false)).unwrap();
        let (_d, locks) = locks();
        let (dispatcher, _) = dispatcher(registry, locks);
        let (ctx, mut events, _p) = dispatch_context();

        let result = dispatcher.dispatch(call_of("git", "write"), &ctx).await;
        assert!(!result.is_error, "{}", result.content);
        assert_eq!(result.content, "ran as write");
        assert!(events.try_recv().is_err(), "nothing was asked");
    }

    /// A lock is refused outright, and the message names the file.
    #[tokio::test]
    async fn a_locked_program_is_refused_and_the_refusal_names_the_file() {
        let mut registry = Registry::new();
        registry.register(fake_run("run", false)).unwrap();
        let (_d, locks) = locks();
        locks.config_for_tests().add_grant(Scope::Global, GrantList::Deny, GrantEntry::program("curl")).unwrap();
        let (dispatcher, _) = dispatcher(registry, locks);
        let (ctx, mut events, _p) = dispatch_context();

        let result = dispatcher.dispatch(call_of("curl", "read"), &ctx).await;
        assert!(result.is_error);
        assert!(result.content.contains("permissions.yaml"), "{}", result.content);
        assert!(events.try_recv().is_err(), "a lock offers no way to say yes");
    }

    /// ADR 0009 §3: where a read cannot be enforced the call runs unconfined
    /// and the developer hears about it once, not every time.
    #[tokio::test]
    async fn an_unenforceable_read_runs_unconfined_and_says_so_once() {
        let tool = fake_run("run", true);
        let mut registry = Registry::new();
        registry.register(tool.clone()).unwrap();
        let (_d, locks) = locks();
        let (tx, mut notices) = mpsc::channel(8);
        let (dispatcher, _) = dispatcher(registry, locks);
        let dispatcher = dispatcher.with_notices(tx);
        let (ctx, _e, _p) = dispatch_context();

        let first = dispatcher.dispatch(call_of("ls", "read"), &ctx).await;
        assert_eq!(first.content, "ran as write");
        assert_eq!(tool.calls.load(std::sync::atomic::Ordering::SeqCst), 2, "attempt, then the unconfined run");
        assert!(matches!(notices.try_recv(), Ok(Event::Notice { message }) if message.contains("not sandboxed")));

        dispatcher.dispatch(call_of("ls", "read"), &ctx).await;
        assert!(notices.try_recv().is_err(), "said once");
    }

    /// A tool outside the lock never consults it — nothing to check, and a
    /// deny on its name would be meaningless.
    #[tokio::test]
    async fn a_tool_outside_the_lock_runs_even_when_its_name_is_denied() {
        struct Outside(ToolDescriptor);
        #[async_trait]
        impl Tool for Outside {
            fn descriptor(&self) -> &ToolDescriptor { &self.0 }
            fn permission(&self, _: &Value) -> Result<Option<PermissionRequest>, ToolError> { Ok(None) }
            async fn call(&self, _: &str, _: Value, _: &DispatchContext) -> Result<String, ToolError> { Ok("ran".into()) }
        }
        let mut registry = Registry::new();
        registry.register(Arc::new(Outside(ToolDescriptor { name: "plan".into(), description: String::new(), input_schema: json!({}), source: ToolSource::Builtin }))).unwrap();
        let (_d, locks) = locks();
        locks.config_for_tests().add_grant(Scope::Global, GrantList::Deny, GrantEntry::program("plan")).unwrap();
        let (dispatcher, _) = dispatcher(registry, locks);
        let (ctx, _e, _p) = dispatch_context();
        let result = dispatcher.dispatch(ToolCall { id: "c1".into(), name: "plan".into(), input: json!({}) }, &ctx).await;
        assert!(!result.is_error);
    }

    // ── The review ────────────────────────────────────────────────────────

    async fn stage(staging: &Staging, dir: &tempfile::TempDir, name: &str, after: &str) {
        let path = dir.path().join(name);
        std::fs::write(&path, "before\n").unwrap();
        staging.edit(path, name, |_| Ok(after.into())).await.unwrap();
    }

    /// Answers the next review with `decision`, returning the changeset shown.
    async fn decide(events: &mut mpsc::Receiver<Event>, pending: &aldwin_core::PendingMap, decision: ReviewDecision) -> Changeset {
        let Some(Event::ReviewRequested { review_id, changeset }) = events.recv().await else { panic!("expected a review") };
        let Some(PendingReply::Review(tx)) = pending.lock().unwrap().remove(&review_id) else { panic!("no pending review") };
        tx.send(decision).unwrap();
        changeset
    }

    #[tokio::test]
    async fn nothing_staged_means_no_review_at_either_moment() {
        let (_d, locks) = locks();
        let (dispatcher, _) = dispatcher(Registry::new(), locks);
        let (ctx, mut events, _p) = dispatch_context();
        assert_eq!(dispatcher.before_step(&[call_of("cargo", "write")], &ctx).await, None);
        assert_eq!(dispatcher.turn_ending(&ctx).await, None);
        assert!(events.try_recv().is_err());
    }

    /// A read does not observe the disk — it reads through the overlay — so
    /// a step of reads opens no review even with edits staged.
    #[tokio::test]
    async fn a_step_that_does_not_observe_the_disk_is_not_reviewed_first() {
        let dir = tempfile::tempdir().unwrap();
        let (_d, locks) = locks();
        let (dispatcher, staging) = dispatcher(Registry::new(), locks);
        stage(&staging, &dir, "f.rs", "after\n").await;
        let (ctx, mut events, _p) = dispatch_context();
        let reads = [ToolCall { id: "c1".into(), name: "read".into(), input: json!({}) }];
        assert_eq!(dispatcher.before_step(&reads, &ctx).await, None);
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn a_run_over_staged_edits_is_reviewed_first_and_an_approve_writes_them() {
        let dir = tempfile::tempdir().unwrap();
        let (_d, locks) = locks();
        let (dispatcher, staging) = dispatcher(Registry::new(), locks);
        stage(&staging, &dir, "f.rs", "after\n").await;
        let (ctx, mut events, pending) = dispatch_context();

        let calls = [call_of("cargo", "write")];
        let step = dispatcher.before_step(&calls, &ctx);
        let (outcome, shown) = tokio::join!(step, decide(&mut events, &pending, ReviewDecision::Approve));

        assert_eq!(outcome, None, "approved: the run may proceed");
        assert_eq!(shown.files, vec![ChangedFile { path: "f.rs".into(), before: Some("before\n".into()), after: "after\n".into() }]);
        assert_eq!(std::fs::read_to_string(dir.path().join("f.rs")).unwrap(), "after\n");
        assert!(staging.is_empty());
        assert!(matches!(events.recv().await, Some(Event::ReviewClosed { outcome: ReviewOutcome::Saved { files, comments_resolved: 0 } }) if files == vec!["f.rs".to_string()]));
    }

    #[tokio::test]
    async fn comments_come_back_as_the_reason_the_step_did_not_run_and_the_changes_stay_staged() {
        let dir = tempfile::tempdir().unwrap();
        let (_d, locks) = locks();
        let (dispatcher, staging) = dispatcher(Registry::new(), locks);
        stage(&staging, &dir, "f.rs", "after\n").await;
        let (ctx, mut events, pending) = dispatch_context();

        let comments = vec![ReviewComment { path: "f.rs".into(), lines: (1, 2), text: "Use config".into() }];
        let calls = [call_of("cargo", "write")];
        let step = dispatcher.before_step(&calls, &ctx);
        let (outcome, _) = tokio::join!(step, decide(&mut events, &pending, ReviewDecision::Comment { comments }));

        assert_eq!(outcome.as_deref(), Some("On f.rs, lines 1–2:\nUse config"));
        assert!(!staging.is_empty(), "the changeset waits for the next review");
        assert_eq!(std::fs::read_to_string(dir.path().join("f.rs")).unwrap(), "before\n");
        assert!(matches!(events.recv().await, Some(Event::ReviewClosed { outcome: ReviewOutcome::Commented { comments: 1 } })));

        // The next approve reports the comment resolved.
        let (ctx, mut events, pending) = dispatch_context();
        let ending = dispatcher.turn_ending(&ctx);
        let (outcome, _) = tokio::join!(ending, decide(&mut events, &pending, ReviewDecision::Approve));
        assert_eq!(outcome, None);
        assert!(matches!(events.recv().await, Some(Event::ReviewClosed { outcome: ReviewOutcome::Saved { comments_resolved: 1, .. } })));
    }

    #[tokio::test]
    async fn a_discard_drops_the_changes_and_tells_the_model() {
        let dir = tempfile::tempdir().unwrap();
        let (_d, locks) = locks();
        let (dispatcher, staging) = dispatcher(Registry::new(), locks);
        stage(&staging, &dir, "f.rs", "after\n").await;
        let (ctx, mut events, pending) = dispatch_context();

        let ending = dispatcher.turn_ending(&ctx);
        let (outcome, _) = tokio::join!(ending, decide(&mut events, &pending, ReviewDecision::Discard));

        assert!(outcome.unwrap().contains("discarded"));
        assert!(staging.is_empty());
        assert_eq!(std::fs::read_to_string(dir.path().join("f.rs")).unwrap(), "before\n");
        assert!(matches!(events.recv().await, Some(Event::ReviewClosed { outcome: ReviewOutcome::Discarded { files } }) if files == vec!["f.rs".to_string()]));
    }

    /// Nobody answers the review — the session is ending under it. A step's
    /// calls must not run; a turn must not start a next one; and what was
    /// staged stays staged, unwritten.
    #[tokio::test]
    async fn an_unanswered_review_runs_nothing_and_starts_no_turn() {
        let dir = tempfile::tempdir().unwrap();
        let (_d, locks) = locks();
        let (dispatcher, staging) = dispatcher(Registry::new(), locks);
        stage(&staging, &dir, "f.rs", "after\n").await;

        async fn walk_away(events: &mut mpsc::Receiver<Event>, pending: &aldwin_core::PendingMap) {
            let Some(Event::ReviewRequested { review_id, .. }) = events.recv().await else { panic!("expected a review") };
            drop(pending.lock().unwrap().remove(&review_id));
        }

        let (ctx, mut events, pending) = dispatch_context();
        let calls = [call_of("cargo", "write")];
        let (outcome, _) = tokio::join!(dispatcher.before_step(&calls, &ctx), walk_away(&mut events, &pending));
        assert!(outcome.is_some(), "the step's calls are answered, not run");

        let (ctx, mut events, pending) = dispatch_context();
        let (outcome, _) = tokio::join!(dispatcher.turn_ending(&ctx), walk_away(&mut events, &pending));
        assert_eq!(outcome, None, "no next turn");

        assert!(!staging.is_empty());
        assert_eq!(std::fs::read_to_string(dir.path().join("f.rs")).unwrap(), "before\n");
    }

    #[test]
    fn comments_render_where_then_what_and_a_general_one_as_itself() {
        let one = vec![ReviewComment { path: "a.rs".into(), lines: (4, 4), text: "x".into() }];
        assert_eq!(render_comments(&one), "On a.rs, line 4:\nx");
        let general = vec![ReviewComment { path: String::new(), lines: (0, 0), text: "and rename it".into() }];
        assert_eq!(render_comments(&general), "and rename it");
    }
}
