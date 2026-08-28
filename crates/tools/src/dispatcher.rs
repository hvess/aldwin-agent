use std::sync::Arc;

use amundsen_core::{DispatchContext, ToolCall, ToolDefinition, ToolResult};
use amundsen_permissions::{CheckOutcome, Decision, Engine, PromptPayload, PromptResponse};
use async_trait::async_trait;

use crate::error::ToolError;
use crate::registry::{Registry, ToolDescriptor};

/// Implements core's `ToolDispatcher`. Owns the dispatch flow described in
/// amundsen-tools.md: resolve name -> tool, run the generic permission check
/// for `edit_class: false` tools (emitting `PromptRequested` / awaiting
/// `PromptResponse` on Deny-by-absence), then run the tool's future.
/// `edit_class: true` tools skip the permission check entirely — their
/// approval gate lives inside their own future (see `gate.rs`).
pub struct Dispatcher {
    registry:    Registry,
    permissions: Arc<Engine>,
}

impl Dispatcher {
    pub fn new(registry: Registry, permissions: Arc<Engine>) -> Self {
        Self { registry, permissions }
    }

    /// `Ok(true)` if the call may proceed, `Ok(false)` if denied (initially
    /// or by the developer's prompt response). Only called for `edit_class:
    /// false` tools — see module doc.
    async fn check(&self, descriptor: &ToolDescriptor, call: &ToolCall, ctx: &DispatchContext) -> Result<bool, ToolError> {
        let tool = self.registry.get(&call.name).expect("caller already resolved this name");
        let target = tool.permission_target(&call.input)?;

        match self.permissions.check_tool(&descriptor.name, &target, false) {
            CheckOutcome::Allow => Ok(true),
            CheckOutcome::Deny => Ok(false),
            CheckOutcome::PromptRequired(payload) => self.prompt_and_record(&descriptor.name, &target, payload, ctx).await,
        }
    }

    async fn prompt_and_record(
        &self,
        kind:    &str,
        target:  &str,
        payload: PromptPayload,
        ctx:     &DispatchContext,
    ) -> Result<bool, ToolError> {
        let value = serde_json::to_value(&payload).expect("PromptPayload always serialises");
        let response_value = ctx.request_prompt(value).await;
        let response: PromptResponse = serde_json::from_value(response_value).map_err(|_| ToolError::MalformedPromptResponse)?;

        match response {
            PromptResponse::Tool { decision, tier } => {
                self.permissions.record_tool_decision(kind, target, false, decision, tier)?;
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
impl amundsen_core::ToolDispatcher for Dispatcher {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ToolSource;
    use crate::test_support::dispatch_context;
    use amundsen_config::Config;
    use amundsen_core::{Event, ToolDispatcher as _};
    use amundsen_permissions::ToolTier;
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
        let (ctx, _events, _approvals, _prompts) = dispatch_context();

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
        let (ctx, _events, _approvals, _prompts) = dispatch_context();

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
        let (ctx, _events, _approvals, _prompts) = dispatch_context();

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
        let (ctx, mut events, _approvals, prompts) = dispatch_context();

        let call = dispatcher.dispatch(ToolCall { id: "c1".into(), name: "echo".into(), input: json!({"text": "hi"}) }, &ctx);
        let resolve = async {
            match events.recv().await.unwrap() {
                Event::PromptRequested { id, payload } => {
                    let payload: PromptPayload = serde_json::from_value(payload).unwrap();
                    assert_eq!(payload, PromptPayload::Tool { kind: "echo".into(), target: "hi".into() });

                    let response = PromptResponse::Tool { decision: Decision::Allow, tier: ToolTier::Project };
                    let tx = prompts.lock().unwrap().remove(&id.0).unwrap();
                    tx.send(serde_json::to_value(response).unwrap()).unwrap();
                }
                other => panic!("unexpected event: {other:?}"),
            }
        };

        let (result, ()) = tokio::join!(call, resolve);
        assert!(!result.is_error);
        assert_eq!(result.content, "hi");

        // Project-tier response actually persisted.
        assert_eq!(permissions.check_tool("echo", "hi", false), CheckOutcome::Allow);
    }

    #[tokio::test]
    async fn edit_class_tool_skips_the_generic_permission_check() {
        let mut registry = Registry::new();
        registry.register(echo_tool("edit-ish", true)).unwrap();
        let permissions = engine();

        let dispatcher = Dispatcher::new(registry, permissions);
        let (ctx, _events, _approvals, _prompts) = dispatch_context();

        // No grant exists and nothing resolves a prompt — if the dispatcher
        // ran the generic check for this tool, dispatch would hang awaiting
        // a PromptResponse that never arrives. It doesn't hang, proving the
        // check was skipped; EchoTool ignores the gate entirely.
        let result = dispatcher.dispatch(ToolCall { id: "c1".into(), name: "edit-ish".into(), input: json!({"text": "hi"}) }, &ctx).await;
        assert!(!result.is_error);
        assert_eq!(result.content, "hi");
    }
}
