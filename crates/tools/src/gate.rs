use async_trait::async_trait;

/// The one thing an approval-gated tool's future needs from core's
/// `DispatchContext` — per amundsen-tools.md, "Edit approval gate lives
/// inside the tool's future, not in the dispatcher." Factored out as a trait
/// (rather than passing `&DispatchContext` straight through) so this crate's
/// own tests can supply a fake without needing core's `DispatchContext`
/// constructor at all; `Dispatcher::dispatch` passes the real one in
/// production, coerced to `&dyn ApprovalGate` via the blanket impl below.
#[async_trait]
pub trait ApprovalGate: Send + Sync {
    async fn request_approval(&self, call_id: String, diff: String) -> bool;
}

#[async_trait]
impl ApprovalGate for amundsen_core::DispatchContext {
    async fn request_approval(&self, call_id: String, diff: String) -> bool {
        // Resolves to the inherent method — inherent impls always win over
        // trait impls on the same receiver, so this is not recursive.
        amundsen_core::DispatchContext::request_approval(self, call_id, diff).await
    }
}
