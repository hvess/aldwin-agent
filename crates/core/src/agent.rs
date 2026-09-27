use futures::{future, StreamExt};
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

use crate::{
    client::{LlmClient, LlmRequest},
    dispatcher::{DispatchContext, PendingMap, PendingReply, ToolDispatcher},
    event::{Command, Event, LlmEvent, LogRecord, StepOutcome, TurnEndReason},
    log::{ConversationLog, RecordSink},
    prompt,
    types::*,
};

/// The conversation loop: one per session. It owns the log, streams each
/// step from `C`, and hands the step's tool calls to `D`; `run` drives it
/// from a command channel and reports on an event channel.
pub struct Agent<C, D> {
    client: C,
    dispatcher: D,
    log: ConversationLog,
    system: String,
    // Pending questions (keyed by the `ask` call's id) and reviews (keyed by
    // their own id) — see `PendingReply`. Shared rather than owned
    // locally by `run` so `DispatchContext` — handed to a dispatch future
    // that runs concurrently with the command loop — can register into the
    // same map the loop resolves against.
    pending: PendingMap,
    /// The last turn and step ids minted. The agent is the only thing that
    /// mints them, so the counters live here rather than in a global; a
    /// resumed conversation moves them past the highest id it carries.
    last_turn: u64,
    last_step: u64,
}

