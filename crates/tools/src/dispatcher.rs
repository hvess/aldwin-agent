use std::sync::Arc;

use aldwin_core::{
    DispatchContext, Event, ReviewComment, ReviewDecision, ReviewOutcome, ToolCall, ToolDefinition,
    ToolResult,
};
use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::error::ToolError;
use crate::registry::Registry;
use crate::staging::Staging;

/// Core's `ToolDispatcher`: runs a call by name, and opens the review before
/// a staged changeset would be observed on disk (ADR 0009 §4).
///
/// No call is gated here: the workspace is the only boundary (ADR 0011),
/// held by each tool and the sandbox.
#[derive(Debug)]
pub struct Dispatcher {
    registry: Registry,
    staging: Arc<Staging>,
    /// Serialises reviews, so one changeset is never reviewed twice at once.
    review: tokio::sync::Mutex<()>,
    /// Receives notices such as a staged file an approve could not write.
    /// `None` without a session.
    notices: Option<mpsc::Sender<Event>>,
}

impl Dispatcher {
    /// A dispatcher over `registry` and `staging`; notices are dropped until
    /// `with_notices`.
    pub fn new(registry: Registry, staging: Arc<Staging>) -> Self {
        Self {
            registry,
            staging,
            review: tokio::sync::Mutex::new(()),
            notices: None,
        }
    }

    /// Sends the dispatcher's notices to the session's event channel.
    pub fn with_notices(mut self, notices: mpsc::Sender<Event>) -> Self {
        self.notices = Some(notices);
        self
    }

