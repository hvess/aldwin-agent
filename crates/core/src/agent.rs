use futures::{future, StreamExt};
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

use crate::{
    client::{LlmClient, LlmRequest},
    dispatcher::{DispatchContext, PendingMap, PendingReply, ToolDispatcher},
    event::{Command, Event, Failure, LlmEvent, LogRecord, StepOutcome, TurnEndReason},
    log::{ConversationLog, RecordSink},
    prompt,
    types::*,
};

/// The conversation loop, one per session: streams each step from `C` and
/// hands its tool calls to `D`.
pub struct Agent<C, D> {
    client: C,
    dispatcher: D,
    log: ConversationLog,
    system: String,
    // Shared with every `DispatchContext`: dispatch futures register while
    // the command loop resolves, concurrently. See `PendingReply`.
    pending: PendingMap,
    /// The last turn and step ids minted; only the agent mints them. A resume
    /// moves them past the highest id it carries (`continue_ids`).
    last_turn: u64,
    last_step: u64,
}

/// By hand: `C` and `D` need not be `Debug`, and the system prompt is omitted.
impl<C, D> fmt::Debug for Agent<C, D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Agent")
            .field("log", &self.log)
            .field("pending", &self.pending)
            .field("last_turn", &self.last_turn)
            .field("last_step", &self.last_step)
            .finish_non_exhaustive()
    }
}

impl<C: LlmClient, D: ToolDispatcher> Agent<C, D> {
    /// An agent with an in-memory log; the system prompt is
    /// `prompt::compose(additional_context)`.
    pub fn new(client: C, dispatcher: D, additional_context: Option<&str>) -> Self {
        Self {
            client,
            dispatcher,
            log: ConversationLog::new(),
            system: prompt::compose(additional_context),
            pending: Arc::new(Mutex::new(HashMap::new())),
            last_turn: 0,
            last_step: 0,
        }
    }

    #[cfg(test)]
    fn log(&self) -> &ConversationLog {
        &self.log
    }

    /// Writes this session's records through to `sink` as well as memory.
    pub fn with_sink(mut self, sink: Arc<dyn RecordSink>) -> Self {
        self.log = ConversationLog::with_sink(sink);
        self
    }

    fn next_turn(&mut self) -> TurnId {
        self.last_turn += 1;
        TurnId(self.last_turn)
    }

    fn next_step(&mut self) -> StepId {
        self.last_step += 1;
        StepId(self.last_step)
    }

    /// Moves the counters past every id in `records`, so ids are not reused
    /// after a resume.
    fn continue_ids(&mut self, records: &[LogRecord]) {
        for record in records {
            let (turn, step) = record.ids();
            self.last_turn = self.last_turn.max(turn.0);
            if let Some(step) = step {
                self.last_step = self.last_step.max(step.0);
            }
        }
    }

    /// Resolves a pending question. An entry of the other kind is put back,
    /// never destroyed.
    fn resolve_answer(&self, call_id: &str, answer: Answer) {
        let mut pending = self.pending.lock().expect("pending lock poisoned");
        match pending.remove(call_id) {
            Some(PendingReply::Answer(tx)) => {
                let _ = tx.send(answer);
            }
            Some(other) => {
                pending.insert(call_id.to_string(), other);
            }
            None => {}
        }
    }

    /// Resolves a pending review; see `resolve_answer`.
    fn resolve_review(&self, review_id: &str, decision: ReviewDecision) {
        let mut pending = self.pending.lock().expect("pending lock poisoned");
        match pending.remove(review_id) {
            Some(PendingReply::Review(tx)) => {
                let _ = tx.send(decision);
            }
            Some(other) => {
                pending.insert(review_id.to_string(), other);
            }
            None => {}
        }
    }

    /// Handles a command that arrived mid-turn. Returns `true` for `Cancel`;
    /// a command that would change the history is discarded with a `Notice`.
    async fn on_mid_turn_command(&self, cmd: Command, events: &mpsc::Sender<Event>) -> bool {
        let discarded = match cmd {
            Command::Cancel => return true,
            Command::Answer { call_id, answer } => {
                self.resolve_answer(&call_id, answer);
                None
            }
            Command::ReviewDecision {
                review_id,
                decision,
            } => {
                self.resolve_review(&review_id, decision);
                None
            }
            Command::Submit { .. } => Some("that message was not sent"),
            Command::ClearHistory => Some("the conversation was not cleared"),
            Command::Resume { .. } => Some("nothing was resumed"),
        };
        // Must be said: the TUI has already drawn the submission.
        if let Some(what) = discarded {
            let _ = events
                .send(Event::Notice {
                    message: format!("A turn is running, so {what}. Stop it with esc first."),
                })
                .await;
        }
        false
    }

    /// Drives the agent until the command channel closes.
    pub async fn run(mut self, mut commands: mpsc::Receiver<Command>, events: mpsc::Sender<Event>) {
        while let Some(cmd) = commands.recv().await {
            match cmd {
                Command::Submit { text } => {
                    // Review comments from `turn_ending` start the next turn
                    // untyped (ADR 0009 §4).
                    let mut next = Some(text);
                    let mut typed = true;
                    while let Some(text) = next.take() {
                        let turn_id = self.next_turn();
                        if !typed {
                            let _ = events
                                .send(Event::FollowUp {
                                    turn_id,
                                    text: text.clone(),
                                })
                                .await;
                        }
                        typed = false;
                        self.log.append(LogRecord::TurnStarted { turn_id });
                        self.log.append(LogRecord::UserMessage { turn_id, text });

                        let (reason, follow_up) =
                            self.run_turn(turn_id, &events, &mut commands).await;

                        self.log.append(LogRecord::TurnEnded {
                            turn_id,
                            reason: reason.clone(),
                        });
                        let _ = events.send(Event::TurnEnded { turn_id, reason }).await;
                        next = follow_up;
                    }
                }

                Command::Answer { call_id, answer } => self.resolve_answer(&call_id, answer),
                Command::ReviewDecision {
                    review_id,
                    decision,
                } => self.resolve_review(&review_id, decision),
                Command::Cancel => {} // no-op outside an active turn

                Command::ClearHistory => {
                    self.log.clear();
                    let _ = events.send(Event::HistoryCleared).await;
                }

                // `replace`, not `append`: see `ConversationLog::replace`.
                Command::Resume { session, records } => {
                    self.continue_ids(&records);
                    self.log.replace(&session, records.clone());
                    let turns = records
                        .iter()
                        .filter(|r| matches!(r, LogRecord::TurnStarted { .. }))
                        .count();
                    let _ = events.send(Event::HistoryLoaded { records }).await;
                    let _ = events
                        .send(Event::Notice {
                            message: match turns {
                                1 => "Resumed the conversation: 1 turn restored.".to_string(),
                                n => format!("Resumed the conversation: {n} turns restored."),
                            },
                        })
                        .await;
                }
            }
        }
    }

    /// Runs one turn to its end. The second half of the result is the
    /// follow-up from `ToolDispatcher::turn_ending`, which starts a new turn.
    async fn run_turn(
        &mut self,
        turn_id: TurnId,
        events: &mpsc::Sender<Event>,
        commands: &mut mpsc::Receiver<Command>,
    ) -> (TurnEndReason, Option<String>) {
        let _ = events.send(Event::TurnStarted { turn_id }).await;

        // The user message is already in the log; pushing it again would send
        // it twice.
        let mut messages = self.messages_from_log();

        loop {
            let step_id = self.next_step();

            let result = self
                .run_step(turn_id, step_id, &mut messages, events, commands)
                .await;

            match result {
                StepResult::EndTurn(outcome) => {
                    self.log.append(LogRecord::StepBoundary {
                        turn_id,
                        step_id,
                        outcome,
                    });
                    // The dispatcher's moment to open the review before the
                    // turn ends (ADR 0009 §4). Must stay cancellable.
                    let ctx = DispatchContext::new(
                        turn_id,
                        step_id,
                        events.clone(),
                        self.pending.clone(),
                    );
                    let ending = self.dispatcher.turn_ending(&ctx);
                    return match self.await_or_cancel(ending, events, commands).await {
                        Ok(follow_up) => (TurnEndReason::EndTurn, follow_up),
                        Err(reason) => {
                            DispatchContext::clear_reviews(&self.pending);
                            (reason, None)
                        }
                    };
                }
                StepResult::ToolsDispatched { outcome, results } => {
                    self.log.append(LogRecord::StepBoundary {
                        turn_id,
                        step_id,
                        outcome,
                    });
                    // Tool results travel as a user-role message.
                    let tool_result_blocks: Vec<ContentBlock> = results
                        .iter()
                        .map(|r| ContentBlock::ToolResult(r.clone()))
                        .collect();
                    messages.push(Message {
                        role: Role::User,
                        content: tool_result_blocks,
                    });
                    for result in results {
                        self.log.append(LogRecord::ToolResult {
                            turn_id,
                            step_id,
                            result,
                        });
                    }
                }
                StepResult::ToolsAborted {
                    outcome,
                    results,
                    reason,
                } => {
                    // ToolUse records are already logged: each needs its
                    // ToolResult, or every later replay is an invalid request.
                    self.log.append(LogRecord::StepBoundary {
                        turn_id,
                        step_id,
                        outcome,
                    });
                    for result in results {
                        self.log.append(LogRecord::ToolResult {
                            turn_id,
                            step_id,
                            result,
                        });
                    }
                    return (reason, None);
                }
                StepResult::Cancelled => return (TurnEndReason::Cancelled, None),
                StepResult::Error(failure) => return (TurnEndReason::Error(failure), None),
            }
        }
    }

