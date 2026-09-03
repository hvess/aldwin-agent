use std::sync::Arc;

use mjolnir_core::{DispatchContext, ToolCall, ToolDefinition, ToolResult};
use mjolnir_permissions::{CheckOutcome, Decision, Engine, PromptPayload, PromptResponse};
use async_trait::async_trait;

use crate::error::ToolError;
use crate::registry::{Registry, ToolDescriptor};

/// Implements core's `ToolDispatcher`. Owns the dispatch flow described in
/// mjolnir-tools.md: resolve name -> tool, run the generic permission check
/// for `edit_class: false` tools (emitting `PromptRequested` / awaiting
/// `PromptResponse` on Deny-by-absence), then run the tool's future.
/// `edit_class: true` tools skip the permission check entirely — their
/// approval gate lives inside their own future (see `gate.rs`).
pub struct Dispatcher {
    registry:    Registry,
    permissions: Arc<Engine>,
    /// Serialises the prompt-and-record half of `check` across the tool
    /// calls of a step, which `mjolnir-core`'s `dispatch_tools` drives
    /// concurrently (`future::join_all`) — see `check`'s own doc comment for
    /// the bug that makes this necessary. Held only while a prompt is
    /// genuinely outstanding, so calls the engine can already answer never
    /// touch it.
    prompt_gate: tokio::sync::Mutex<()>,
}

impl Dispatcher {
    pub fn new(registry: Registry, permissions: Arc<Engine>) -> Self {
        Self { registry, permissions, prompt_gate: tokio::sync::Mutex::new(()) }
    }

    /// `Ok(true)` if the call may proceed, `Ok(false)` if denied (initially
    /// or by the developer's prompt response). Only called for `edit_class:
    /// false` tools — see module doc.
    ///
    /// The check happens twice on the prompt path, either side of
    /// `prompt_gate`, and that is the whole point: a step's tool calls are
    /// dispatched concurrently, so with one shared check every call in the
    /// step reached `check_tool` before the developer had answered anything,
    /// and each one independently got `PromptRequired` back. Answering the
    /// first prompt with a grant that plainly covered the rest — approving a
    /// directory, say — changed nothing for them, because their outcome was
    /// already decided; the developer was asked again for every queued call
    /// in the same directory they had just approved. That is the reported
    /// "directory permissions don't appear to count properly when commands
    /// are queued".
    ///
    /// Taking the gate before prompting makes the queued calls wait, and
    /// re-checking after acquiring it is what lets the grant the developer
    /// just made actually apply: a call whose target is now covered proceeds
    /// silently, and only a genuinely still-uncovered one prompts. This is
    /// also what makes prompting one-at-a-time real rather than incidental
    /// — the TUI already only makes the front of its queue interactive.
    async fn check(&self, descriptor: &ToolDescriptor, call: &ToolCall, ctx: &DispatchContext) -> Result<bool, ToolError> {
        let tool = self.registry.get(&call.name).expect("caller already resolved this name");
        let target = tool.permission_target(&call.input)?;
        let path_like = tool.permission_target_is_path(&call.input);
        let kind = permission_kind(&descriptor.name);

        match self.permissions.check_tool(&kind, &target, false, path_like) {
            CheckOutcome::Allow => Ok(true),
            CheckOutcome::Deny => Ok(false),
            CheckOutcome::PromptRequired(_) => {
                let _gate = self.prompt_gate.lock().await;
                match self.permissions.check_tool(&kind, &target, false, path_like) {
                    CheckOutcome::Allow => Ok(true),
                    CheckOutcome::Deny => Ok(false),
                    CheckOutcome::PromptRequired(payload) => self.prompt_and_record(&kind, payload, &call.id, ctx).await,
                }
            }
        }
    }

    async fn prompt_and_record(
        &self,
        kind:    &str,
        payload: PromptPayload,
        call_id: &str,
        ctx:     &DispatchContext,
    ) -> Result<bool, ToolError> {
        let value = serde_json::to_value(&payload).expect("PromptPayload always serialises");
        let response_value = ctx.request_prompt(call_id.to_string(), value).await;
        let response: PromptResponse = serde_json::from_value(response_value).map_err(|_| ToolError::MalformedPromptResponse)?;

        match response {
            // `pattern` is the developer's own choice of grant coarseness
            // (exact target, or a broadened `<dir>/**` glob when the prompt
            // offered one) — not necessarily `payload`'s original target.
            PromptResponse::Tool { decision, tier, pattern } => {
                self.permissions.record_tool_decision(kind, &pattern, false, decision, tier)?;
                Ok(matches!(decision, Decision::Allow))
            }
            // `check_tool` with `edit_class: false` never yields an Edit or
            // ContextFile shape, so a well-behaved caller can't produce this;
            // a malformed developer-side response still shouldn't panic.
            PromptResponse::ContextFile { .. } => Err(ToolError::MalformedPromptResponse),
        }
    }
}

