use futures::{future, StreamExt};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::{
    client::{LlmClient, LlmRequest},
    dispatcher::{DispatchContext, ToolDispatcher},
    event::{Command, Event, LlmEvent, LogRecord, StepOutcome, TurnEndReason},
    log::ConversationLog,
    prompt,
    types::*,
};

pub struct Agent<C, D> {
    client:     C,
    dispatcher: D,
    log:        ConversationLog,
    model:      String,
    system:     String,
    // Pending Edit approval gates and permission prompts, keyed by call_id /
    // PromptId. Shared rather than owned locally by `run` so `DispatchContext`
    // — handed to a dispatch future that runs concurrently with the command
    // loop — can register into the same map the loop resolves against.
    approvals:  Arc<Mutex<HashMap<String, oneshot::Sender<bool>>>>,
    prompts:    Arc<Mutex<HashMap<u64, oneshot::Sender<serde_json::Value>>>>,
}

impl<C: LlmClient, D: ToolDispatcher> Agent<C, D> {
    pub fn new(
        client:             C,
        dispatcher:         D,
        model:              impl Into<String>,
        additional_context: Option<&str>,
    ) -> Self {
        Self {
            client,
            dispatcher,
            log: ConversationLog::new(),
            model: model.into(),
            system: prompt::compose(additional_context),
            approvals: Arc::new(Mutex::new(HashMap::new())),
            prompts:   Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn log(&self) -> &ConversationLog { &self.log }

    /// Resolve a pending Edit approval gate. Returns false if `call_id` has no
    /// pending gate (already resolved, or never registered).
    fn resolve_approval(&self, call_id: &str, approved: bool) -> bool {
        match self.approvals.lock().expect("approvals lock poisoned").remove(call_id) {
            Some(tx) => { let _ = tx.send(approved); true }
            None => false,
        }
    }

    /// Resolve a pending permission prompt. Returns false if `id` has no
    /// pending prompt (already resolved, or never registered).
    fn resolve_prompt(&self, id: u64, payload: serde_json::Value) -> bool {
        match self.prompts.lock().expect("prompts lock poisoned").remove(&id) {
            Some(tx) => { let _ = tx.send(payload); true }
            None => false,
        }
    }

    /// Drive the agent. Returns when the command channel closes.
    pub async fn run(
        mut self,
        mut commands: mpsc::Receiver<Command>,
        events:       mpsc::Sender<Event>,
    ) {
        while let Some(cmd) = commands.recv().await {
            match cmd {
                Command::Submit { text } => {
                    let turn_id = TurnId::next();
                    self.log.append(LogRecord::TurnStarted { turn_id });
                    self.log.append(LogRecord::UserMessage { turn_id, text: text.clone() });

                    let cancel = CancellationToken::new();
                    let reason = self
                        .run_turn(turn_id, text, &events, &mut commands, cancel)
                        .await;

                    self.log.append(LogRecord::TurnEnded { turn_id, reason: reason.clone() });
                    let _ = events.send(Event::TurnEnded { turn_id, reason }).await;
                }

                Command::ApproveTool { call_id } => {
                    if !self.resolve_approval(&call_id, true) {
                        warn!("ApproveTool for unknown call_id {call_id}");
                    }
                }
                Command::DenyTool { call_id } => {
                    if !self.resolve_approval(&call_id, false) {
                        warn!("DenyTool for unknown call_id {call_id}");
                    }
                }
                Command::PromptResponse { id, payload } => {
                    if !self.resolve_prompt(id.0, payload) {
                        warn!("PromptResponse for unknown prompt {}", id.0);
                    }
                }
                Command::Cancel => {} // no-op outside an active turn
            }
        }
    }

    async fn run_turn(
        &mut self,
        turn_id:   TurnId,
        user_text: String,
        events:    &mpsc::Sender<Event>,
        commands:  &mut mpsc::Receiver<Command>,
        cancel:    CancellationToken,
    ) -> TurnEndReason {
        let _ = events.send(Event::TurnStarted { turn_id }).await;

        let mut messages: Vec<Message> = self.messages_from_log();
        messages.push(Message::user(user_text));

        loop {
            let step_id = StepId::next();
            debug!("turn {turn_id:?} step {step_id:?}");

            let result = self
                .run_step(turn_id, step_id, &mut messages, events, commands, cancel.clone())
                .await;

            match result {
                StepResult::EndTurn(outcome) => {
                    self.log.append(LogRecord::StepBoundary { turn_id, step_id, outcome });
                    return TurnEndReason::EndTurn;
                }
                StepResult::ToolsDispatched { outcome, results } => {
                    self.log.append(LogRecord::StepBoundary { turn_id, step_id, outcome });
                    // Tool results go back as a user-role message (Anthropic convention).
                    let tool_result_blocks: Vec<ContentBlock> = results
                        .iter()
                        .map(|r| ContentBlock::ToolResult(r.clone()))
                        .collect();
                    messages.push(Message { role: Role::User, content: tool_result_blocks });
                    for result in results {
                        self.log.append(LogRecord::ToolResult { turn_id, step_id, result });
                    }
                    // Continue to next step.
                }
                StepResult::ToolsAborted { outcome, results, reason } => {
                    // Dispatch was cut short (cancelled, or the command channel
                    // closed) after ToolUse records were already logged for this
                    // step. Close the step out the same shape a completed round
                    // trip would have — StepBoundary, then a ToolResult for every
                    // call — so no ToolUse is ever left without a matching
                    // ToolResult; a later replay of the log stays a valid request.
                    self.log.append(LogRecord::StepBoundary { turn_id, step_id, outcome });
                    for result in results {
                        self.log.append(LogRecord::ToolResult { turn_id, step_id, result });
                    }
                    return reason;
                }
                StepResult::Cancelled  => return TurnEndReason::Cancelled,
                StepResult::Error(msg) => return TurnEndReason::Error(msg),
            }
        }
    }

    async fn run_step(
        &mut self,
        turn_id:   TurnId,
        step_id:   StepId,
        messages:  &mut Vec<Message>,
        events:    &mpsc::Sender<Event>,
        commands:  &mut mpsc::Receiver<Command>,
        cancel:    CancellationToken,
    ) -> StepResult {
        let cache_breakpoints = self.cache_breakpoints(messages);
        let tools             = self.dispatcher.definitions();

        // Clone messages so the stream's borrow doesn't block subsequent mutations.
        // The LlmClient will serialise this snapshot; any mutations we make after
        // StepEnded are for the *next* step's request.
        let messages_snap = messages.clone();

        let mut text_buf   = String::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        let mut step_outcome: Option<StepOutcome> = None;

        // Inner block: stream lives here, releasing the borrow on messages_snap at end.
        let stream_terminal: StepTerminal = {
            let request = LlmRequest {
                model:             &self.model,
                system:            &self.system,
                tools:             &tools,
                messages:          &messages_snap,
                cache_breakpoints: &cache_breakpoints,
            };
            let mut stream = self.client.stream(request);

            loop {
                tokio::select! {
                    biased;

                    _ = cancel.cancelled() => {
                        break StepTerminal::Cancelled;
                    }

                    cmd = commands.recv() => {
                        match cmd {
                            Some(Command::Cancel) => {
                                cancel.cancel();
                                break StepTerminal::Cancelled;
                            }
                            Some(Command::ApproveTool { call_id }) => { self.resolve_approval(&call_id, true); }
                            Some(Command::DenyTool { call_id })    => { self.resolve_approval(&call_id, false); }
                            Some(Command::PromptResponse { id, payload }) => { self.resolve_prompt(id.0, payload); }
                            Some(Command::Submit { .. }) => {
                                warn!("Submit received mid-turn; discarding");
                            }
                            None => break StepTerminal::Error("command channel closed".into()),
                        }
                    }

                    item = stream.next() => {
                        match item {
                            None => break StepTerminal::Error("stream closed without StepEnded".into()),
                            Some(Err(e)) => break StepTerminal::Error(e.to_string()),
                            Some(Ok(ev)) => {
                                match ev {
                                    LlmEvent::TextDelta { text } => {
                                        text_buf.push_str(&text);
                                        let _ = events.send(Event::TextDelta {
                                            turn_id, step_id, text,
                                        }).await;
                                    }
                                    LlmEvent::ThinkingStart => {
                                        let _ = events.send(Event::ThinkingStart { turn_id, step_id }).await;
                                    }
                                    LlmEvent::ThinkingEnd => {
                                        let _ = events.send(Event::ThinkingEnd { turn_id, step_id }).await;
                                    }
                                    LlmEvent::ToolUseRequested { call } => {
                                        let _ = events.send(Event::ToolUseRequested {
                                            turn_id, step_id, call: call.clone(),
                                        }).await;
                                        tool_calls.push(call);
                                    }
                                    LlmEvent::RetryAttempt { info } => {
                                        let _ = events.send(Event::RetryAttempt {
                                            turn_id, step_id, info,
                                        }).await;
                                    }
                                    LlmEvent::StepEnded { outcome } => {
                                        let _ = events.send(Event::StepEnded {
                                            turn_id, step_id, outcome: outcome.clone(),
                                        }).await;
                                        step_outcome = Some(outcome);
                                        break StepTerminal::Ok;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }; // stream dropped here

        // Commit accumulated text to log and messages before anything else.
        if !text_buf.is_empty() {
            let text = std::mem::take(&mut text_buf);
            self.log.append(LogRecord::AssistantMessage { turn_id, step_id, text: text.clone() });
            messages.push(Message {
                role:    Role::Assistant,
                content: vec![ContentBlock::Text { text }],
            });
        }

        match stream_terminal {
            StepTerminal::Cancelled  => return StepResult::Cancelled,
            StepTerminal::Error(msg) => return StepResult::Error(msg),
            StepTerminal::Ok         => {}
        }

        // Commit observed tool calls to log only once the step is known to have
        // completed — logging these before a cancellation/error check would leave
        // a ToolUse record with no matching ToolResult or StepBoundary (a torn log).
        for call in &tool_calls {
            self.log.append(LogRecord::ToolUse { turn_id, step_id, call: call.clone() });
        }

        let outcome = step_outcome.expect("StepTerminal::Ok implies StepEnded was received");

        match outcome.stop_reason {
            StopReason::EndTurn  => StepResult::EndTurn(outcome),
            StopReason::ToolUse  => {
                match self.dispatch_tools(turn_id, step_id, tool_calls, events, commands, cancel).await {
                    DispatchOutcome::Completed(results) => StepResult::ToolsDispatched { outcome, results },
                    DispatchOutcome::Aborted { results, reason } => {
                        StepResult::ToolsAborted { outcome, results, reason }
                    }
                }
            }
        }
    }

    /// Dispatch all tool calls for a step concurrently, staying responsive to
    /// commands (Cancel above all) for the whole duration — a slow or stuck
    /// tool must not make cancellation meaningless. Approval-gated tools (Edit)
    /// block inside their own future via `DispatchContext`; the agent loop
    /// just awaits.
    async fn dispatch_tools(
        &self,
        turn_id:  TurnId,
        step_id:  StepId,
        calls:    Vec<ToolCall>,
        events:   &mpsc::Sender<Event>,
        commands: &mut mpsc::Receiver<Command>,
        cancel:   CancellationToken,
    ) -> DispatchOutcome {
        for call in &calls {
            let _ = events.send(Event::ToolDispatched {
                turn_id, step_id, call_id: call.id.clone(),
            }).await;
        }

        let ctx = DispatchContext::new(
            turn_id, step_id, events.clone(), self.approvals.clone(), self.prompts.clone(),
        );

        // Drive all dispatch futures on the current task (cooperative).
        // Dispatcher impls use spawn_blocking internally for CPU-heavy work.
        let futs: Vec<_> = calls.iter()
            .map(|call| self.dispatcher.dispatch(call.clone(), &ctx))
            .collect();
        let joined = future::join_all(futs);
        tokio::pin!(joined);

        loop {
            tokio::select! {
                biased;

                _ = cancel.cancelled() => {
                    return self.abort_dispatch(turn_id, step_id, &calls, events, TurnEndReason::Cancelled).await;
                }

                cmd = commands.recv() => {
                    match cmd {
                        Some(Command::Cancel) => {
                            cancel.cancel();
                            return self.abort_dispatch(turn_id, step_id, &calls, events, TurnEndReason::Cancelled).await;
                        }
                        Some(Command::ApproveTool { call_id }) => { self.resolve_approval(&call_id, true); }
                        Some(Command::DenyTool { call_id })    => { self.resolve_approval(&call_id, false); }
                        Some(Command::PromptResponse { id, payload }) => { self.resolve_prompt(id.0, payload); }
                        Some(Command::Submit { .. }) => warn!("Submit received mid-turn; discarding"),
                        None => {
                            let reason = TurnEndReason::Error("command channel closed".into());
                            return self.abort_dispatch(turn_id, step_id, &calls, events, reason).await;
                        }
                    }
                }

                results = &mut joined => {
                    for result in &results {
                        let _ = events.send(Event::ToolCompleted {
                            turn_id, step_id, result: result.clone(),
                        }).await;
                    }
                    return DispatchOutcome::Completed(results);
                }
            }
        }
    }

    /// Close out a step whose tool dispatch was cut short (cancelled, or the
    /// command channel closed) without leaving the already-logged ToolUse
    /// records orphaned: every in-flight call gets a synthetic error
    /// ToolResult, so the step still reads as a well-formed round trip and
    /// never a torn one.
    async fn abort_dispatch(
        &self,
        turn_id: TurnId,
        step_id: StepId,
        calls:   &[ToolCall],
        events:  &mpsc::Sender<Event>,
        reason:  TurnEndReason,
    ) -> DispatchOutcome {
        // A call whose dispatch future was mid-`request_approval` leaves a
        // dangling entry here otherwise — nothing will ever resolve it once the
        // future backing its receiver has been dropped.
        {
            let mut approvals = self.approvals.lock().expect("approvals lock poisoned");
            for call in calls {
                approvals.remove(&call.id);
            }
        }

        let message = match &reason {
            TurnEndReason::Cancelled => "cancelled",
            _ => "tool dispatch aborted",
        };
        let results: Vec<ToolResult> = calls.iter()
            .map(|call| ToolResult { call_id: call.id.clone(), content: message.into(), is_error: true })
            .collect();
        for result in &results {
            let _ = events.send(Event::ToolCompleted { turn_id, step_id, result: result.clone() }).await;
        }
        DispatchOutcome::Aborted { results, reason }
    }

    /// Reconstruct the wire message list from the append-only log.
    fn messages_from_log(&self) -> Vec<Message> {
        let snap = self.log.snapshot();
        let mut messages: Vec<Message> = Vec::new();

        for record in snap.iter() {
            match record {
                LogRecord::UserMessage { text, .. } => {
                    messages.push(Message::user(text.clone()));
                }
                LogRecord::AssistantMessage { text, .. } => {
                    // Merge into the last assistant message if possible.
                    if let Some(last) = messages.last_mut() {
                        if last.role == Role::Assistant {
                            last.content.push(ContentBlock::Text { text: text.clone() });
                            continue;
                        }
                    }
                    messages.push(Message {
                        role:    Role::Assistant,
                        content: vec![ContentBlock::Text { text: text.clone() }],
                    });
                }
                LogRecord::ToolUse { call, .. } => {
                    if let Some(last) = messages.last_mut() {
                        if last.role == Role::Assistant {
                            last.content.push(ContentBlock::ToolUse(call.clone()));
                            continue;
                        }
                    }
                    messages.push(Message {
                        role:    Role::Assistant,
                        content: vec![ContentBlock::ToolUse(call.clone())],
                    });
                }
                LogRecord::ToolResult { result, .. } => {
                    if let Some(last) = messages.last_mut() {
                        if last.role == Role::User
                            && matches!(last.content.first(), Some(ContentBlock::ToolResult(_)))
                        {
                            last.content.push(ContentBlock::ToolResult(result.clone()));
                            continue;
                        }
                    }
                    messages.push(Message {
                        role:    Role::User,
                        content: vec![ContentBlock::ToolResult(result.clone())],
                    });
                }
                _ => {}
            }
        }

        messages
    }

    fn cache_breakpoints(&self, messages: &[Message]) -> Vec<usize> {
        if messages.is_empty() { return vec![]; }
        let last = messages.len() - 1;
        if last == 0 { vec![0] } else { vec![0, last] }
    }
}

// ── Internal ─────────────────────────────────────────────────────────────────

enum StepTerminal { Ok, Cancelled, Error(String) }

enum StepResult {
    EndTurn(StepOutcome),
    ToolsDispatched { outcome: StepOutcome, results: Vec<ToolResult> },
    ToolsAborted { outcome: StepOutcome, results: Vec<ToolResult>, reason: TurnEndReason },
    Cancelled,
    Error(String),
}

enum DispatchOutcome {
    Completed(Vec<ToolResult>),
    Aborted { results: Vec<ToolResult>, reason: TurnEndReason },
}

// ── Tests ────────────────────────────────────────────────────────────────────
//
// Covers the loop's real failure modes per amundsen-core.md's Pitfalls: torn logs
// on cancellation, tool round trips, and step/turn boundary bookkeeping. Not
// exhaustive by design — these are the invariants a refactor is most likely to
// break silently.

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use crate::client::LlmError;
    use futures::stream;
    use std::collections::VecDeque;
    use std::pin::Pin;
    use std::sync::Mutex;

    /// Replays one canned event script per `stream()` call, one script per step.
    struct ScriptedClient {
        scripts: Mutex<VecDeque<Vec<LlmEvent>>>,
    }

    impl ScriptedClient {
        fn new(scripts: Vec<Vec<LlmEvent>>) -> Self {
            Self { scripts: Mutex::new(scripts.into()) }
        }
    }

    impl LlmClient for ScriptedClient {
        fn stream<'a>(
            &'a self,
            _request: LlmRequest<'a>,
        ) -> Pin<Box<dyn futures::Stream<Item = Result<LlmEvent, LlmError>> + Send + 'a>> {
            let events = self.scripts.lock().unwrap().pop_front()
                .expect("ScriptedClient: no more scripted steps");
            Box::pin(stream::iter(events.into_iter().map(Ok)))
        }
    }

    /// Yields one delta then never terminates — for exercising mid-step cancellation.
    struct StallingClient;

    impl LlmClient for StallingClient {
        fn stream<'a>(
            &'a self,
            _request: LlmRequest<'a>,
        ) -> Pin<Box<dyn futures::Stream<Item = Result<LlmEvent, LlmError>> + Send + 'a>> {
            let first = stream::iter(vec![Ok(LlmEvent::TextDelta { text: "partial".into() })]);
            Box::pin(first.chain(stream::pending()))
        }
    }

    /// Yields a tool-use request then never terminates — for exercising cancellation
    /// that lands after a tool call is requested but before the step ends.
    struct StallingAfterToolUseClient;

    impl LlmClient for StallingAfterToolUseClient {
        fn stream<'a>(
            &'a self,
            _request: LlmRequest<'a>,
        ) -> Pin<Box<dyn futures::Stream<Item = Result<LlmEvent, LlmError>> + Send + 'a>> {
            let call = ToolCall { id: "t1".into(), name: "read".into(), input: serde_json::json!({}) };
            let first = stream::iter(vec![Ok(LlmEvent::ToolUseRequested { call })]);
            Box::pin(first.chain(stream::pending()))
        }
    }

    struct EchoDispatcher;

    #[async_trait]
    impl ToolDispatcher for EchoDispatcher {
        async fn dispatch(&self, call: ToolCall, _ctx: &DispatchContext) -> ToolResult {
            ToolResult { call_id: call.id, content: format!("ok:{}", call.name), is_error: false }
        }
        fn definitions(&self) -> Vec<ToolDefinition> { vec![] }
    }

    /// Never resolves — for exercising cancellation that lands while tools are
    /// actually dispatching (as opposed to mid-stream, before dispatch starts).
    struct StallingDispatcher;

    #[async_trait]
    impl ToolDispatcher for StallingDispatcher {
        async fn dispatch(&self, _call: ToolCall, _ctx: &DispatchContext) -> ToolResult {
            future::pending().await
        }
        fn definitions(&self) -> Vec<ToolDefinition> { vec![] }
    }

    /// Blocks on `DispatchContext::request_approval` — for exercising the
    /// Edit-style approval round trip end to end.
    struct ApprovalGatedDispatcher;

    #[async_trait]
    impl ToolDispatcher for ApprovalGatedDispatcher {
        async fn dispatch(&self, call: ToolCall, ctx: &DispatchContext) -> ToolResult {
            let approved = ctx.request_approval(call.id.clone(), "diff".into()).await;
            ToolResult {
                call_id:  call.id,
                content:  if approved { "approved".into() } else { "denied".into() },
                is_error: !approved,
            }
        }
        fn definitions(&self) -> Vec<ToolDefinition> { vec![] }
    }

    /// Blocks on `DispatchContext::request_prompt` — for exercising the
    /// permission-prompt round trip end to end.
    struct PromptGatedDispatcher;

    #[async_trait]
    impl ToolDispatcher for PromptGatedDispatcher {
        async fn dispatch(&self, call: ToolCall, ctx: &DispatchContext) -> ToolResult {
            let payload = ctx.request_prompt(serde_json::json!({"ask": "confirm"})).await;
            ToolResult { call_id: call.id, content: payload.to_string(), is_error: false }
        }
        fn definitions(&self) -> Vec<ToolDefinition> { vec![] }
    }

    fn outcome(stop_reason: StopReason) -> StepOutcome {
        StepOutcome {
            stop_reason,
            usage: UsageStats { input_tokens: 0, output_tokens: 0 },
            cache: CacheStats { cache_creation_input_tokens: 0, cache_read_input_tokens: 0 },
        }
    }

    #[tokio::test]
    async fn simple_turn_completes_and_logs_cleanly() {
        let client = ScriptedClient::new(vec![vec![
            LlmEvent::TextDelta { text: "hi".into() },
            LlmEvent::StepEnded { outcome: outcome(StopReason::EndTurn) },
        ]]);
        let agent = Agent::new(client, EchoDispatcher, "test-model", None);
        let log = agent.log().clone();

        let (cmd_tx, cmd_rx) = mpsc::channel(8);
        let (ev_tx, mut ev_rx) = mpsc::channel(32);
        tokio::spawn(agent.run(cmd_rx, ev_tx));

        cmd_tx.send(Command::Submit { text: "hello".into() }).await.unwrap();

        loop {
            match ev_rx.recv().await.expect("agent dropped the event channel") {
                Event::TurnEnded { reason, .. } => {
                    assert!(matches!(reason, TurnEndReason::EndTurn));
                    break;
                }
                _ => {}
            }
        }

        let snap = log.snapshot();
        assert!(matches!(snap[0], LogRecord::TurnStarted { .. }));
        assert!(matches!(snap[1], LogRecord::UserMessage { .. }));
        assert!(matches!(snap[2], LogRecord::AssistantMessage { .. }));
        assert!(matches!(snap[3], LogRecord::StepBoundary { .. }));
        assert!(matches!(snap[4], LogRecord::TurnEnded { .. }));
    }

    #[tokio::test]
    async fn tool_round_trip_dispatches_and_continues() {
        let client = ScriptedClient::new(vec![
            vec![
                LlmEvent::ToolUseRequested {
                    call: ToolCall { id: "t1".into(), name: "read".into(), input: serde_json::json!({}) },
                },
                LlmEvent::StepEnded { outcome: outcome(StopReason::ToolUse) },
            ],
            vec![
                LlmEvent::TextDelta { text: "done".into() },
                LlmEvent::StepEnded { outcome: outcome(StopReason::EndTurn) },
            ],
        ]);
        let agent = Agent::new(client, EchoDispatcher, "test-model", None);
        let log = agent.log().clone();

        let (cmd_tx, cmd_rx) = mpsc::channel(8);
        let (ev_tx, mut ev_rx) = mpsc::channel(32);
        tokio::spawn(agent.run(cmd_rx, ev_tx));

        cmd_tx.send(Command::Submit { text: "go".into() }).await.unwrap();

        let mut completed = false;
        loop {
            match ev_rx.recv().await.expect("agent dropped the event channel") {
                Event::ToolCompleted { result, .. } => {
                    assert_eq!(result.content, "ok:read");
                    completed = true;
                }
                Event::TurnEnded { .. } => break,
                _ => {}
            }
        }
        assert!(completed);

        let snap = log.snapshot();
        assert!(snap.iter().any(|r| matches!(r, LogRecord::ToolUse { .. })));
        assert!(snap.iter().any(|r| matches!(r, LogRecord::ToolResult { .. })));
        assert!(matches!(snap.last().unwrap(), LogRecord::TurnEnded { .. }));
    }

    #[tokio::test]
    async fn cancellation_leaves_well_formed_log_not_a_torn_one() {
        let agent = Agent::new(StallingClient, EchoDispatcher, "test-model", None);
        let log = agent.log().clone();

        let (cmd_tx, cmd_rx) = mpsc::channel(8);
        let (ev_tx, mut ev_rx) = mpsc::channel(32);
        tokio::spawn(agent.run(cmd_rx, ev_tx));

        cmd_tx.send(Command::Submit { text: "go".into() }).await.unwrap();

        // Wait for the partial delta so cancel lands mid-step, not before the step starts.
        loop {
            match ev_rx.recv().await.expect("agent dropped the event channel") {
                Event::TextDelta { .. } => break,
                _ => {}
            }
        }
        cmd_tx.send(Command::Cancel).await.unwrap();

        loop {
            match ev_rx.recv().await.expect("agent dropped the event channel") {
                Event::TurnEnded { reason, .. } => {
                    assert!(matches!(reason, TurnEndReason::Cancelled));
                    break;
                }
                _ => {}
            }
        }

        let snap = log.snapshot();
        assert!(snap.iter().any(|r| matches!(
            r,
            LogRecord::AssistantMessage { text, .. } if text.as_str() == "partial"
        )));
        assert!(matches!(
            snap.last().unwrap(),
            LogRecord::TurnEnded { reason: TurnEndReason::Cancelled, .. }
        ));
    }

    #[tokio::test]
    async fn cancel_after_tool_use_requested_leaves_no_orphaned_tool_use() {
        let agent = Agent::new(StallingAfterToolUseClient, EchoDispatcher, "test-model", None);
        let log = agent.log().clone();

        let (cmd_tx, cmd_rx) = mpsc::channel(8);
        let (ev_tx, mut ev_rx) = mpsc::channel(32);
        tokio::spawn(agent.run(cmd_rx, ev_tx));

        cmd_tx.send(Command::Submit { text: "go".into() }).await.unwrap();

        // Wait for the tool call to be requested so cancel lands after it, before StepEnded.
        loop {
            match ev_rx.recv().await.expect("agent dropped the event channel") {
                Event::ToolUseRequested { .. } => break,
                _ => {}
            }
        }
        cmd_tx.send(Command::Cancel).await.unwrap();

        loop {
            match ev_rx.recv().await.expect("agent dropped the event channel") {
                Event::TurnEnded { reason, .. } => {
                    assert!(matches!(reason, TurnEndReason::Cancelled));
                    break;
                }
                _ => {}
            }
        }

        // The requested tool call was never dispatched, so it must not appear in the
        // log — a logged ToolUse with no ToolResult/StepBoundary would be a torn log.
        let snap = log.snapshot();
        assert!(!snap.iter().any(|r| matches!(r, LogRecord::ToolUse { .. })));
        assert!(matches!(
            snap.last().unwrap(),
            LogRecord::TurnEnded { reason: TurnEndReason::Cancelled, .. }
        ));
    }

    #[tokio::test]
    async fn cancel_during_tool_dispatch_ends_turn_cancelled_with_well_formed_log() {
        let client = ScriptedClient::new(vec![vec![
            LlmEvent::ToolUseRequested {
                call: ToolCall { id: "t1".into(), name: "read".into(), input: serde_json::json!({}) },
            },
            LlmEvent::StepEnded { outcome: outcome(StopReason::ToolUse) },
        ]]);
        let agent = Agent::new(client, StallingDispatcher, "test-model", None);
        let log = agent.log().clone();

        let (cmd_tx, cmd_rx) = mpsc::channel(8);
        let (ev_tx, mut ev_rx) = mpsc::channel(32);
        tokio::spawn(agent.run(cmd_rx, ev_tx));

        cmd_tx.send(Command::Submit { text: "go".into() }).await.unwrap();

        // Wait until the tool is actually dispatched (not merely requested) so
        // cancel lands while dispatch_tools is awaiting it, not before.
        loop {
            match ev_rx.recv().await.expect("agent dropped the event channel") {
                Event::ToolDispatched { .. } => break,
                _ => {}
            }
        }
        cmd_tx.send(Command::Cancel).await.unwrap();

        let mut saw_tool_completed = false;
        loop {
            match ev_rx.recv().await.expect("agent dropped the event channel") {
                Event::ToolCompleted { result, .. } => {
                    assert!(result.is_error);
                    saw_tool_completed = true;
                }
                Event::TurnEnded { reason, .. } => {
                    assert!(matches!(reason, TurnEndReason::Cancelled));
                    break;
                }
                _ => {}
            }
        }
        assert!(saw_tool_completed);

        // Every ToolUse must be followed by a matching ToolResult before
        // TurnEnded — an unresolved one would make a later replay of this log
        // an invalid request to the LLM (a dangling tool_use block).
        let snap = log.snapshot();
        let tool_use_count = snap.iter().filter(|r| matches!(r, LogRecord::ToolUse { .. })).count();
        let tool_result_count = snap.iter().filter(|r| matches!(r, LogRecord::ToolResult { .. })).count();
        assert_eq!(tool_use_count, 1);
        assert_eq!(tool_result_count, 1);
        assert!(matches!(
            snap.last().unwrap(),
            LogRecord::TurnEnded { reason: TurnEndReason::Cancelled, .. }
        ));
    }

    #[tokio::test]
    async fn approval_gate_round_trips_through_approve_tool_command() {
        let client = ScriptedClient::new(vec![
            vec![
                LlmEvent::ToolUseRequested {
                    call: ToolCall { id: "t1".into(), name: "edit".into(), input: serde_json::json!({}) },
                },
                LlmEvent::StepEnded { outcome: outcome(StopReason::ToolUse) },
            ],
            vec![
                LlmEvent::TextDelta { text: "done".into() },
                LlmEvent::StepEnded { outcome: outcome(StopReason::EndTurn) },
            ],
        ]);
        let agent = Agent::new(client, ApprovalGatedDispatcher, "test-model", None);

        let (cmd_tx, cmd_rx) = mpsc::channel(8);
        let (ev_tx, mut ev_rx) = mpsc::channel(32);
        tokio::spawn(agent.run(cmd_rx, ev_tx));

        cmd_tx.send(Command::Submit { text: "go".into() }).await.unwrap();

        loop {
            match ev_rx.recv().await.expect("agent dropped the event channel") {
                Event::ToolApprovalRequested { call_id, .. } => {
                    cmd_tx.send(Command::ApproveTool { call_id }).await.unwrap();
                    break;
                }
                _ => {}
            }
        }

        let mut saw_approved = false;
        loop {
            match ev_rx.recv().await.expect("agent dropped the event channel") {
                Event::ToolCompleted { result, .. } => {
                    assert_eq!(result.content, "approved");
                    saw_approved = true;
                }
                Event::TurnEnded { reason, .. } => {
                    assert!(matches!(reason, TurnEndReason::EndTurn));
                    break;
                }
                _ => {}
            }
        }
        assert!(saw_approved);
    }

    #[tokio::test]
    async fn prompt_gate_round_trips_through_prompt_response_command() {
        let client = ScriptedClient::new(vec![
            vec![
                LlmEvent::ToolUseRequested {
                    call: ToolCall { id: "t1".into(), name: "risky".into(), input: serde_json::json!({}) },
                },
                LlmEvent::StepEnded { outcome: outcome(StopReason::ToolUse) },
            ],
            vec![LlmEvent::StepEnded { outcome: outcome(StopReason::EndTurn) }],
        ]);
        let agent = Agent::new(client, PromptGatedDispatcher, "test-model", None);

        let (cmd_tx, cmd_rx) = mpsc::channel(8);
        let (ev_tx, mut ev_rx) = mpsc::channel(32);
        tokio::spawn(agent.run(cmd_rx, ev_tx));

        cmd_tx.send(Command::Submit { text: "go".into() }).await.unwrap();

        loop {
            match ev_rx.recv().await.expect("agent dropped the event channel") {
                Event::PromptRequested { id, .. } => {
                    cmd_tx.send(Command::PromptResponse { id, payload: serde_json::json!("yes") })
                        .await.unwrap();
                    break;
                }
                _ => {}
            }
        }

        let mut saw_result = false;
        loop {
            match ev_rx.recv().await.expect("agent dropped the event channel") {
                Event::ToolCompleted { result, .. } => {
                    assert_eq!(result.content, "\"yes\"");
                    saw_result = true;
                }
                Event::TurnEnded { reason, .. } => {
                    assert!(matches!(reason, TurnEndReason::EndTurn));
                    break;
                }
                _ => {}
            }
        }
        assert!(saw_result);
    }
}
