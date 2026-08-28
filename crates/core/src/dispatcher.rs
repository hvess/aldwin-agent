use async_trait::async_trait;
use crate::types::{ToolCall, ToolResult};

/// Implementors live in amundsen-tools. Approval-gated tools (Edit) block inside
/// their own dispatch future; the agent loop just awaits.
#[async_trait]
pub trait ToolDispatcher: Send + Sync {
    async fn dispatch(&self, call: ToolCall) -> ToolResult;

    /// The set of tools available to the model in the current session.
    fn definitions(&self) -> Vec<crate::types::ToolDefinition>;
}