#[async_trait]
impl mjolnir_core::ToolDispatcher for Dispatcher {
    async fn dispatch(&self, call: ToolCall, ctx: &DispatchContext) -> ToolResult {
        let Some(tool) = self.registry.get(&call.name) else {
            return error_result(&call.id, ToolError::UnknownTool { name: call.name });
        };
        let descriptor = tool.descriptor().clone();

        if !descriptor.edit_class {
            match self.check(&descriptor, &call, ctx).await {
                Ok(true) => {}
                Ok(false) => return error_result(&call.id, ToolError::Denied),
                Err(e) => return error_result(&call.id, e),
            }
        }

        match tool.call(&call.id, call.input, ctx).await {
            Ok(content) => ToolResult { call_id: call.id, content, is_error: false },
            Err(e) => error_result(&call.id, e),
        }
    }

    fn definitions(&self) -> Vec<ToolDefinition> {
        self.registry.definitions()
    }
}

fn error_result(call_id: &str, err: ToolError) -> ToolResult {
    ToolResult { call_id: call_id.to_string(), content: err.to_string(), is_error: true }
}

/// Permission grants persist as an opaque `kind:pattern` string
/// (mjolnir-permissions' `GrantKey::parse` splits on the *first* `:`, so
/// patterns can contain their own colons — e.g. `read:./f.rs:1`). A tool's
/// registered name is ordinarily a safe `kind`, but an MCP tool namespaced
/// under a collision (`<server>:<name>`, per mjolnir-tools.md) already
/// contains a colon itself: persisted as `server:name:pattern`, that would
/// parse back as kind `server`, pattern `name:pattern` — never matching the
/// original kind again, so an "always allow" answer would silently stop
/// taking effect on the very next call. Escaping `:` to `/` here (only ever
/// needed for namespaced MCP kinds — plain tool names never contain it)
/// keeps the grant grammar's own splitting rule correct without
/// mjolnir-permissions needing to know anything about MCP namespacing.
fn permission_kind(tool_name: &str) -> std::borrow::Cow<'_, str> {
    if tool_name.contains(':') {
        std::borrow::Cow::Owned(tool_name.replace(':', "/"))
    } else {
        std::borrow::Cow::Borrowed(tool_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ToolSource;
    use crate::test_support::dispatch_context;
    use mjolnir_config::Config;
    use mjolnir_core::{Event, ToolDispatcher as _};
    use mjolnir_permissions::ToolTier;
    use async_trait::async_trait;
    use serde_json::{json, Value};

    struct EchoTool(ToolDescriptor);

    #[async_trait]
    impl crate::registry::Tool for EchoTool {
        fn descriptor(&self) -> &ToolDescriptor {
            &self.0
        }
        fn permission_target(&self, input: &Value) -> Result<String, ToolError> {
            Ok(input.get("text").and_then(Value::as_str).unwrap_or_default().to_string())
        }
        async fn call(&self, _call_id: &str, input: Value, _gate: &dyn crate::gate::ApprovalGate) -> Result<String, ToolError> {
            Ok(input.get("text").and_then(Value::as_str).unwrap_or_default().to_string())
        }
    }

    fn echo_tool(name: &str, edit_class: bool) -> std::sync::Arc<dyn crate::registry::Tool> {
        std::sync::Arc::new(EchoTool(ToolDescriptor {
            name:         name.into(),
            description:  "echo".into(),
            input_schema: json!({}),
            edit_class,
            source:       ToolSource::Builtin,
        }))
    }

    fn engine() -> Arc<Engine> {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::open_at(dir.path(), dir.path().join("global")).unwrap();
        Arc::new(Engine::new(config))
    }

    #[tokio::test]
    async fn unknown_tool_returns_a_structured_error() {
        let dispatcher = Dispatcher::new(Registry::new(), engine());
        let (ctx, _events, _pending) = dispatch_context();

        let result = dispatcher.dispatch(ToolCall { id: "c1".into(), name: "nope".into(), input: json!({}) }, &ctx).await;
        assert!(result.is_error);
        assert!(result.content.contains("no such tool"));
    }

    #[tokio::test]
    async fn pre_granted_allow_runs_the_tool_without_prompting() {
        let mut registry = Registry::new();
        registry.register(echo_tool("echo", false)).unwrap();
        let permissions = engine();
        permissions.record_tool_decision("echo", "hi", false, Decision::Allow, ToolTier::Session).unwrap();

        let dispatcher = Dispatcher::new(registry, permissions);
        let (ctx, _events, _pending) = dispatch_context();

        let result = dispatcher.dispatch(ToolCall { id: "c1".into(), name: "echo".into(), input: json!({"text": "hi"}) }, &ctx).await;
        assert!(!result.is_error);
        assert_eq!(result.content, "hi");
    }

    #[tokio::test]
    async fn pre_denied_returns_denied_without_running_the_tool() {
        let mut registry = Registry::new();
        registry.register(echo_tool("echo", false)).unwrap();
        let permissions = engine();
        permissions.record_tool_decision("echo", "hi", false, Decision::Deny, ToolTier::Session).unwrap();

        let dispatcher = Dispatcher::new(registry, permissions);
        let (ctx, _events, _pending) = dispatch_context();

        let result = dispatcher.dispatch(ToolCall { id: "c1".into(), name: "echo".into(), input: json!({"text": "hi"}) }, &ctx).await;
        assert!(result.is_error);
        assert!(result.content.contains("denied"));
    }

    #[tokio::test]
    async fn deny_by_absence_prompts_and_records_the_response() {
        let mut registry = Registry::new();
        registry.register(echo_tool("echo", false)).unwrap();
        let permissions = engine();

        let dispatcher = Dispatcher::new(registry, permissions.clone());
        let (ctx, mut events, pending) = dispatch_context();

        let call = dispatcher.dispatch(ToolCall { id: "c1".into(), name: "echo".into(), input: json!({"text": "hi"}) }, &ctx);
        let resolve = async {
            match events.recv().await.unwrap() {
                Event::PromptRequested { call_id, payload } => {
                    let payload: PromptPayload = serde_json::from_value(payload).unwrap();
                    assert_eq!(payload, PromptPayload::Tool { kind: "echo".into(), target: "hi".into(), path_like: false });

                    let response = PromptResponse::Tool { decision: Decision::Allow, tier: ToolTier::Project, pattern: "hi".into() };
                    let Some(mjolnir_core::PendingReply::Prompt(tx)) = pending.lock().unwrap().remove(&call_id) else {
                        panic!("expected a pending Prompt entry for {call_id}");
                    };
                    tx.send(serde_json::to_value(response).unwrap()).unwrap();
                }
                other => panic!("unexpected event: {other:?}"),
            }
        };

        let (result, ()) = tokio::join!(call, resolve);
        assert!(!result.is_error);
        assert_eq!(result.content, "hi");

        // Project-tier response actually persisted.
        assert_eq!(permissions.check_tool("echo", "hi", false, false), CheckOutcome::Allow);
    }

    /// The reported bug: "directory permissions don't appear to count
    /// properly when commands are queued (approving a directory in the
    /// first request doesn't automatically approve the next request in the
    /// same directory)." Both calls of a step are dispatched concurrently,
    /// so both used to reach `check_tool` before the developer had answered
    /// anything and both were told `PromptRequired` — the grant made in
    /// answer to the first could not affect the second, whose outcome was
    /// already fixed. Now the second waits on `prompt_gate` and re-checks,
    /// so a `<dir>/**` grant covers it and it never prompts at all.
    #[tokio::test]
    async fn a_directory_grant_answered_for_one_queued_call_covers_the_others() {
        let mut registry = Registry::new();
        registry.register(echo_tool("read", false)).unwrap();
        let permissions = engine();
        let dispatcher = Dispatcher::new(registry, permissions.clone());
        let (ctx, mut events, pending) = dispatch_context();

        let first = dispatcher.dispatch(ToolCall { id: "c1".into(), name: "read".into(), input: json!({"text": "./crates/tui/src/ui.rs"}) }, &ctx);
        let second = dispatcher.dispatch(ToolCall { id: "c2".into(), name: "read".into(), input: json!({"text": "./crates/tui/src/app.rs"}) }, &ctx);

        // Answers whichever of the two won the gate, with the broadened
        // directory pattern the TUI's own scope toggle produces — the same
        // `<dir>/**` glob `App::directory_glob` builds.
        let resolve = async {
            match events.recv().await.unwrap() {
                Event::PromptRequested { call_id, .. } => {
                    let response = PromptResponse::Tool { decision: Decision::Allow, tier: ToolTier::Session, pattern: "./crates/tui/src/**".into() };
                    let Some(mjolnir_core::PendingReply::Prompt(tx)) = pending.lock().unwrap().remove(&call_id) else {
                        panic!("expected a pending Prompt entry for {call_id}");
                    };
                    tx.send(serde_json::to_value(response).unwrap()).unwrap();
                }
                other => panic!("unexpected event: {other:?}"),
            }
        };

        // Bounded, because the pre-fix failure mode is not a wrong answer
        // but a hang: the second call raised its own prompt, and with only
        // one answer sent, `join!` would wait on it forever. The timeout
        // turns that into a legible failure instead of a stuck test run.
        let joined = tokio::time::timeout(std::time::Duration::from_secs(5), async { tokio::join!(first, second, resolve) });
        let (first, second, ()) = joined.await.expect("the queued call must resolve from the grant already made, not sit waiting on a second prompt");
        assert!(!first.is_error, "the answered call must run: {}", first.content);
        assert!(!second.is_error, "the queued call must run under the grant just made for its directory: {}", second.content);
        assert!(
            events.try_recv().is_err(),
            "the queued call must not raise a second prompt for a directory the developer has already approved"
        );
    }

    #[tokio::test]
    async fn namespaced_mcp_style_tool_names_round_trip_an_always_grant() {
        let mut registry = Registry::new();
        registry.register(echo_tool("fake:echo", false)).unwrap();
        let permissions = engine();
        let dispatcher = Dispatcher::new(registry, permissions.clone());

        // First call: deny-by-absence prompts; answer "always allow".
        let (ctx, mut events, pending) = dispatch_context();
        let call = dispatcher.dispatch(ToolCall { id: "c1".into(), name: "fake:echo".into(), input: json!({"text": "hi"}) }, &ctx);
        let resolve = async {
            match events.recv().await.unwrap() {
                Event::PromptRequested { call_id, .. } => {
                    let response = PromptResponse::Tool { decision: Decision::Allow, tier: ToolTier::Always, pattern: "hi".into() };
                    let Some(mjolnir_core::PendingReply::Prompt(tx)) = pending.lock().unwrap().remove(&call_id) else {
                        panic!("expected a pending Prompt entry for {call_id}");
                    };
                    tx.send(serde_json::to_value(response).unwrap()).unwrap();
                }
                other => panic!("unexpected event: {other:?}"),
            }
        };
        let (result, ()) = tokio::join!(call, resolve);
        assert!(!result.is_error);

        // Second call, brand new DispatchContext (as if a new session):
        // must be allowed without prompting again — this is exactly what
        // broke before kind-escaping (the persisted grant's kind couldn't
        // be reconstructed from the "server:name:pattern" string, since
        // GrantKey::parse only splits on the first colon).
        let (ctx2, _events2, _pending2) = dispatch_context();
        let result2 = dispatcher.dispatch(ToolCall { id: "c2".into(), name: "fake:echo".into(), input: json!({"text": "hi"}) }, &ctx2).await;
        assert!(!result2.is_error, "expected the always-allow grant to still apply: {}", result2.content);
        assert_eq!(result2.content, "hi");
    }

    #[tokio::test]
    async fn edit_class_tool_skips_the_generic_permission_check() {
        let mut registry = Registry::new();
        registry.register(echo_tool("edit-ish", true)).unwrap();
        let permissions = engine();

        let dispatcher = Dispatcher::new(registry, permissions);
        let (ctx, _events, _pending) = dispatch_context();

        // No grant exists and nothing resolves a prompt — if the dispatcher
        // ran the generic check for this tool, dispatch would hang awaiting
        // a PromptResponse that never arrives. It doesn't hang, proving the
        // check was skipped; EchoTool ignores the gate entirely.
        let result = dispatcher.dispatch(ToolCall { id: "c1".into(), name: "edit-ish".into(), input: json!({"text": "hi"}) }, &ctx).await;
        assert!(!result.is_error);
        assert_eq!(result.content, "hi");
    }
}