    /// Drives `fut` while handling commands, `Cancel` first. `Err` is the
    /// reason the turn ends instead.
    async fn await_or_cancel<T>(
        &self,
        fut: impl std::future::Future<Output = T>,
        events: &mpsc::Sender<Event>,
        commands: &mut mpsc::Receiver<Command>,
    ) -> std::result::Result<T, TurnEndReason> {
        tokio::pin!(fut);
        loop {
            tokio::select! {
                biased;

                cmd = commands.recv() => {
                    match cmd {
                        Some(cmd) => {
                            if self.on_mid_turn_command(cmd, events).await {
                                return Err(TurnEndReason::Cancelled);
                            }
                        }
                        None => return Err(TurnEndReason::Error(Failure::other("command channel closed"))),
                    }
                }

                value = &mut fut => return Ok(value),
            }
        }
    }

    async fn run_step(
        &mut self,
        turn_id: TurnId,
        step_id: StepId,
        messages: &mut Vec<Message>,
        events: &mpsc::Sender<Event>,
        commands: &mut mpsc::Receiver<Command>,
    ) -> StepResult {
        let tools = self.dispatcher.definitions();

        let mut text_buf = String::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        // In arrival order: the provider wants interleaved thinking and text
        // back as emitted, so never sort thinking to the front.
        let mut content: Vec<ContentBlock> = Vec::new();

        // Inner block: the stream's borrow of `messages` ends with it.
        let stream_terminal: StepTerminal = {
            let request = LlmRequest {
                system: &self.system,
                tools: &tools,
                messages: messages.as_slice(),
                cache_breakpoint: messages.len().checked_sub(1),
            };
            let mut stream = self.client.stream(request);

            loop {
                tokio::select! {
                    biased;

                    cmd = commands.recv() => {
                        match cmd {
                            Some(cmd) => if self.on_mid_turn_command(cmd, events).await { break StepTerminal::Cancelled },
                            None => break StepTerminal::Error(Failure::other("command channel closed")),
                        }
                    }

                    item = stream.next() => {
                        match item {
                            None => break StepTerminal::Error(Failure::other("stream closed without StepEnded")),
                            Some(Err(e)) => break StepTerminal::Error(e.into()),
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
                                    LlmEvent::ThinkingDelta { text } => {
                                        let _ = events.send(Event::ThinkingDelta {
                                            turn_id, step_id, text,
                                        }).await;
                                    }
                                    LlmEvent::ThinkingEnd { text, signature } => {
                                        self.flush_text(turn_id, step_id, &mut text_buf, &mut content);
                                        content.push(ContentBlock::Thinking {
                                            text: text.clone(), signature: signature.clone(),
                                        });
                                        self.log.append(LogRecord::Thinking {
                                            turn_id, step_id, text, signature,
                                        });
                                        let _ = events.send(Event::ThinkingEnd { turn_id, step_id }).await;
                                    }
                                    LlmEvent::RedactedThinking { data } => {
                                        self.flush_text(turn_id, step_id, &mut text_buf, &mut content);
                                        content.push(ContentBlock::RedactedThinking { data: data.clone() });
                                        self.log.append(LogRecord::RedactedThinking { turn_id, step_id, data });
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
                                        break StepTerminal::Ok(outcome);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }; // stream dropped here

        // Before the terminal check: a cancelled or failed step keeps what it said.
        self.flush_text(turn_id, step_id, &mut text_buf, &mut content);
        let produced_text = content
            .iter()
            .any(|b| matches!(b, ContentBlock::Text { .. }));
        if !content.is_empty() {
            messages.push(Message {
                role: Role::Assistant,
                content,
            });
        }

        let outcome = match stream_terminal {
            StepTerminal::Cancelled => return StepResult::Cancelled,
            StepTerminal::Error(failure) => return StepResult::Error(failure),
            StepTerminal::Ok(outcome) => outcome,
        };

        // Log a ToolUse only if it will be dispatched: one without a ToolResult
        // tears the log and the provider rejects every later request. A
        // `max_tokens` stop after a tool_use block arrives here as `EndTurn`.
        //
        // Must be said: the TUI already drew the call, and a step with text
        // skips the silent-turn notice below.
        if outcome.stop_reason != StopReason::ToolUse && !tool_calls.is_empty() {
            let dropped = tool_calls.len();
            tool_calls.clear();
            let _ = events
                .send(Event::Notice {
                    message: format!(
                        "the reply stopped before {dropped} requested tool call(s) could run"
                    ),
                })
                .await;
        }

        // Also into live `messages`: the provider rejects a ToolResult whose
        // preceding assistant message lacks the ToolUse, even with no text.
        for call in &tool_calls {
            self.log.append(LogRecord::ToolUse {
                turn_id,
                step_id,
                call: call.clone(),
            });
            push_assistant_block(messages, ContentBlock::ToolUse(call.clone()));
        }

        // ADR 0006: a turn with nothing to render says so instead of looking hung.
        if !produced_text && tool_calls.is_empty() {
            let _ = events
                .send(Event::Notice {
                    message: "the agent ended the turn without a reply".into(),
                })
                .await;
        }

        // A `ToolUse` stop with no call ends the turn: an empty tool-result
        // message would be rejected.
        if tool_calls.is_empty() {
            return StepResult::EndTurn(outcome);
        }
        match self
            .dispatch_tools(turn_id, step_id, tool_calls, events, commands)
            .await
        {
            DispatchOutcome::Completed(results) => StepResult::ToolsDispatched { outcome, results },
            DispatchOutcome::Aborted { results, reason } => StepResult::ToolsAborted {
                outcome,
                results,
                reason,
            },
        }
    }

    /// Commits the buffered text as one block, logging it immediately so a
    /// crash cannot lose it and replay keeps the order.
    fn flush_text(
        &self,
        turn_id: TurnId,
        step_id: StepId,
        text_buf: &mut String,
        content: &mut Vec<ContentBlock>,
    ) {
        if text_buf.is_empty() {
            return;
        }
        let text = std::mem::take(text_buf);
        self.log.append(LogRecord::AssistantMessage {
            turn_id,
            step_id,
            text: text.clone(),
        });
        content.push(ContentBlock::Text { text });
    }

    /// Dispatches a step's calls concurrently, handling commands throughout
    /// so a stuck tool cannot block `Cancel`.
    async fn dispatch_tools(
        &self,
        turn_id: TurnId,
        step_id: StepId,
        calls: Vec<ToolCall>,
        events: &mpsc::Sender<Event>,
        commands: &mut mpsc::Receiver<Command>,
    ) -> DispatchOutcome {
        let ctx = DispatchContext::new(turn_id, step_id, events.clone(), self.pending.clone());

        // ADR 0009 §4: a staged changeset these calls would observe is
        // reviewed first. A reason back answers every call without running it.
        let before = self.dispatcher.before_step(&calls, &ctx);
        match self.await_or_cancel(before, events, commands).await {
            Ok(None) => {}
            Ok(Some(reason)) => {
                let results: Vec<ToolResult> = calls
                    .iter()
                    .map(|call| ToolResult {
                        call_id: call.id.clone(),
                        content: reason.clone(),
                        is_error: true,
                    })
                    .collect();
                for result in &results {
                    let _ = events
                        .send(Event::ToolCompleted {
                            turn_id,
                            step_id,
                            result: result.clone(),
                        })
                        .await;
                }
                return DispatchOutcome::Completed(results);
            }
            Err(reason) => {
                return self
                    .abort_dispatch(turn_id, step_id, &calls, events, reason)
                    .await
            }
        }

        for call in &calls {
            let _ = events
                .send(Event::ToolDispatched {
                    turn_id,
                    step_id,
                    call_id: call.id.clone(),
                })
                .await;
        }

        // One task, cooperative: dispatchers must `spawn_blocking` heavy work.
        let futs: Vec<_> = calls
            .iter()
            .map(|call| self.dispatcher.dispatch(call.clone(), &ctx))
            .collect();
        let joined = future::join_all(futs);
        tokio::pin!(joined);

        loop {
            tokio::select! {
                biased;

                cmd = commands.recv() => {
                    let reason = match cmd {
                        Some(cmd) => {
                            if !self.on_mid_turn_command(cmd, events).await { continue; }
                            TurnEndReason::Cancelled
                        }
                        None => TurnEndReason::Error(Failure::other("command channel closed")),
                    };
                    return self.abort_dispatch(turn_id, step_id, &calls, events, reason).await;
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

    /// Closes out a step whose dispatch was cut short: every call gets a
    /// synthetic error ToolResult, so no logged ToolUse is orphaned.
    async fn abort_dispatch(
        &self,
        turn_id: TurnId,
        step_id: StepId,
        calls: &[ToolCall],
        events: &mpsc::Sender<Event>,
        reason: TurnEndReason,
    ) -> DispatchOutcome {
        // Nothing else removes an `ask` entry once its future is dropped.
        // Reviews are not keyed by call id, so they are cleared separately.
        {
            let mut pending = self.pending.lock().expect("pending lock poisoned");
            for call in calls {
                pending.remove(&call.id);
            }
        }
        DispatchContext::clear_reviews(&self.pending);

        let message = match &reason {
            TurnEndReason::Cancelled => "cancelled",
            _ => "tool dispatch aborted",
        };
        let results: Vec<ToolResult> = calls
            .iter()
            .map(|call| ToolResult {
                call_id: call.id.clone(),
                content: message.into(),
                is_error: true,
            })
            .collect();
        for result in &results {
            let _ = events
                .send(Event::ToolCompleted {
                    turn_id,
                    step_id,
                    result: result.clone(),
                })
                .await;
        }
        DispatchOutcome::Aborted { results, reason }
    }

    /// Rebuilds the message list from the log. Log order is the provider's
    /// emission order, which it wants back (ADR 0006 §2).
    fn messages_from_log(&self) -> Vec<Message> {
        let mut messages: Vec<Message> = Vec::new();

        for record in self.log.snapshot().iter() {
            match record {
                LogRecord::UserMessage { text, .. } => messages.push(Message::user(text.clone())),
                LogRecord::AssistantMessage { text, .. } => {
                    push_assistant_block(&mut messages, ContentBlock::Text { text: text.clone() });
                }
                LogRecord::Thinking {
                    text, signature, ..
                } => {
                    let block = ContentBlock::Thinking {
                        text: text.clone(),
                        signature: signature.clone(),
                    };
                    push_assistant_block(&mut messages, block);
                }
                LogRecord::RedactedThinking { data, .. } => {
                    push_assistant_block(
                        &mut messages,
                        ContentBlock::RedactedThinking { data: data.clone() },
                    );
                }
                LogRecord::ToolUse { call, .. } => {
                    push_assistant_block(&mut messages, ContentBlock::ToolUse(call.clone()));
                }
                LogRecord::ToolResult { result, .. } => {
                    let block = ContentBlock::ToolResult(result.clone());
                    match messages.last_mut() {
                        Some(last)
                            if last.role == Role::User
                                && matches!(
                                    last.content.first(),
                                    Some(ContentBlock::ToolResult(_))
                                ) =>
                        {
                            last.content.push(block);
                        }
                        _ => messages.push(Message {
                            role: Role::User,
                            content: vec![block],
                        }),
                    }
                }
                LogRecord::TurnStarted { .. }
                | LogRecord::StepBoundary { .. }
                | LogRecord::TurnEnded { .. } => {}
            }
        }

        messages
    }
}

/// Appends to the last assistant message, or opens one: a step's blocks
/// form one assistant message.
fn push_assistant_block(messages: &mut Vec<Message>, block: ContentBlock) {
    match messages.last_mut() {
        Some(last) if last.role == Role::Assistant => last.content.push(block),
        _ => messages.push(Message {
            role: Role::Assistant,
            content: vec![block],
        }),
    }
}

enum StepTerminal {
    Ok(StepOutcome),
    Cancelled,
    Error(Failure),
}

enum StepResult {
    EndTurn(StepOutcome),
    ToolsDispatched {
        outcome: StepOutcome,
        results: Vec<ToolResult>,
    },
    ToolsAborted {
        outcome: StepOutcome,
        results: Vec<ToolResult>,
        reason: TurnEndReason,
    },
    Cancelled,
    Error(Failure),
}

enum DispatchOutcome {
    Completed(Vec<ToolResult>),
    Aborted {
        results: Vec<ToolResult>,
        reason: TurnEndReason,
    },
}

// Pins the failure modes in `docs/spec/archive/aldwin-core.md` Pitfalls:
// torn logs on cancellation, tool round trips, step/turn bookkeeping.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::LlmError;
    use async_trait::async_trait;
    use futures::stream;
    use std::collections::VecDeque;
    use std::pin::Pin;
    use std::sync::Mutex;

    /// Replays one script per `stream()` call and records each request's
    /// `messages`.
    struct ScriptedClient {
        scripts: Mutex<VecDeque<Vec<LlmEvent>>>,
        // `Arc` so a test keeps a handle after `self` moves into the agent.
        seen_messages: Arc<Mutex<Vec<Vec<Message>>>>,
    }

    impl ScriptedClient {
        fn new(scripts: Vec<Vec<LlmEvent>>) -> Self {
            Self {
                scripts: Mutex::new(scripts.into()),
                seen_messages: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn seen_messages_handle(&self) -> Arc<Mutex<Vec<Vec<Message>>>> {
            self.seen_messages.clone()
        }
    }

    impl LlmClient for ScriptedClient {
        fn stream<'a>(
            &'a self,
            request: LlmRequest<'a>,
        ) -> Pin<Box<dyn futures::Stream<Item = Result<LlmEvent, LlmError>> + Send + 'a>> {
            self.seen_messages
                .lock()
                .unwrap()
                .push(request.messages.to_vec());
            let events = self
                .scripts
                .lock()
                .unwrap()
                .pop_front()
                .expect("ScriptedClient: no more scripted steps");
            Box::pin(stream::iter(events.into_iter().map(Ok)))
        }
    }

    /// Yields one delta then never terminates.
    struct StallingClient;

    impl LlmClient for StallingClient {
        fn stream<'a>(
            &'a self,
            _request: LlmRequest<'a>,
        ) -> Pin<Box<dyn futures::Stream<Item = Result<LlmEvent, LlmError>> + Send + 'a>> {
            let first = stream::iter(vec![Ok(LlmEvent::TextDelta {
                text: "partial".into(),
            })]);
            Box::pin(first.chain(stream::pending()))
        }
    }

    /// Yields a tool-use request then never terminates.
    struct StallingAfterToolUseClient;

    impl LlmClient for StallingAfterToolUseClient {
        fn stream<'a>(
            &'a self,
            _request: LlmRequest<'a>,
        ) -> Pin<Box<dyn futures::Stream<Item = Result<LlmEvent, LlmError>> + Send + 'a>> {
            let call = ToolCall {
                id: "t1".into(),
                name: "read".into(),
                input: serde_json::json!({}),
            };
            let first = stream::iter(vec![Ok(LlmEvent::ToolUseRequested { call })]);
            Box::pin(first.chain(stream::pending()))
        }
    }

    struct EchoDispatcher;

    #[async_trait]
    impl ToolDispatcher for EchoDispatcher {
        async fn dispatch(&self, call: ToolCall, _ctx: &DispatchContext) -> ToolResult {
            ToolResult {
                call_id: call.id,
                content: format!("ok:{}", call.name),
                is_error: false,
            }
        }
        fn definitions(&self) -> Vec<ToolDefinition> {
            vec![]
        }
    }

    /// Never resolves: cancellation during dispatch, not mid-stream.
    struct StallingDispatcher;

    #[async_trait]
    impl ToolDispatcher for StallingDispatcher {
        async fn dispatch(&self, _call: ToolCall, _ctx: &DispatchContext) -> ToolResult {
            future::pending().await
        }
        fn definitions(&self) -> Vec<ToolDefinition> {
            vec![]
        }
    }

    /// Opens a review from `turn_ending`.
    struct ReviewingDispatcher;

    #[async_trait]
    impl ToolDispatcher for ReviewingDispatcher {
        async fn dispatch(&self, call: ToolCall, _ctx: &DispatchContext) -> ToolResult {
            ToolResult {
                call_id: call.id,
                content: "staged".into(),
                is_error: false,
            }
        }
        fn definitions(&self) -> Vec<ToolDefinition> {
            vec![]
        }
        async fn turn_ending(&self, ctx: &DispatchContext) -> Option<String> {
            let changeset = Changeset {
                files: vec![ChangedFile {
                    path: "f.rs".into(),
                    before: Some("a".into()),
                    after: "b".into(),
                }],
            };
            match ctx.review(changeset).await? {
                ReviewDecision::Approve => {
                    ctx.review_closed(ReviewOutcome::Saved {
                        files: vec!["f.rs".into()],
                        comments_resolved: 0,
                    })
                    .await;
                    None
                }
                ReviewDecision::Comment { comments } => Some(
                    comments
                        .into_iter()
                        .map(|c| c.text)
                        .collect::<Vec<_>>()
                        .join("\n"),
                ),
                ReviewDecision::Discard => None,
            }
        }
    }

    /// Refuses every step from `before_step`.
    struct RefusingDispatcher;

    #[async_trait]
    impl ToolDispatcher for RefusingDispatcher {
        async fn dispatch(&self, _call: ToolCall, _ctx: &DispatchContext) -> ToolResult {
            panic!("a refused step must not dispatch")
        }
        fn definitions(&self) -> Vec<ToolDefinition> {
            vec![]
        }
        async fn before_step(&self, _calls: &[ToolCall], _ctx: &DispatchContext) -> Option<String> {
            Some("the developer commented first".into())
        }
    }

    /// Blocks on `DispatchContext::ask`.
    struct AskingDispatcher;

    #[async_trait]
    impl ToolDispatcher for AskingDispatcher {
        async fn dispatch(&self, call: ToolCall, ctx: &DispatchContext) -> ToolResult {
            let question = Question {
                question: "Limit anonymous requests too?".into(),
                detail: "why".into(),
                options: vec!["Yes".into(), "No".into(), "Chat about this".into()],
            };
            let answer = ctx.ask(call.id.clone(), question).await;
            ToolResult {
                call_id: call.id,
                content: format!("{answer:?}"),
                is_error: false,
            }
        }
        fn definitions(&self) -> Vec<ToolDefinition> {
            vec![]
        }
    }

    fn outcome(stop_reason: StopReason) -> StepOutcome {
        StepOutcome {
            stop_reason,
            usage: UsageStats {
                input_tokens: 0,
                output_tokens: 0,
            },
            cache: CacheStats {
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            },
        }
    }

    #[tokio::test]
    async fn simple_turn_completes_and_logs_cleanly() {
        let client = ScriptedClient::new(vec![vec![
            LlmEvent::TextDelta { text: "hi".into() },
            LlmEvent::StepEnded {
                outcome: outcome(StopReason::EndTurn),
            },
        ]]);
        let agent = Agent::new(client, EchoDispatcher, None);
        let log = agent.log().clone();

        let (cmd_tx, cmd_rx) = mpsc::channel(8);
        let (ev_tx, mut ev_rx) = mpsc::channel(32);
        tokio::spawn(agent.run(cmd_rx, ev_tx));

        cmd_tx
            .send(Command::Submit {
                text: "hello".into(),
            })
            .await
            .unwrap();

        loop {
            if let Event::TurnEnded { reason, .. } =
                ev_rx.recv().await.expect("agent dropped the event channel")
            {
                assert!(matches!(reason, TurnEndReason::EndTurn));
                break;
            }
        }

        let snap = log.snapshot();
        assert!(matches!(snap[0], LogRecord::TurnStarted { .. }));
        assert!(matches!(snap[1], LogRecord::UserMessage { .. }));
        assert!(matches!(snap[2], LogRecord::AssistantMessage { .. }));
        assert!(matches!(snap[3], LogRecord::StepBoundary { .. }));
        assert!(matches!(snap[4], LogRecord::TurnEnded { .. }));
    }

    /// ADR 0006. Regression: a thinking-only step committed nothing and
    /// rendered blank.
    #[tokio::test]
    async fn a_thinking_only_step_is_neither_dropped_nor_silent() {
        let client = ScriptedClient::new(vec![vec![
            LlmEvent::ThinkingStart,
            LlmEvent::ThinkingDelta {
                text: "weighing it up".into(),
            },
            LlmEvent::ThinkingEnd {
                text: "weighing it up".into(),
                signature: "sig-1".into(),
            },
            LlmEvent::StepEnded {
                outcome: outcome(StopReason::EndTurn),
            },
        ]]);
        let agent = Agent::new(client, EchoDispatcher, None);
        let log = agent.log().clone();

        let (cmd_tx, cmd_rx) = mpsc::channel(8);
        let (ev_tx, mut ev_rx) = mpsc::channel(32);
        tokio::spawn(agent.run(cmd_rx, ev_tx));
        cmd_tx
            .send(Command::Submit {
                text: "hello".into(),
            })
            .await
            .unwrap();

        let mut noticed = false;
        loop {
            match ev_rx.recv().await.expect("agent dropped the event channel") {
                Event::Notice { .. } => noticed = true,
                Event::TurnEnded { .. } => break,
                _ => {}
            }
        }

        // The thinking survived into the transcript...
        let snap = log.snapshot();
        assert!(
            snap.iter()
                .any(|r| matches!(r, LogRecord::Thinking { text, signature, .. }
                                          if text == "weighing it up" && signature == "sig-1")),
            "thinking must be persisted: {snap:?}"
        );
        // ...and the developer was told there was no reply.
        assert!(noticed, "a turn with no visible output must say so");
    }

    /// The provider rejects the next request unless thinking precedes its
    /// tool call in the same assistant message.
    #[tokio::test]
    async fn thinking_goes_back_ahead_of_the_tool_call_it_preceded() {
        let call = ToolCall {
            id: "t1".into(),
            name: "read".into(),
            input: serde_json::json!({}),
        };
        let client = ScriptedClient::new(vec![
            vec![
                LlmEvent::ThinkingStart,
                LlmEvent::ThinkingEnd {
                    text: "first, read it".into(),
                    signature: "sig-1".into(),
                },
                LlmEvent::TextDelta {
                    text: "Reading it now.".into(),
                },
                LlmEvent::ToolUseRequested { call },
                LlmEvent::StepEnded {
                    outcome: outcome(StopReason::ToolUse),
                },
            ],
            vec![
                LlmEvent::TextDelta {
                    text: "done".into(),
                },
                LlmEvent::StepEnded {
                    outcome: outcome(StopReason::EndTurn),
                },
            ],
        ]);
        let seen_messages = client.seen_messages_handle();
        let agent = Agent::new(client, EchoDispatcher, None);

        let (cmd_tx, cmd_rx) = mpsc::channel(8);
        let (ev_tx, mut ev_rx) = mpsc::channel(32);
        tokio::spawn(agent.run(cmd_rx, ev_tx));
        cmd_tx
            .send(Command::Submit {
                text: "hello".into(),
            })
            .await
            .unwrap();
        loop {
            if let Event::TurnEnded { .. } =
                ev_rx.recv().await.expect("agent dropped the event channel")
            {
                break;
            }
        }

        let seen = seen_messages.lock().unwrap();
        let second = &seen[1];
        let assistant = second
            .iter()
            .find(|m| m.role == Role::Assistant)
            .expect("an assistant message");
        assert!(
            matches!(assistant.content.first(), Some(ContentBlock::Thinking { signature, .. }) if signature == "sig-1"),
            "thinking must come first: {:?}",
            assistant.content
        );
        assert!(
            matches!(assistant.content.last(), Some(ContentBlock::ToolUse(_))),
            "the tool call still comes last: {:?}",
            assistant.content
        );
    }

    /// Interleaved text and thinking go back in emission order, live and on
    /// replay.
    #[tokio::test]
    async fn interleaved_blocks_keep_the_order_they_arrived_in() {
        let script = vec![
            LlmEvent::ThinkingStart,
            LlmEvent::ThinkingEnd {
                text: "one".into(),
                signature: "s1".into(),
            },
            LlmEvent::TextDelta {
                text: "between".into(),
            },
            LlmEvent::ThinkingStart,
            LlmEvent::ThinkingEnd {
                text: "two".into(),
                signature: "s2".into(),
            },
            LlmEvent::TextDelta {
                text: "after".into(),
            },
            LlmEvent::StepEnded {
                outcome: outcome(StopReason::EndTurn),
            },
        ];
        let client = ScriptedClient::new(vec![
            script,
            vec![
                LlmEvent::TextDelta { text: "ok".into() },
                LlmEvent::StepEnded {
                    outcome: outcome(StopReason::EndTurn),
                },
            ],
        ]);
        let seen_messages = client.seen_messages_handle();
        let agent = Agent::new(client, EchoDispatcher, None);

        let (cmd_tx, cmd_rx) = mpsc::channel(8);
        let (ev_tx, mut ev_rx) = mpsc::channel(64);
        tokio::spawn(agent.run(cmd_rx, ev_tx));
        for text in ["first", "second"] {
            cmd_tx
                .send(Command::Submit { text: text.into() })
                .await
                .unwrap();
            loop {
                if let Event::TurnEnded { .. } =
                    ev_rx.recv().await.expect("agent dropped the event channel")
                {
                    break;
                }
            }
        }

        // The second turn's request is rebuilt from the log.
        let seen = seen_messages.lock().unwrap();
        let assistant = seen[1]
            .iter()
            .find(|m| m.role == Role::Assistant)
            .expect("an assistant message");
        let shape: Vec<&str> = assistant
            .content
            .iter()
            .map(|b| match b {
                ContentBlock::Thinking { text, .. } => text.as_str(),
                ContentBlock::Text { text } => text.as_str(),
                _ => "?",
            })
            .collect();
        assert_eq!(shape, vec!["one", "between", "two", "after"]);
    }

    /// Regression: the submitted message was pushed on top of
    /// `messages_from_log` and sent twice.
    #[tokio::test]
    async fn submitted_text_reaches_the_llm_exactly_once() {
        let client = ScriptedClient::new(vec![vec![
            LlmEvent::TextDelta { text: "hi".into() },
            LlmEvent::StepEnded {
                outcome: outcome(StopReason::EndTurn),
            },
        ]]);
        let seen_messages = client.seen_messages_handle();
        let agent = Agent::new(client, EchoDispatcher, None);

        let (cmd_tx, cmd_rx) = mpsc::channel(8);
        let (ev_tx, mut ev_rx) = mpsc::channel(32);
        tokio::spawn(agent.run(cmd_rx, ev_tx));

        cmd_tx
            .send(Command::Submit {
                text: "ls -la".into(),
            })
            .await
            .unwrap();

        loop {
            if let Event::TurnEnded { .. } =
                ev_rx.recv().await.expect("agent dropped the event channel")
            {
                break;
            }
        }

        let seen = seen_messages.lock().unwrap();
        assert_eq!(
            seen.len(),
            1,
            "expected one LlmClient::stream call for a single-step turn"
        );
        let user_message_count = seen[0]
            .iter()
            .filter(|m| {
                m.role == Role::User
                    && m.content
                        .iter()
                        .any(|c| matches!(c, ContentBlock::Text { text } if text == "ls -la"))
            })
            .count();
        assert_eq!(
            user_message_count, 1,
            "the submitted text must appear exactly once in the request sent to the LLM"
        );
    }

    #[tokio::test]
    async fn clear_history_wipes_the_log_and_notifies_the_tui() {
        let client = ScriptedClient::new(vec![vec![
            LlmEvent::TextDelta { text: "hi".into() },
            LlmEvent::StepEnded {
                outcome: outcome(StopReason::EndTurn),
            },
        ]]);
        let agent = Agent::new(client, EchoDispatcher, None);
        let log = agent.log().clone();

        let (cmd_tx, cmd_rx) = mpsc::channel(8);
        let (ev_tx, mut ev_rx) = mpsc::channel(32);
        tokio::spawn(agent.run(cmd_rx, ev_tx));

        cmd_tx
            .send(Command::Submit {
                text: "hello".into(),
            })
            .await
            .unwrap();
        loop {
            if matches!(
                ev_rx.recv().await.expect("agent dropped the event channel"),
                Event::TurnEnded { .. }
            ) {
                break;
            }
        }
        assert!(!log.is_empty(), "the turn should have left records behind");

        cmd_tx.send(Command::ClearHistory).await.unwrap();
        assert!(
            matches!(ev_rx.recv().await, Some(Event::HistoryCleared)),
            "ClearHistory must be acknowledged so the TUI can wipe its own rendered log"
        );
        assert!(
            log.is_empty(),
            "ClearHistory must wipe ConversationLog so the next turn starts from nothing"
        );
    }

    #[tokio::test]
    async fn tool_round_trip_dispatches_and_continues() {
        let client = ScriptedClient::new(vec![
            vec![
                LlmEvent::ToolUseRequested {
                    call: ToolCall {
                        id: "t1".into(),
                        name: "read".into(),
                        input: serde_json::json!({}),
                    },
                },
                LlmEvent::StepEnded {
                    outcome: outcome(StopReason::ToolUse),
                },
            ],
            vec![
                LlmEvent::TextDelta {
                    text: "done".into(),
                },
                LlmEvent::StepEnded {
                    outcome: outcome(StopReason::EndTurn),
                },
            ],
        ]);
        let agent = Agent::new(client, EchoDispatcher, None);
        let log = agent.log().clone();

        let (cmd_tx, cmd_rx) = mpsc::channel(8);
        let (ev_tx, mut ev_rx) = mpsc::channel(32);
        tokio::spawn(agent.run(cmd_rx, ev_tx));

        cmd_tx
            .send(Command::Submit { text: "go".into() })
            .await
            .unwrap();

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
        assert!(snap
            .iter()
            .any(|r| matches!(r, LogRecord::ToolResult { .. })));
        assert!(matches!(snap.last().unwrap(), LogRecord::TurnEnded { .. }));
    }

    /// `max_tokens` after a complete tool_use block arrives as `EndTurn`; the
    /// undispatched call must not be logged, or every later request fails.
    #[tokio::test]
    async fn a_call_the_step_did_not_stop_for_is_never_committed() {
        let client = ScriptedClient::new(vec![
            vec![
                LlmEvent::ToolUseRequested {
                    call: ToolCall {
                        id: "t1".into(),
                        name: "read".into(),
                        input: serde_json::json!({}),
                    },
                },
                LlmEvent::StepEnded {
                    outcome: outcome(StopReason::EndTurn),
                },
            ],
            vec![
                LlmEvent::TextDelta { text: "ok".into() },
                LlmEvent::StepEnded {
                    outcome: outcome(StopReason::EndTurn),
                },
            ],
        ]);
        let seen_messages = client.seen_messages_handle();
        let agent = Agent::new(client, EchoDispatcher, None);
        let log = agent.log().clone();

        let (cmd_tx, cmd_rx) = mpsc::channel(8);
        let (ev_tx, mut ev_rx) = mpsc::channel(32);
        tokio::spawn(agent.run(cmd_rx, ev_tx));

        let mut notices = Vec::new();
        for text in ["first", "second"] {
            cmd_tx
                .send(Command::Submit { text: text.into() })
                .await
                .unwrap();
            loop {
                match ev_rx.recv().await.expect("agent dropped the event channel") {
                    Event::Notice { message } => notices.push(message),
                    Event::TurnEnded { .. } => break,
                    _ => {}
                }
            }
        }

        assert!(
            !log.snapshot()
                .iter()
                .any(|r| matches!(r, LogRecord::ToolUse { .. })),
            "an undispatched call must not be logged"
        );
        let seen = seen_messages.lock().unwrap();
        assert!(
            !seen[1]
                .iter()
                .flat_map(|m| &m.content)
                .any(|b| matches!(b, ContentBlock::ToolUse(_))),
            "the next request must not replay a tool_use that has no result: {:?}",
            seen[1]
        );
        assert!(
            notices
                .iter()
                .any(|m| m.contains("before 1 requested tool call(s) could run")),
            "a dropped call must be said out loud, not only logged: {notices:?}"
        );
    }

    /// Looping on a callless `ToolUse` stop would send an empty tool-result
    /// message.
    #[tokio::test]
    async fn a_tool_use_stop_that_named_no_call_ends_the_turn() {
        let client = ScriptedClient::new(vec![vec![
            LlmEvent::TextDelta { text: "hi".into() },
            LlmEvent::StepEnded {
                outcome: outcome(StopReason::ToolUse),
            },
        ]]);
        let seen_messages = client.seen_messages_handle();
        let agent = Agent::new(client, EchoDispatcher, None);

        let (cmd_tx, cmd_rx) = mpsc::channel(8);
        let (ev_tx, mut ev_rx) = mpsc::channel(32);
        tokio::spawn(agent.run(cmd_rx, ev_tx));
        cmd_tx
            .send(Command::Submit { text: "go".into() })
            .await
            .unwrap();

        loop {
            if let Event::TurnEnded { reason, .. } =
                ev_rx.recv().await.expect("agent dropped the event channel")
            {
                assert!(matches!(reason, TurnEndReason::EndTurn));
                break;
            }
        }
        assert_eq!(
            seen_messages.lock().unwrap().len(),
            1,
            "no second step may be requested"
        );
    }

    /// The TUI has already drawn the submission, so discarding it must be said.
    #[tokio::test]
    async fn a_command_discarded_mid_turn_is_said_not_only_logged() {
        let agent = Agent::new(StallingClient, EchoDispatcher, None);
        let log = agent.log().clone();

        let (cmd_tx, cmd_rx) = mpsc::channel(8);
        let (ev_tx, mut ev_rx) = mpsc::channel(32);
        tokio::spawn(agent.run(cmd_rx, ev_tx));

        cmd_tx
            .send(Command::Submit { text: "go".into() })
            .await
            .unwrap();
        loop {
            if let Event::TextDelta { .. } =
                ev_rx.recv().await.expect("agent dropped the event channel")
            {
                break;
            }
        }
        let before = log.len();

        for cmd in [
            Command::Submit {
                text: "and another".into(),
            },
            Command::ClearHistory,
            Command::Resume {
                session: SessionId("s".into()),
                records: vec![],
            },
        ] {
            cmd_tx.send(cmd).await.unwrap();
            let event = ev_rx.recv().await.expect("agent dropped the event channel");
            assert!(
                matches!(&event, Event::Notice { message } if message.starts_with("A turn is running")),
                "{event:?}"
            );
        }
        assert_eq!(
            log.len(),
            before,
            "and the running turn's history is untouched"
        );
    }

    #[tokio::test]
    async fn cancellation_leaves_well_formed_log_not_a_torn_one() {
        let agent = Agent::new(StallingClient, EchoDispatcher, None);
        let log = agent.log().clone();

        let (cmd_tx, cmd_rx) = mpsc::channel(8);
        let (ev_tx, mut ev_rx) = mpsc::channel(32);
        tokio::spawn(agent.run(cmd_rx, ev_tx));

        cmd_tx
            .send(Command::Submit { text: "go".into() })
            .await
            .unwrap();

        // Cancel mid-step, not before the step starts.
        loop {
            if let Event::TextDelta { .. } =
                ev_rx.recv().await.expect("agent dropped the event channel")
            {
                break;
            }
        }
        cmd_tx.send(Command::Cancel).await.unwrap();

        loop {
            if let Event::TurnEnded { reason, .. } =
                ev_rx.recv().await.expect("agent dropped the event channel")
            {
                assert!(matches!(reason, TurnEndReason::Cancelled));
                break;
            }
        }

        let snap = log.snapshot();
        assert!(snap.iter().any(|r| matches!(
            r,
            LogRecord::AssistantMessage { text, .. } if text.as_str() == "partial"
        )));
        assert!(matches!(
            snap.last().unwrap(),
            LogRecord::TurnEnded {
                reason: TurnEndReason::Cancelled,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn cancel_after_tool_use_requested_leaves_no_orphaned_tool_use() {
        let agent = Agent::new(StallingAfterToolUseClient, EchoDispatcher, None);
        let log = agent.log().clone();

        let (cmd_tx, cmd_rx) = mpsc::channel(8);
        let (ev_tx, mut ev_rx) = mpsc::channel(32);
        tokio::spawn(agent.run(cmd_rx, ev_tx));

        cmd_tx
            .send(Command::Submit { text: "go".into() })
            .await
            .unwrap();

        // Cancel after the tool call is requested, before StepEnded.
        loop {
            if let Event::ToolUseRequested { .. } =
                ev_rx.recv().await.expect("agent dropped the event channel")
            {
                break;
            }
        }
        cmd_tx.send(Command::Cancel).await.unwrap();

        loop {
            if let Event::TurnEnded { reason, .. } =
                ev_rx.recv().await.expect("agent dropped the event channel")
            {
                assert!(matches!(reason, TurnEndReason::Cancelled));
                break;
            }
        }

        // Undispatched, so not logged: a ToolUse without ToolResult tears the log.
        let snap = log.snapshot();
        assert!(!snap.iter().any(|r| matches!(r, LogRecord::ToolUse { .. })));
        assert!(matches!(
            snap.last().unwrap(),
            LogRecord::TurnEnded {
                reason: TurnEndReason::Cancelled,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn cancel_during_tool_dispatch_ends_turn_cancelled_with_well_formed_log() {
        let client = ScriptedClient::new(vec![vec![
            LlmEvent::ToolUseRequested {
                call: ToolCall {
                    id: "t1".into(),
                    name: "read".into(),
                    input: serde_json::json!({}),
                },
            },
            LlmEvent::StepEnded {
                outcome: outcome(StopReason::ToolUse),
            },
        ]]);
        let agent = Agent::new(client, StallingDispatcher, None);
        let log = agent.log().clone();

        let (cmd_tx, cmd_rx) = mpsc::channel(8);
        let (ev_tx, mut ev_rx) = mpsc::channel(32);
        tokio::spawn(agent.run(cmd_rx, ev_tx));

        cmd_tx
            .send(Command::Submit { text: "go".into() })
            .await
            .unwrap();

        // Cancel while `dispatch_tools` awaits the call, not merely once requested.
        loop {
            if let Event::ToolDispatched { .. } =
                ev_rx.recv().await.expect("agent dropped the event channel")
            {
                break;
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

        // Every ToolUse needs its ToolResult before TurnEnded, or replay fails.
        let snap = log.snapshot();
        let tool_use_count = snap
            .iter()
            .filter(|r| matches!(r, LogRecord::ToolUse { .. }))
            .count();
        let tool_result_count = snap
            .iter()
            .filter(|r| matches!(r, LogRecord::ToolResult { .. }))
            .count();
        assert_eq!(tool_use_count, 1);
        assert_eq!(tool_result_count, 1);
        assert!(matches!(
            snap.last().unwrap(),
            LogRecord::TurnEnded {
                reason: TurnEndReason::Cancelled,
                ..
            }
        ));
    }

    fn one_edit_then_done() -> ScriptedClient {
        ScriptedClient::new(vec![
            vec![
                LlmEvent::ToolUseRequested {
                    call: ToolCall {
                        id: "t1".into(),
                        name: "edit".into(),
                        input: serde_json::json!({}),
                    },
                },
                LlmEvent::StepEnded {
                    outcome: outcome(StopReason::ToolUse),
                },
            ],
            vec![
                LlmEvent::TextDelta {
                    text: "done".into(),
                },
                LlmEvent::StepEnded {
                    outcome: outcome(StopReason::EndTurn),
                },
            ],
        ])
    }

    /// ADR 0009 §4: the review opens when the turn is about to end, and an
    /// approve is answered with `ReviewClosed` before `TurnEnded`.
    #[tokio::test]
    async fn the_review_opens_at_the_end_of_the_turn_and_an_approve_closes_it() {
        let agent = Agent::new(one_edit_then_done(), ReviewingDispatcher, None);

        let (cmd_tx, cmd_rx) = mpsc::channel(8);
        let (ev_tx, mut ev_rx) = mpsc::channel(32);
        tokio::spawn(agent.run(cmd_rx, ev_tx));
        cmd_tx
            .send(Command::Submit { text: "go".into() })
            .await
            .unwrap();

        loop {
            if let Event::ReviewRequested {
                review_id,
                changeset,
            } = ev_rx.recv().await.expect("agent dropped the event channel")
            {
                assert_eq!(changeset.files[0].path, "f.rs");
                cmd_tx
                    .send(Command::ReviewDecision {
                        review_id,
                        decision: ReviewDecision::Approve,
                    })
                    .await
                    .unwrap();
                break;
            }
        }

        let mut closed = false;
        loop {
            match ev_rx.recv().await.expect("agent dropped the event channel") {
                Event::ReviewClosed {
                    outcome: ReviewOutcome::Saved { files, .. },
                } => {
                    assert_eq!(files, vec!["f.rs".to_string()]);
                    closed = true;
                }
                Event::TurnEnded { reason, .. } => {
                    assert!(closed, "the review must close before the turn ends");
                    assert!(matches!(reason, TurnEndReason::EndTurn));
                    break;
                }
                _ => {}
            }
        }
    }

    /// A comment at the closing review starts the next turn without a `Submit`.
    #[tokio::test]
    async fn a_comment_at_the_closing_review_starts_the_next_turn() {
        let client = ScriptedClient::new(vec![
            vec![
                LlmEvent::ToolUseRequested {
                    call: ToolCall {
                        id: "t1".into(),
                        name: "edit".into(),
                        input: serde_json::json!({}),
                    },
                },
                LlmEvent::StepEnded {
                    outcome: outcome(StopReason::ToolUse),
                },
            ],
            vec![
                LlmEvent::TextDelta {
                    text: "ready".into(),
                },
                LlmEvent::StepEnded {
                    outcome: outcome(StopReason::EndTurn),
                },
            ],
            // The follow-up turn.
            vec![
                LlmEvent::TextDelta {
                    text: "addressed".into(),
                },
                LlmEvent::StepEnded {
                    outcome: outcome(StopReason::EndTurn),
                },
            ],
        ]);
        let seen_messages = client.seen_messages_handle();
        let agent = Agent::new(client, ReviewingDispatcher, None);
        let log = agent.log().clone();

        let (cmd_tx, cmd_rx) = mpsc::channel(8);
        let (ev_tx, mut ev_rx) = mpsc::channel(32);
        tokio::spawn(agent.run(cmd_rx, ev_tx));
        cmd_tx
            .send(Command::Submit { text: "go".into() })
            .await
            .unwrap();

        let mut reviews = 0;
        let mut turns_ended = 0;
        let mut saw_follow_up = false;
        loop {
            match ev_rx.recv().await.expect("agent dropped the event channel") {
                Event::FollowUp { text, .. } => {
                    assert_eq!(text, "Use config");
                    assert_eq!(
                        turns_ended, 1,
                        "announced after the first turn ends and before the second starts"
                    );
                    saw_follow_up = true;
                }
                Event::ReviewRequested { review_id, .. } => {
                    reviews += 1;
                    let decision = if reviews == 1 {
                        ReviewDecision::Comment {
                            comments: vec![ReviewComment {
                                path: "f.rs".into(),
                                lines: (1, 1),
                                text: "Use config".into(),
                            }],
                        }
                    } else {
                        ReviewDecision::Approve
                    };
                    cmd_tx
                        .send(Command::ReviewDecision {
                            review_id,
                            decision,
                        })
                        .await
                        .unwrap();
                }
                Event::TurnEnded { .. } => {
                    turns_ended += 1;
                    if turns_ended == 2 {
                        break;
                    }
                }
                _ => {}
            }
        }
        assert_eq!(
            reviews, 2,
            "the changeset stays staged and is reviewed again"
        );
        assert!(
            saw_follow_up,
            "the comment is announced before the turn it starts"
        );

        let turns = log
            .snapshot()
            .iter()
            .filter(|r| matches!(r, LogRecord::TurnStarted { .. }))
            .count();
        assert_eq!(turns, 2, "the comment started a second turn");
        assert!(
            log.snapshot()
                .iter()
                .any(|r| matches!(r, LogRecord::UserMessage { text, .. } if text == "Use config")),
            "the comment is the second turn's user message"
        );
        let seen = seen_messages.lock().unwrap();
        assert!(seen[2].iter().any(|m| m.role == Role::User
            && m.content
                .iter()
                .any(|c| matches!(c, ContentBlock::Text { text } if text == "Use config"))));
    }

    #[tokio::test]
    async fn a_refused_step_answers_its_calls_without_dispatching_them() {
        let client = ScriptedClient::new(vec![
            vec![
                LlmEvent::ToolUseRequested {
                    call: ToolCall {
                        id: "t1".into(),
                        name: "run".into(),
                        input: serde_json::json!({}),
                    },
                },
                LlmEvent::ToolUseRequested {
                    call: ToolCall {
                        id: "t2".into(),
                        name: "run".into(),
                        input: serde_json::json!({}),
                    },
                },
                LlmEvent::StepEnded {
                    outcome: outcome(StopReason::ToolUse),
                },
            ],
            vec![
                LlmEvent::TextDelta { text: "ok".into() },
                LlmEvent::StepEnded {
                    outcome: outcome(StopReason::EndTurn),
                },
            ],
        ]);
        let agent = Agent::new(client, RefusingDispatcher, None);
        let log = agent.log().clone();

        let (cmd_tx, cmd_rx) = mpsc::channel(8);
        let (ev_tx, mut ev_rx) = mpsc::channel(32);
        tokio::spawn(agent.run(cmd_rx, ev_tx));
        cmd_tx
            .send(Command::Submit { text: "go".into() })
            .await
            .unwrap();

        let mut answered = 0;
        loop {
            match ev_rx.recv().await.expect("agent dropped the event channel") {
                Event::ToolDispatched { .. } => panic!("a refused step dispatches nothing"),
                Event::ToolCompleted { result, .. } => {
                    assert!(result.is_error);
                    assert_eq!(result.content, "the developer commented first");
                    answered += 1;
                }
                Event::TurnEnded { .. } => break,
                _ => {}
            }
        }
        assert_eq!(answered, 2);
        let results = log
            .snapshot()
            .iter()
            .filter(|r| matches!(r, LogRecord::ToolResult { .. }))
            .count();
        assert_eq!(
            results, 2,
            "the log still pairs every ToolUse with a ToolResult"
        );
    }

    #[tokio::test]
    async fn a_question_round_trips_through_the_answer_command() {
        let client = ScriptedClient::new(vec![
            vec![
                LlmEvent::ToolUseRequested {
                    call: ToolCall {
                        id: "t1".into(),
                        name: "ask".into(),
                        input: serde_json::json!({}),
                    },
                },
                LlmEvent::StepEnded {
                    outcome: outcome(StopReason::ToolUse),
                },
            ],
            vec![LlmEvent::StepEnded {
                outcome: outcome(StopReason::EndTurn),
            }],
        ]);
        let agent = Agent::new(client, AskingDispatcher, None);

        let (cmd_tx, cmd_rx) = mpsc::channel(8);
        let (ev_tx, mut ev_rx) = mpsc::channel(32);
        tokio::spawn(agent.run(cmd_rx, ev_tx));
        cmd_tx
            .send(Command::Submit { text: "go".into() })
            .await
            .unwrap();

        loop {
            if let Event::QuestionAsked { call_id, question } =
                ev_rx.recv().await.expect("agent dropped the event channel")
            {
                assert_eq!(question.options.len(), 3);
                cmd_tx
                    .send(Command::Answer {
                        call_id,
                        answer: Answer::Chose { index: 0 },
                    })
                    .await
                    .unwrap();
                break;
            }
        }

        let mut saw_result = false;
        loop {
            match ev_rx.recv().await.expect("agent dropped the event channel") {
                Event::ToolCompleted { result, .. } => {
                    assert_eq!(result.content, "Some(Chose { index: 0 })");
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

    /// Nothing but the abort path removes a cancelled question from `pending`.
    #[tokio::test]
    async fn cancel_during_a_pending_question_does_not_leak_its_entry() {
        let client = ScriptedClient::new(vec![vec![
            LlmEvent::ToolUseRequested {
                call: ToolCall {
                    id: "t1".into(),
                    name: "ask".into(),
                    input: serde_json::json!({}),
                },
            },
            LlmEvent::StepEnded {
                outcome: outcome(StopReason::ToolUse),
            },
        ]]);
        let agent = Agent::new(client, AskingDispatcher, None);
        let pending = agent.pending.clone();

        let (cmd_tx, cmd_rx) = mpsc::channel(8);
        let (ev_tx, mut ev_rx) = mpsc::channel(32);
        tokio::spawn(agent.run(cmd_rx, ev_tx));
        cmd_tx
            .send(Command::Submit { text: "go".into() })
            .await
            .unwrap();

        loop {
            if let Event::QuestionAsked { .. } =
                ev_rx.recv().await.expect("agent dropped the event channel")
            {
                break;
            }
        }
        assert_eq!(
            pending.lock().unwrap().len(),
            1,
            "the pending question should be registered before cancel"
        );

        cmd_tx.send(Command::Cancel).await.unwrap();

        loop {
            if let Event::TurnEnded { reason, .. } =
                ev_rx.recv().await.expect("agent dropped the event channel")
            {
                assert!(matches!(reason, TurnEndReason::Cancelled));
                break;
            }
        }
        assert!(
            pending.lock().unwrap().is_empty(),
            "abort_dispatch must clear dangling pending entries on cancel"
        );
    }

    /// The same for a review, which is not keyed by call id.
    #[tokio::test]
    async fn cancel_during_a_pending_review_does_not_leak_its_entry() {
        let agent = Agent::new(one_edit_then_done(), ReviewingDispatcher, None);
        let pending = agent.pending.clone();

        let (cmd_tx, cmd_rx) = mpsc::channel(8);
        let (ev_tx, mut ev_rx) = mpsc::channel(32);
        tokio::spawn(agent.run(cmd_rx, ev_tx));
        cmd_tx
            .send(Command::Submit { text: "go".into() })
            .await
            .unwrap();

        loop {
            if let Event::ReviewRequested { .. } =
                ev_rx.recv().await.expect("agent dropped the event channel")
            {
                break;
            }
        }
        assert_eq!(pending.lock().unwrap().len(), 1);
        cmd_tx.send(Command::Cancel).await.unwrap();
        loop {
            if let Event::TurnEnded { reason, .. } =
                ev_rx.recv().await.expect("agent dropped the event channel")
            {
                assert!(matches!(reason, TurnEndReason::Cancelled));
                break;
            }
        }
        assert!(pending.lock().unwrap().is_empty());
    }

    /// Regression: a tool-use-only step's ToolUse reached the log but not the
    /// live `messages`, so the next step sent an orphan ToolResult.
    #[tokio::test]
    async fn multi_step_turn_carries_tool_use_into_the_next_steps_live_request() {
        let client = ScriptedClient::new(vec![
            vec![
                LlmEvent::ToolUseRequested {
                    call: ToolCall {
                        id: "t1".into(),
                        name: "read".into(),
                        input: serde_json::json!({}),
                    },
                },
                LlmEvent::StepEnded {
                    outcome: outcome(StopReason::ToolUse),
                },
            ],
            vec![
                LlmEvent::TextDelta {
                    text: "done".into(),
                },
                LlmEvent::StepEnded {
                    outcome: outcome(StopReason::EndTurn),
                },
            ],
        ]);
        let seen_messages = client.seen_messages_handle();
        let agent = Agent::new(client, EchoDispatcher, None);

        let (cmd_tx, cmd_rx) = mpsc::channel(8);
        let (ev_tx, mut ev_rx) = mpsc::channel(32);
        tokio::spawn(agent.run(cmd_rx, ev_tx));

        cmd_tx
            .send(Command::Submit { text: "go".into() })
            .await
            .unwrap();

        loop {
            if let Event::TurnEnded { .. } =
                ev_rx.recv().await.expect("agent dropped the event channel")
            {
                break;
            }
        }

        let seen = seen_messages.lock().unwrap();
        assert_eq!(
            seen.len(),
            2,
            "expected one LlmClient::stream call per step"
        );
        let second_request = &seen[1];

        let tool_use_idx = second_request
            .iter()
            .position(|m| {
                m.role == Role::Assistant
                    && m.content
                        .iter()
                        .any(|c| matches!(c, ContentBlock::ToolUse(call) if call.id == "t1"))
            })
            .expect("second step's request must include the assistant's tool_use block for t1");
        let tool_result_idx = second_request
            .iter()
            .position(|m| {
                m.role == Role::User
                    && m.content
                        .iter()
                        .any(|c| matches!(c, ContentBlock::ToolResult(r) if r.call_id == "t1"))
            })
            .expect("second step's request must include the tool result for t1");
        assert_eq!(
            tool_result_idx,
            tool_use_idx + 1,
            "tool_use must be immediately followed by its tool_result, with nothing in between"
        );
    }

    /// Every record and move the log sent its sink.
    #[derive(Debug, Default)]
    struct SpySink {
        records: Mutex<Vec<LogRecord>>,
        moves: Mutex<Vec<String>>,
    }

    impl RecordSink for SpySink {
        fn append(&self, record: &LogRecord) {
            self.records.lock().unwrap().push(record.clone());
        }
        fn cleared(&self) {
            self.moves.lock().unwrap().push("cleared".into());
        }
        fn resumed(&self, session: &SessionId) {
            self.moves
                .lock()
                .unwrap()
                .push(format!("resumed {session}"));
        }
    }

    /// Answers each step from a script, then stalls.
    struct ScriptedThenStallingClient(Mutex<VecDeque<Vec<LlmEvent>>>);

    impl LlmClient for ScriptedThenStallingClient {
        fn stream<'a>(
            &'a self,
            _request: LlmRequest<'a>,
        ) -> Pin<Box<dyn futures::Stream<Item = Result<LlmEvent, LlmError>> + Send + 'a>> {
            match self.0.lock().unwrap().pop_front() {
                Some(events) => Box::pin(stream::iter(events.into_iter().map(Ok))),
                None => Box::pin(stream::pending()),
            }
        }
    }

    /// Hands back one follow-up at the end of the first turn, as a review
    /// with comments does, and none after.
    #[derive(Default)]
    struct FollowingUpDispatcher(std::sync::atomic::AtomicBool);

    #[async_trait]
    impl ToolDispatcher for FollowingUpDispatcher {
        async fn dispatch(&self, call: ToolCall, _ctx: &DispatchContext) -> ToolResult {
            ToolResult {
                call_id: call.id,
                content: "ok".into(),
                is_error: false,
            }
        }
        fn definitions(&self) -> Vec<ToolDefinition> {
            vec![]
        }
        async fn turn_ending(&self, _ctx: &DispatchContext) -> Option<String> {
            let already = self.0.swap(true, std::sync::atomic::Ordering::Relaxed);
            (!already).then(|| "follow-up".to_string())
        }
    }

    async fn until<T>(
        ev_rx: &mut mpsc::Receiver<Event>,
        mut wanted: impl FnMut(Event) -> Option<T>,
    ) -> T {
        loop {
            let event = ev_rx.recv().await.expect("agent dropped the event channel");
            if let Some(found) = wanted(event) {
                return found;
            }
        }
    }

    /// Regression: `/clear` or `/resume` between a follow-up's `TurnEnded`
    /// and the next `TurnStarted` moved the sink although core refused it.
    #[tokio::test]
    async fn the_sink_moves_only_when_core_acts_and_never_between_follow_up_turns() {
        let client = ScriptedThenStallingClient(Mutex::new(VecDeque::from([vec![
            LlmEvent::TextDelta {
                text: "first".into(),
            },
            LlmEvent::StepEnded {
                outcome: outcome(StopReason::EndTurn),
            },
        ]])));
        let sink = Arc::new(SpySink::default());
        let agent =
            Agent::new(client, FollowingUpDispatcher::default(), None).with_sink(sink.clone());

        let (cmd_tx, cmd_rx) = mpsc::channel(8);
        let (ev_tx, mut ev_rx) = mpsc::channel(32);
        tokio::spawn(agent.run(cmd_rx, ev_tx));
        cmd_tx
            .send(Command::Submit { text: "go".into() })
            .await
            .unwrap();

        // The gap: follow-up announced, its `TurnStarted` not yet sent.
        until(&mut ev_rx, |e| {
            matches!(e, Event::FollowUp { .. }).then_some(())
        })
        .await;
        cmd_tx.send(Command::ClearHistory).await.unwrap();
        cmd_tx
            .send(Command::Resume {
                session: SessionId("elsewhere".into()),
                records: vec![],
            })
            .await
            .unwrap();

        for _ in 0..2 {
            let message = until(&mut ev_rx, |e| match e {
                Event::Notice { message } => Some(message),
                _ => None,
            })
            .await;
            assert!(message.starts_with("A turn is running"), "{message}");
        }
        assert!(
            sink.moves.lock().unwrap().is_empty(),
            "the writer stays on this conversation's file"
        );
        assert!(
            sink.records
                .lock()
                .unwrap()
                .iter()
                .any(|r| matches!(r, LogRecord::UserMessage { text, .. } if text == "follow-up")),
            "and the follow-up turn is written to it"
        );

        // Once the conversation is idle, the same command moves the writer.
        cmd_tx.send(Command::Cancel).await.unwrap();
        until(&mut ev_rx, |e| {
            matches!(e, Event::TurnEnded { .. }).then_some(())
        })
        .await;
        cmd_tx.send(Command::ClearHistory).await.unwrap();
        until(&mut ev_rx, |e| {
            matches!(e, Event::HistoryCleared).then_some(())
        })
        .await;
        assert_eq!(*sink.moves.lock().unwrap(), ["cleared"]);
    }

    #[tokio::test]
    async fn a_resumed_conversation_continues_past_its_highest_ids() {
        let client = ScriptedClient::new(vec![vec![
            LlmEvent::TextDelta { text: "hi".into() },
            LlmEvent::StepEnded {
                outcome: outcome(StopReason::EndTurn),
            },
        ]]);
        let agent = Agent::new(client, EchoDispatcher, None);

        let (cmd_tx, cmd_rx) = mpsc::channel(8);
        let (ev_tx, mut ev_rx) = mpsc::channel(32);
        tokio::spawn(agent.run(cmd_rx, ev_tx));
        cmd_tx
            .send(Command::Resume {
                session: SessionId("earlier".into()),
                records: vec![
                    LogRecord::TurnStarted { turn_id: TurnId(7) },
                    LogRecord::AssistantMessage {
                        turn_id: TurnId(7),
                        step_id: StepId(12),
                        text: "from disk".into(),
                    },
                    LogRecord::TurnEnded {
                        turn_id: TurnId(7),
                        reason: TurnEndReason::EndTurn,
                    },
                ],
            })
            .await
            .unwrap();
        // Said by core after acting, not by the interceptor: core may refuse.
        let said = until(&mut ev_rx, |e| match e {
            Event::Notice { message } => Some(message),
            _ => None,
        })
        .await;
        assert_eq!(said, "Resumed the conversation: 1 turn restored.");
        cmd_tx
            .send(Command::Submit {
                text: "again".into(),
            })
            .await
            .unwrap();

        let turn = until(&mut ev_rx, |e| match e {
            Event::TurnStarted { turn_id } => Some(turn_id),
            _ => None,
        })
        .await;
        let step = until(&mut ev_rx, |e| match e {
            Event::StepEnded { step_id, .. } => Some(step_id),
            _ => None,
        })
        .await;
        assert_eq!((turn, step), (TurnId(8), StepId(13)));
    }
}
