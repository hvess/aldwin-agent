//! `plan`: the plan as a short list of steps (ADR 0009 §2). Each call carries
//! the whole list; the TUI draws the latest and nothing else consumes it.

use aldwin_core::{DispatchContext, PlanStep, StepState};
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::error::ToolError;
use crate::registry::{Tool, ToolDescriptor};

/// Beyond this the plan is a task list the developer cannot hold in view.
const MAX_STEPS: usize = 7;

pub struct PlanTool {
    descriptor: ToolDescriptor,
}

impl Default for PlanTool {
    fn default() -> Self {
        Self::new()
    }
}

impl PlanTool {
    pub fn new() -> Self {
        Self {
            descriptor: ToolDescriptor {
                name:         "plan".into(),
                description:  "Show the developer the plan, as outcomes in plain words — \"Count requests per key\", \
                               never a command or a file name. Call it with the whole list before starting a change \
                               that takes more than one step, and again as each step starts (`running`) and \
                               finishes (`done`). Two or three steps is usual. The plan is drawn on screen; do \
                               not list its steps again in your reply."
                    .into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "steps": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "text":  { "type": "string" },
                                    "state": { "type": "string", "enum": ["pending", "running", "done"] },
                                },
                                "required": ["text", "state"],
                            },
                        },
                    },
                    "required": ["steps"],
                }),
                observes_disk: false,
            },
        }
    }
}

fn invalid(message: impl Into<String>) -> ToolError {
    ToolError::InvalidInput {
        tool: "plan".into(),
        message: message.into(),
    }
}

fn parse(input: &Value) -> Result<Vec<PlanStep>, ToolError> {
    let items = input
        .get("steps")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("missing \"steps\" array"))?;
    if items.is_empty() {
        return Err(invalid("a plan has at least one step"));
    }
    if items.len() > MAX_STEPS {
        return Err(invalid(format!(
            "a plan has at most {MAX_STEPS} steps; fold the small ones together"
        )));
    }
    items
        .iter()
        .map(|item| {
            let text = item
                .get("text")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .ok_or_else(|| invalid("every step needs a non-empty \"text\""))?;
            let state = match item.get("state").and_then(Value::as_str) {
                Some("pending") => StepState::Pending,
                Some("running") => StepState::Running,
                Some("done") => StepState::Done,
                _ => {
                    return Err(invalid(
                        "every step needs a \"state\" of pending, running or done",
                    ))
                }
            };
            Ok(PlanStep {
                text: text.to_string(),
                state,
            })
        })
        .collect()
}

#[async_trait]
impl Tool for PlanTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn call(
        &self,
        _call_id: &str,
        input: Value,
        ctx: &DispatchContext,
    ) -> Result<String, ToolError> {
        let steps = parse(&input)?;
        let running = steps
            .iter()
            .filter(|s| s.state == StepState::Running)
            .count();
        let done = steps.iter().filter(|s| s.state == StepState::Done).count();
        let total = steps.len();
        ctx.plan_updated(steps).await;
        Ok(format!(
            "plan shown: {done} of {total} done, {running} running"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::dispatch_context;
    use aldwin_core::Event;

    #[tokio::test]
    async fn the_plan_reaches_the_screen_as_typed_steps() {
        let tool = PlanTool::new();
        let (ctx, mut events, _p) = dispatch_context();
        let out = tool
            .call("c1", json!({"steps": [{"text": "Count requests per key", "state": "done"}, {"text": "Check that it works", "state": "running"}]}), &ctx)
            .await
            .unwrap();
        assert_eq!(out, "plan shown: 1 of 2 done, 1 running");
        match events.try_recv().unwrap() {
            Event::PlanUpdated { steps, .. } => {
                assert_eq!(
                    steps[0],
                    PlanStep {
                        text: "Count requests per key".into(),
                        state: StepState::Done
                    }
                );
                assert_eq!(steps[1].state, StepState::Running);
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn a_malformed_plan_is_refused_and_shows_nothing() {
        let tool = PlanTool::new();
        let (ctx, mut events, _p) = dispatch_context();
        for input in [
            json!({}),
            json!({"steps": []}),
            json!({"steps": [{"text": "", "state": "done"}]}),
            json!({"steps": [{"text": "x", "state": "soon"}]}),
        ] {
            assert!(matches!(
                tool.call("c1", input, &ctx).await,
                Err(ToolError::InvalidInput { .. })
            ));
        }
        let eight: Vec<Value> = (0..8)
            .map(|i| json!({"text": format!("step {i}"), "state": "pending"}))
            .collect();
        assert!(matches!(
            tool.call("c1", json!({"steps": eight}), &ctx).await,
            Err(ToolError::InvalidInput { .. })
        ));
        assert!(events.try_recv().is_err(), "nothing reached the screen");
    }
}