/// Written by hand because neither `C` nor `D` need be `Debug`, and the
/// system prompt is too long to be worth printing.
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
    /// An agent with an empty, historyless log, whose system prompt is the
    /// base prompt with `additional_context` appended when there is one.
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

    /// Write this session's records through to a transcript as well as to
    /// memory. Builder-style rather than a `new` argument because every
    /// caller but aldwin-cli's bootstrap — every test in this crate
    /// included — wants the historyless log `new` already builds.
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

    /// Moves the counters past every id in `records`, so a turn started
    /// after a resume never reuses an id the resumed conversation carries.
    fn continue_ids(&mut self, records: &[LogRecord]) {
        for record in records {
            let (turn, step) = record.ids();
            self.last_turn = self.last_turn.max(turn.0);
            if let Some(step) = step {
                self.last_step = self.last_step.max(step.0);
            }
        }
    }

    /// Resolve a pending question. An entry that turns out to be a review is
    /// put back unconsumed — that shouldn't happen, but it must not destroy
    /// a live entry if it somehow does.
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

    /// Resolve a pending review — same shape as `resolve_answer`.
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

    /// A command that arrived while a turn is in flight. Returns `true` when
    /// it is `Cancel`; everything that would change the history under a
    /// running turn is discarded.
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
        // Said: the TUI has already drawn the submission, and a developer
        // who sees it in the transcript assumes it arrived.
        if let Some(what) = discarded {
            let _ = events
                .send(Event::Notice {
                    message: format!("A turn is running, so {what}. Stop it with esc first."),
                })
                .await;
        }
        false
    }

    /// Drive the agent. Returns when the command channel closes.
    pub async fn run(mut self, mut commands: mpsc::Receiver<Command>, events: mpsc::Sender<Event>) {
        while let Some(cmd) = commands.recv().await {
            match cmd {
                Command::Submit { text } => {
                    // A review at the end of a turn can hand back comments,
                    // which are a message from the developer and start the
                    // next turn without them having to type it (ADR 0009 §4).
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

                // `replace`, not a loop of `append` — see
                // `ConversationLog::replace`. The event carries the records
                // back out so the TUI rebuilds from the copy core just took
                // rather than from a second read of the file.
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

    /// Runs one turn to its end. The second half of the result is a message
    /// the developer left at the closing review — see `ToolDispatcher::turn_ending`
    /// — which the caller starts a new turn with.
    async fn run_turn(
        &mut self,
        turn_id: TurnId,
        events: &mpsc::Sender<Event>,
        commands: &mut mpsc::Receiver<Command>,
    ) -> (TurnEndReason, Option<String>) {
        let _ = events.send(Event::TurnStarted { turn_id }).await;

        // The just-submitted user message is already in `self.log`, so
        // `messages_from_log` reconstructs it — pushing it again here would
        // send it to the LLM twice.
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
                    // The model has stopped; anything staged is about to be
                    // left un-reviewed, so the dispatcher gets the moment to
                    // open the review (ADR 0009 §4). Still cancellable: the
                    // review is a wait on the developer like any other.
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
                    // Tool results go back as a user-role message (Anthropic convention).
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
                    // Dispatch was cut short after ToolUse records were already
                    // logged. Close the step out in the shape of a completed
                    // round trip so no ToolUse is left without a ToolResult and
                    // a later replay of the log stays a valid request.
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
                StepResult::Error(msg) => return (TurnEndReason::Error(msg), None),
            }
        }
    }

    /// Drives `fut` while staying responsive to commands — `Cancel` above
    /// all — the same way `dispatch_tools` does. `Err` is the reason the turn
    /// ends instead.
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
                        None => return Err(TurnEndReason::Error("command channel closed".into())),
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
        // This step's assistant content, **in the order it arrived**. The
        // provider wants its blocks back as it emitted them; with interleaved
        // thinking that can be thinking, text, thinking again, and sorting
        // the thinking to the front would hand back a turn it never wrote.
        // Text is flushed into here whenever something else interrupts it.
        let mut content: Vec<ContentBlock> = Vec::new();

        // Inner block: the stream lives here, releasing its borrow of
        // `messages` at the end so the step's output can be pushed onto it.
        let stream_terminal: StepTerminal = {
            let request = LlmRequest {
                system: &self.system,
                tools: &tools,
                messages: messages.as_slice(),
                // The whole conversation so far is the cached prefix.
                cache_breakpoint: messages.len().checked_sub(1),
            };
            let mut stream = self.client.stream(request);

            loop {
                tokio::select! {
                    biased;

                    cmd = commands.recv() => {
                        match cmd {
                            Some(cmd) => if self.on_mid_turn_command(cmd, events).await { break StepTerminal::Cancelled },
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

        // Commit this step's assistant content before anything else — a
        // cancelled or failed step keeps what it said. Everything up to the
        // last interruption is already in `content`; what remains is the
        // trailing text.
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
            StepTerminal::Error(msg) => return StepResult::Error(msg),
            StepTerminal::Ok(outcome) => outcome,
        };

        // A call is committed only if it is about to be dispatched, because a
        // ToolUse with no matching ToolResult is a torn log: every later
        // request replays it and the provider rejects them all. So nothing is
        // logged before the cancellation/error check above, and nothing is
        // logged for a step that requested a call and then stopped for another
        // reason — `max_tokens` landing after a complete tool_use block
        // reaches here as `EndTurn`.
        //
        // Said: the TUI has already drawn the call as
        // requested, and a step that also produced text does not reach the
        // silent-turn notice below — so without this the call simply
        // disappears between the reply and the end of the turn.
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

        // Committed to the live `messages` as well as the log: the ToolResult
        // for this step rides as a `Role::User` message, and the provider
        // rejects it unless the assistant message in front of it carries the
        // matching ToolUse block — including on a step that produced no text.
        for call in &tool_calls {
            self.log.append(LogRecord::ToolUse {
                turn_id,
                step_id,
                call: call.clone(),
            });
            push_assistant_block(messages, ContentBlock::ToolUse(call.clone()));
        }

        // The floor against a silent turn (ADR 0006): a step that ends the
        // turn with no text and no tool call has nothing to render, so it
        // says so rather than looking hung.
        if !produced_text && tool_calls.is_empty() {
            let _ = events
                .send(Event::Notice {
                    message: "the agent ended the turn without a reply".into(),
                })
                .await;
        }

        // A `ToolUse` stop that named no call has nothing to dispatch, and an
        // empty tool-result message would be rejected; the turn ends instead.
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

    /// Commits the text streamed so far as one block — to the log as it
    /// happens, so a crash mid-step cannot lose it and a replay of the log
    /// reproduces the same order.
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

    /// Dispatch all tool calls for a step concurrently, staying responsive to
    /// commands (Cancel above all) for the whole duration — a slow or stuck
    /// tool must not make cancellation meaningless. A tool that needs the
    /// developer blocks inside its own future via `DispatchContext`; the
    /// agent loop just awaits.
    async fn dispatch_tools(
        &self,
        turn_id: TurnId,
        step_id: StepId,
        calls: Vec<ToolCall>,
        events: &mpsc::Sender<Event>,
        commands: &mut mpsc::Receiver<Command>,
    ) -> DispatchOutcome {
        let ctx = DispatchContext::new(turn_id, step_id, events.clone(), self.pending.clone());

        // The dispatcher's moment before the step (ADR 0009 §4): a staged
        // changeset that these calls would observe is reviewed first. A
        // reason back means the calls do not run and are each answered with
        // it — the developer's comments, or that the changes were discarded.
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

        // Drive all dispatch futures on the current task (cooperative).
        // Dispatcher impls use spawn_blocking internally for CPU-heavy work.
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
                        None => TurnEndReason::Error("command channel closed".into()),
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

    /// Close out a step whose tool dispatch was cut short (cancelled, or the
    /// command channel closed) without leaving the already-logged ToolUse
    /// records orphaned: every in-flight call gets a synthetic error
    /// ToolResult, so the step still reads as a well-formed round trip and
    /// never a torn one.
    async fn abort_dispatch(
        &self,
        turn_id: TurnId,
        step_id: StepId,
        calls: &[ToolCall],
        events: &mpsc::Sender<Event>,
        reason: TurnEndReason,
    ) -> DispatchOutcome {
        // A call whose dispatch future was mid-`ask` otherwise leaves a
        // dangling entry here — nothing resolves it once the future holding
        // its receiver is gone. A review in flight is keyed by its own id
        // rather than a call's, so it is cleared separately.
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

    /// Reconstruct the wire message list from the append-only log.
    ///
    /// Thinking is logged as each block closes, and text is flushed to the
    /// log whenever thinking interrupts it — so replaying in log order
    /// reproduces the order the provider emitted, which is the order it wants
    /// back (ADR 0006 §2).
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

/// Appends to the assistant message being built, or opens one: a step's
/// blocks all belong to a single assistant message, in the order they arrived.
fn push_assistant_block(messages: &mut Vec<Message>, block: ContentBlock) {
    match messages.last_mut() {
        Some(last) if last.role == Role::Assistant => last.content.push(block),
        _ => messages.push(Message {
            role: Role::Assistant,
            content: vec![block],
        }),
    }
}

// ── Internal ─────────────────────────────────────────────────────────────────

enum StepTerminal {
    Ok(StepOutcome),
    Cancelled,
    Error(String),
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
    Error(String),
}

enum DispatchOutcome {
    Completed(Vec<ToolResult>),
    Aborted {
        results: Vec<ToolResult>,
        reason: TurnEndReason,
    },
}

// ── Tests ────────────────────────────────────────────────────────────────────
//
// Covers the loop's real failure modes per aldwin-core.md's Pitfalls: torn logs
// on cancellation, tool round trips, and step/turn boundary bookkeeping. Not
// exhaustive by design — these are the invariants a refactor is most likely to
// break silently.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::LlmError;
    use async_trait::async_trait;
    use futures::stream;
    use std::collections::VecDeque;
    use std::pin::Pin;
    use std::sync::Mutex;

    /// Replays one canned event script per `stream()` call, one script per
    /// step. Also records the `messages` slice it was called with, so a test
    /// can inspect exactly what a later step's request looked like.
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

    /// Yields one delta then never terminates — for exercising mid-step cancellation.
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

    /// Yields a tool-use request then never terminates — for exercising cancellation
    /// that lands after a tool call is requested but before the step ends.
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

    /// Never resolves — for exercising cancellation that lands while tools are
    /// actually dispatching (as opposed to mid-stream, before dispatch starts).
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

    /// Opens a review from `turn_ending` — for exercising the review round
    /// trip end to end, including a comment starting the next turn.
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

    /// Refuses every step from `before_step` — the comments-before-a-run
    /// path, where the step's calls are answered rather than dispatched.
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

    /// Blocks on `DispatchContext::ask` — for exercising the question
    /// round trip end to end.
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

    /// ADR 0006. A step whose whole output was a thinking block used to
    /// commit nothing: the block was dropped at the wire and the turn
    /// reached the developer blank. The observed case spent 14,096 output
    /// tokens that way and was answered by hand with "Continue".
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
        // ...and the developer was told the turn produced no reply, rather
        // than being shown nothing at all.
        assert!(noticed, "a turn with no visible output must say so");
    }

    /// The wire-correctness half: thinking that preceded a tool call has to
    /// go back in front of it, in the assistant message that made the call,
    /// or the provider rejects the next request.
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

        // The second step's request is where the first step's assistant turn
        // shows up as history.
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

    /// With interleaved thinking the provider can emit text *between* two
    /// thinking blocks, and it wants the turn back as it wrote it — live, and
    /// again when the log is replayed for the next turn.
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

    /// `messages_from_log` already includes the just-logged `UserMessage`;
    /// an extra push of the same text once sent every message to the LLM
    /// twice while the log itself stayed clean.
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

    /// `max_tokens` can land after a complete tool_use block, and the wire
    /// layer reports every stop that is not `tool_use` as `EndTurn`. The call
    /// is never dispatched, so committing it would leave a ToolUse with no
    /// ToolResult — and every later request in the session would be rejected.
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

    /// A `ToolUse` stop with no call has nothing to dispatch; looping would
    /// send the provider an empty tool-result message.
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

    /// The TUI draws a submission the moment it is typed, so one that core
    /// throws away has to be said out loud.
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

        // Wait for the partial delta so cancel lands mid-step, not before the step starts.
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

        // Wait for the tool call to be requested so cancel lands after it, before StepEnded.
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

        // The requested tool call was never dispatched, so it must not appear in the
        // log — a logged ToolUse with no ToolResult/StepBoundary would be a torn log.
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

        // Wait until the tool is actually dispatched (not merely requested) so
        // cancel lands while dispatch_tools is awaiting it, not before.
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

        // Every ToolUse must be followed by a matching ToolResult before
        // TurnEnded — an unresolved one would make a later replay of this log
        // an invalid request to the LLM (a dangling tool_use block).
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

    /// A comment at the closing review is a message from the developer: the
    /// turn ends and the next one starts with it, without a `Submit`.
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

    /// `before_step`'s refusal answers every call in the step and dispatches
    /// none of them.
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

    /// Cancelling with a question in flight must not leave a dangling
    /// `oneshot::Sender` in `pending` — nothing else ever removes it.
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

    /// The same for a review, which is keyed by its own id rather than a
    /// call's and so needs its own clearing.
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

    /// A live run surfaced this: a tool-use-only step (no text) got its
    /// ToolUse block into the log but not the live `messages`, so the *next*
    /// step's request carried a ToolResult with no ToolUse in front of it.
    /// `messages_from_log` rebuilt it correctly, which is why the gap only
    /// showed mid-turn, on a provider strict about role sequencing.
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

    /// What the log told its sink: every record, and every time the sink
    /// was moved.
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

    /// Answers each step from a script, and stalls once the script is spent
    /// — a turn that stays open for as long as a test needs it to.
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

    /// `/clear` or `/resume` sent in the gap between a review's follow-up
    /// turns — after one turn's `TurnEnded`, before the next one's
    /// `TurnStarted` — used to move the transcript writer on the way past,
    /// though core then refused the command: the rest of the conversation
    /// went into another session's file. The sink moves only when core acts,
    /// and core does not act while the conversation is still going.
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

        // The gap: the first turn has ended and its follow-up is announced,
        // ahead of the follow-up turn's own `TurnStarted`.
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

    /// Turn and step ids are minted by the agent, and a resumed conversation
    /// carries its own: the next turn must not reuse one of them.
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
        // Said by core, once it has acted — never by the interceptor ahead
        // of a resume core may refuse.
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
