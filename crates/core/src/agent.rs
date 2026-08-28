use futures::{future, StreamExt};
use std::collections::HashMap;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::{
    client::{LlmClient, LlmRequest},
    dispatcher::ToolDispatcher,
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
        }
    }

    pub fn log(&self) -> &ConversationLog { &self.log }

    /// Drive the agent. Returns when the command channel closes.
    pub async fn run(
        mut self,
        mut commands: mpsc::Receiver<Command>,
        events:       mpsc::Sender<Event>,
    ) {
        // Pending Edit approval gates: call_id → oneshot tx (resolved by ApproveTool/DenyTool).
        let mut approvals: HashMap<String, oneshot::Sender<bool>> = HashMap::new();
        // Pending permission prompts: PromptId → oneshot tx.
        let mut prompts: HashMap<u64, oneshot::Sender<serde_json::Value>> = HashMap::new();

        while let Some(cmd) = commands.recv().await {
            match cmd {
                Command::Submit { text } => {
                    let turn_id = TurnId::next();
                    self.log.append(LogRecord::TurnStarted { turn_id });
                    self.log.append(LogRecord::UserMessage { turn_id, text: text.clone() });

                    let cancel = CancellationToken::new();
                    let reason = self
                        .run_turn(
                            turn_id, text, &events, &mut commands,
                            &mut approvals, &mut prompts, cancel,
                        )
                        .await;

                    self.log.append(LogRecord::TurnEnded { turn_id, reason: reason.clone() });
                    let _ = events.send(Event::TurnEnded { turn_id, reason }).await;
                }

                Command::ApproveTool { call_id } => {
                    if let Some(tx) = approvals.remove(&call_id) { let _ = tx.send(true); }
                    else { warn!("ApproveTool for unknown call_id {call_id}"); }
                }
                Command::DenyTool { call_id } => {
                    if let Some(tx) = approvals.remove(&call_id) { let _ = tx.send(false); }
                    else { warn!("DenyTool for unknown call_id {call_id}"); }
                }
                Command::PromptResponse { id, payload } => {
                    if let Some(tx) = prompts.remove(&id.0) { let _ = tx.send(payload); }
                    else { warn!("PromptResponse for unknown prompt {}", id.0); }
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
        approvals: &mut HashMap<String, oneshot::Sender<bool>>,
        prompts:   &mut HashMap<u64, oneshot::Sender<serde_json::Value>>,
        cancel:    CancellationToken,
    ) -> TurnEndReason {
        let _ = events.send(Event::TurnStarted { turn_id }).await;

        let mut messages: Vec<Message> = self.messages_from_log();
        messages.push(Message::user(user_text));

        loop {
            let step_id = StepId::next();
            debug!("turn {turn_id:?} step {step_id:?}");

            let result = self
                .run_step(
                    turn_id, step_id, &mut messages, events,
                    commands, approvals, prompts, cancel.clone(),
                )
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
        approvals: &mut HashMap<String, oneshot::Sender<bool>>,
        prompts:   &mut HashMap<u64, oneshot::Sender<serde_json::Value>>,
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
                            Some(Command::ApproveTool { call_id }) => {
                                if let Some(tx) = approvals.remove(&call_id) { let _ = tx.send(true); }
                            }
                            Some(Command::DenyTool { call_id }) => {
                                if let Some(tx) = approvals.remove(&call_id) { let _ = tx.send(false); }
                            }
                            Some(Command::PromptResponse { id, payload }) => {
                                if let Some(tx) = prompts.remove(&id.0) { let _ = tx.send(payload); }
                            }
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

        // Commit observed tool calls to log.
        for call in &tool_calls {
            self.log.append(LogRecord::ToolUse { turn_id, step_id, call: call.clone() });
        }

        match stream_terminal {
            StepTerminal::Cancelled  => return StepResult::Cancelled,
            StepTerminal::Error(msg) => return StepResult::Error(msg),
            StepTerminal::Ok         => {}
        }

        let outcome = step_outcome.expect("StepTerminal::Ok implies StepEnded was received");

        match outcome.stop_reason {
            StopReason::EndTurn  => StepResult::EndTurn(outcome),
            StopReason::ToolUse  => {
                match self.dispatch_tools(turn_id, step_id, tool_calls, events, cancel).await {
                    Ok(results) => StepResult::ToolsDispatched { outcome, results },
                    Err(msg)    => StepResult::Error(msg),
                }
            }
        }
    }

    /// Dispatch all tool calls for a step concurrently.
    /// Approval-gated tools (Edit) block inside their own future.
    async fn dispatch_tools(
        &self,
        turn_id: TurnId,
        step_id: StepId,
        calls:   Vec<ToolCall>,
        events:  &mpsc::Sender<Event>,
        cancel:  CancellationToken,
    ) -> Result<Vec<ToolResult>, String> {
        for call in &calls {
            let _ = events.send(Event::ToolDispatched {
                turn_id, step_id, call_id: call.id.clone(),
            }).await;
        }

        // Drive all dispatch futures on the current task (cooperative).
        // Dispatcher impls use spawn_blocking internally for CPU-heavy work.
        let futs: Vec<_> = calls.iter().map(|call| self.dispatcher.dispatch(call.clone())).collect();

        tokio::select! {
            _ = cancel.cancelled() => Err("cancelled during tool dispatch".into()),
            results = future::join_all(futs) => {
                for result in &results {
                    let _ = events.send(Event::ToolCompleted {
                        turn_id, step_id, result: result.clone(),
                    }).await;
                }
                Ok(results)
            }
        }
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
    Cancelled,
    Error(String),
}
