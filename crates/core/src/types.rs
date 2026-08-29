use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TURN: AtomicU64 = AtomicU64::new(1);
static NEXT_STEP: AtomicU64 = AtomicU64::new(1);
static NEXT_PROMPT: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TurnId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StepId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PromptId(pub u64);

impl TurnId {
    pub fn next() -> Self { Self(NEXT_TURN.fetch_add(1, Ordering::Relaxed)) }
}

impl StepId {
    pub fn next() -> Self { Self(NEXT_STEP.fetch_add(1, Ordering::Relaxed)) }
}

impl PromptId {
    pub fn next() -> Self { Self(NEXT_PROMPT.fetch_add(1, Ordering::Relaxed)) }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role { User, Assistant }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id:    String,
    pub name:  String,
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub call_id:  String,
    pub content:  String,
    pub is_error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text       { text: String },
    ToolUse    (ToolCall),
    ToolResult (ToolResult),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role:    Role,
    pub content: Vec<ContentBlock>,
}

impl Message {
    pub fn user(text: impl Into<String>) -> Self {
        Self { role: Role::User, content: vec![ContentBlock::Text { text: text.into() }] }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name:         String,
    pub description:  String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageStats {
    pub input_tokens:  u32,
    pub output_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheStats {
    pub cache_creation_input_tokens: u32,
    pub cache_read_input_tokens:     u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StopReason { EndTurn, ToolUse }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetryInfo {
    pub provider: String,
    pub status:   Option<u16>,
    pub message:  String,
    pub attempt:  u32,
}