    /// Whether any call would see disk rather than the overlay. Read from
    /// each descriptor: the dispatcher must know no tool by name.
    fn observes_disk(&self, calls: &[ToolCall]) -> bool {
        calls.iter().any(|call| {
            self.registry
                .get(&call.name)
                .is_some_and(|t| t.descriptor().observes_disk)
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
                    let _ = ctx
                        .review_closed(ReviewOutcome::Discarded {
                            files: vec![path.clone()],
                        })
                        .await;
                    self.say(format!("{path} was not written: {why}")).await;
                }
                ctx.review_closed(ReviewOutcome::Saved {
                    files: written.files,
                    comments_resolved: written.comments_resolved,
                })
                .await;
                Reviewed::Proceed
            }
            Some(ReviewDecision::Comment { comments }) => {
                self.staging.note_comments(comments.len());
                ctx.review_closed(ReviewOutcome::Commented {
                    comments: comments.len(),
                })
                .await;
                Reviewed::Reason(render_comments(&comments))
            }
            // In the developer's voice: at a turn's end this is the next
            // turn's message, echoed as theirs (`Event::FollowUp`).
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
    /// Comments or a discard, for the model: the work is not done.
    Reason(String),
    /// Unanswered because the session is ending; nothing written, staging
    /// kept. A cancel drops the future instead (`Agent::await_or_cancel`).
    /// A step's calls must not run, and a turn must not start another.
    Gone,
}

/// The developer's comments as one message to the model: where, then what.
fn render_comments(comments: &[ReviewComment]) -> String {
    comments
        .iter()
        .map(|c| {
            // No path: a comment on the whole change.
            if c.path.is_empty() {
                return c.text.clone();
            }
            let lines = if c.lines.0 == c.lines.1 {
                format!("line {}", c.lines.0)
            } else {
                format!("lines {}–{}", c.lines.0, c.lines.1)
            };
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

        finish(&call.id, tool.call(&call.id, call.input, ctx).await)
    }

    fn definitions(&self) -> Vec<ToolDefinition> {
        self.registry.definitions()
    }

    /// Reviews a staged changeset before any call that would see disk
    /// without it: nothing reaches disk before an approve.
    async fn before_step(&self, calls: &[ToolCall], ctx: &DispatchContext) -> Option<String> {
        if self.staging.is_empty() || !self.observes_disk(calls) {
            return None;
        }
        match self.review(ctx).await {
            Reviewed::Proceed => None,
            Reviewed::Reason(text) => Some(text),
            Reviewed::Gone => {
                Some("the review was not answered; nothing was written and nothing ran".into())
            }
        }
    }

    /// Reviews whatever is staged; a comment or discard starts the next turn.
    async fn turn_ending(&self, ctx: &DispatchContext) -> Option<String> {
        match self.review(ctx).await {
            Reviewed::Proceed | Reviewed::Gone => None,
            Reviewed::Reason(text) => Some(text),
        }
    }
}

fn finish(call_id: &str, outcome: Result<String, ToolError>) -> ToolResult {
    match outcome {
        Ok(content) => ToolResult {
            call_id: call_id.to_string(),
            content,
            is_error: false,
        },
        Err(e) => error_result(call_id, e),
    }
}

fn error_result(call_id: &str, err: ToolError) -> ToolResult {
    ToolResult {
        call_id: call_id.to_string(),
        content: err.to_string(),
        is_error: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Tool, ToolDescriptor};
    use crate::test_support::dispatch_context;
    use aldwin_core::{ChangedFile, Changeset, PendingReply, ToolDispatcher as _};
    use serde_json::{json, Value};

    struct Fake(ToolDescriptor);

    #[async_trait]
    impl Tool for Fake {
        fn descriptor(&self) -> &ToolDescriptor {
            &self.0
        }
        async fn call(&self, _: &str, _: Value, _: &DispatchContext) -> Result<String, ToolError> {
            Ok("ran".into())
        }
    }

    fn fake(name: &str, observes_disk: bool) -> Arc<Fake> {
        Arc::new(Fake(ToolDescriptor {
            name: name.into(),
            description: "fake".into(),
            input_schema: json!({}),
            observes_disk,
        }))
    }

    fn call_of(name: &str) -> ToolCall {
        ToolCall {
            id: "c1".into(),
            name: name.into(),
            input: json!({}),
        }
    }

    /// `shell` observes disk and `look` does not; neither is a real tool name.
    fn dispatcher(root: &std::path::Path) -> (Dispatcher, Arc<Staging>) {
        let mut registry = Registry::new();
        registry.register(fake("shell", true)).unwrap();
        registry.register(fake("look", false)).unwrap();
        let staging = Arc::new(Staging::new(crate::Workspace::new(root)));
        (Dispatcher::new(registry, staging.clone()), staging)
    }

    #[tokio::test]
    async fn unknown_tool_returns_a_structured_error() {
        let (dispatcher, _) = dispatcher(std::path::Path::new("."));
        let (ctx, _e, _p) = dispatch_context();
        let result = dispatcher.dispatch(call_of("nope"), &ctx).await;
        assert!(result.is_error);
        assert!(result.content.contains("no such tool"));
    }

    #[tokio::test]
    async fn a_call_runs_without_a_grant_and_without_a_prompt() {
        let (dispatcher, _) = dispatcher(std::path::Path::new("."));
        let (ctx, mut events, _p) = dispatch_context();
        let result = dispatcher.dispatch(call_of("shell"), &ctx).await;
        assert!(!result.is_error, "{}", result.content);
        assert_eq!(result.content, "ran");
        assert!(events.try_recv().is_err(), "nothing was asked");
    }

    async fn stage(staging: &Staging, dir: &tempfile::TempDir, name: &str, after: &str) {
        let path = dir.path().canonicalize().unwrap().join(name);
        std::fs::write(&path, "before\n").unwrap();
        staging
            .edit(path, name, |_| Ok(after.into()))
            .await
            .unwrap();
    }

    /// Answers the next review with `decision`, returning the changeset shown.
    async fn decide(
        events: &mut mpsc::Receiver<Event>,
        pending: &aldwin_core::PendingMap,
        decision: ReviewDecision,
    ) -> Changeset {
        let Some(Event::ReviewRequested {
            review_id,
            changeset,
        }) = events.recv().await
        else {
            panic!("expected a review")
        };
        let Some(PendingReply::Review(tx)) = pending.lock().unwrap().remove(&review_id) else {
            panic!("no pending review")
        };
        tx.send(decision).unwrap();
        changeset
    }

    #[tokio::test]
    async fn nothing_staged_means_no_review_at_either_moment() {
        let (dispatcher, _) = dispatcher(std::path::Path::new("."));
        let (ctx, mut events, _p) = dispatch_context();
        assert_eq!(
            dispatcher.before_step(&[call_of("shell")], &ctx).await,
            None
        );
        assert_eq!(dispatcher.turn_ending(&ctx).await, None);
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn a_step_that_does_not_observe_the_disk_is_not_reviewed_first() {
        let dir = tempfile::tempdir().unwrap();
        let (dispatcher, staging) = dispatcher(dir.path());
        stage(&staging, &dir, "f.rs", "after\n").await;
        let (ctx, mut events, _p) = dispatch_context();
        let reads = [ToolCall {
            id: "c1".into(),
            name: "look".into(),
            input: json!({}),
        }];
        assert_eq!(dispatcher.before_step(&reads, &ctx).await, None);
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn a_run_over_staged_edits_is_reviewed_first_and_an_approve_writes_them() {
        let dir = tempfile::tempdir().unwrap();
        let (dispatcher, staging) = dispatcher(dir.path());
        stage(&staging, &dir, "f.rs", "after\n").await;
        let (ctx, mut events, pending) = dispatch_context();

        let calls = [call_of("shell")];
        let step = dispatcher.before_step(&calls, &ctx);
        let (outcome, shown) =
            tokio::join!(step, decide(&mut events, &pending, ReviewDecision::Approve));

        assert_eq!(outcome, None, "approved: the run may proceed");
        assert_eq!(
            shown.files,
            vec![ChangedFile {
                path: "f.rs".into(),
                before: Some("before\n".into()),
                after: "after\n".into()
            }]
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.rs")).unwrap(),
            "after\n"
        );
        assert!(staging.is_empty());
        assert!(
            matches!(events.recv().await, Some(Event::ReviewClosed { outcome: ReviewOutcome::Saved { files, comments_resolved: 0 } }) if files == vec!["f.rs".to_string()])
        );
    }

    #[tokio::test]
    async fn comments_come_back_as_the_reason_the_step_did_not_run_and_the_changes_stay_staged() {
        let dir = tempfile::tempdir().unwrap();
        let (dispatcher, staging) = dispatcher(dir.path());
        stage(&staging, &dir, "f.rs", "after\n").await;
        let (ctx, mut events, pending) = dispatch_context();

        let comments = vec![ReviewComment {
            path: "f.rs".into(),
            lines: (1, 2),
            text: "Use config".into(),
        }];
        let calls = [call_of("shell")];
        let step = dispatcher.before_step(&calls, &ctx);
        let (outcome, _) = tokio::join!(
            step,
            decide(&mut events, &pending, ReviewDecision::Comment { comments })
        );

        assert_eq!(outcome.as_deref(), Some("On f.rs, lines 1–2:\nUse config"));
        assert!(
            !staging.is_empty(),
            "the changeset waits for the next review"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.rs")).unwrap(),
            "before\n"
        );
        assert!(matches!(
            events.recv().await,
            Some(Event::ReviewClosed {
                outcome: ReviewOutcome::Commented { comments: 1 }
            })
        ));

        let (ctx, mut events, pending) = dispatch_context();
        let ending = dispatcher.turn_ending(&ctx);
        let (outcome, _) = tokio::join!(
            ending,
            decide(&mut events, &pending, ReviewDecision::Approve)
        );
        assert_eq!(outcome, None);
        assert!(matches!(
            events.recv().await,
            Some(Event::ReviewClosed {
                outcome: ReviewOutcome::Saved {
                    comments_resolved: 1,
                    ..
                }
            })
        ));
    }

    #[tokio::test]
    async fn a_discard_drops_the_changes_and_tells_the_model() {
        let dir = tempfile::tempdir().unwrap();
        let (dispatcher, staging) = dispatcher(dir.path());
        stage(&staging, &dir, "f.rs", "after\n").await;
        let (ctx, mut events, pending) = dispatch_context();

        let ending = dispatcher.turn_ending(&ctx);
        let (outcome, _) = tokio::join!(
            ending,
            decide(&mut events, &pending, ReviewDecision::Discard)
        );

        assert!(outcome.unwrap().contains("discarded"));
        assert!(staging.is_empty());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.rs")).unwrap(),
            "before\n"
        );
        assert!(
            matches!(events.recv().await, Some(Event::ReviewClosed { outcome: ReviewOutcome::Discarded { files } }) if files == vec!["f.rs".to_string()])
        );
    }

    /// Staged edits also stay staged, unwritten.
    #[tokio::test]
    async fn an_unanswered_review_runs_nothing_and_starts_no_turn() {
        let dir = tempfile::tempdir().unwrap();
        let (dispatcher, staging) = dispatcher(dir.path());
        stage(&staging, &dir, "f.rs", "after\n").await;

        async fn walk_away(events: &mut mpsc::Receiver<Event>, pending: &aldwin_core::PendingMap) {
            let Some(Event::ReviewRequested { review_id, .. }) = events.recv().await else {
                panic!("expected a review")
            };
            drop(pending.lock().unwrap().remove(&review_id));
        }

        let (ctx, mut events, pending) = dispatch_context();
        let calls = [call_of("shell")];
        let (outcome, _) = tokio::join!(
            dispatcher.before_step(&calls, &ctx),
            walk_away(&mut events, &pending)
        );
        assert!(outcome.is_some(), "the step's calls are answered, not run");

        let (ctx, mut events, pending) = dispatch_context();
        let (outcome, _) = tokio::join!(
            dispatcher.turn_ending(&ctx),
            walk_away(&mut events, &pending)
        );
        assert_eq!(outcome, None, "no next turn");

        assert!(!staging.is_empty());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.rs")).unwrap(),
            "before\n"
        );
    }

    #[test]
    fn comments_render_where_then_what_and_a_general_one_as_itself() {
        let one = vec![ReviewComment {
            path: "a.rs".into(),
            lines: (4, 4),
            text: "x".into(),
        }];
        assert_eq!(render_comments(&one), "On a.rs, line 4:\nx");
        let general = vec![ReviewComment {
            path: String::new(),
            lines: (0, 0),
            text: "and rename it".into(),
        }];
        assert_eq!(render_comments(&general), "and rename it");
    }
}
