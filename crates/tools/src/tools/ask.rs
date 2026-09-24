//! `ask` — one question, one line of why, a short list of answers (ADR 0009
//! §2). The design's rule is that the list always carries a yes, a no and
//! "Chat about this"; the third is appended here if the model left it out,
//! because it is the developer's way out of a question that was wrongly
//! framed, and it is not the model's to omit.

use aldwin_core::{Answer, DispatchContext, Question};
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::error::ToolError;
use crate::registry::{PermissionRequest, Tool, ToolDescriptor, ToolSource};

/// The row every question ends with. Sentence case, the design's copy.
pub const CHAT_ABOUT_THIS: &str = "Chat about this";

/// Beyond this the list is a menu, and a menu is a sign the question was
/// not one question.
const MAX_OPTIONS: usize = 5;

pub struct AskTool {
    descriptor: ToolDescriptor,
}

impl Default for AskTool {
    fn default() -> Self {
        Self::new()
    }
}

impl AskTool {
    pub fn new() -> Self {
        Self {
            descriptor: ToolDescriptor {
                name:         "ask".into(),
                description:  "Ask the developer one question they have to answer before you can go on. One line \
                               of question, one line of why it matters, and two to four short answers that \
                               include a yes and a no. \"Chat about this\" is always offered as well; if they \
                               take it, the result is what they typed. Ask only when the answer changes what \
                               you do and you cannot settle it yourself."
                    .into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "question": { "type": "string" },
                        "detail":   { "type": "string", "description": "One line on why the answer matters." },
                        "options":  { "type": "array", "items": { "type": "string" }, "minItems": 1, "maxItems": 4 },
                    },
                    "required": ["question", "options"],
                }),
                source: ToolSource::Builtin,
            },
        }
    }
}

fn invalid(message: impl Into<String>) -> ToolError {
    ToolError::InvalidInput {
        tool: "ask".into(),
        message: message.into(),
    }
}

fn parse(input: &Value) -> Result<Question, ToolError> {
    let question = input
        .get("question")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .ok_or_else(|| invalid("missing \"question\""))?;
    let detail = input
        .get("detail")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let mut options: Vec<String> = input
        .get("options")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("missing \"options\" array"))?
        .iter()
        .map(|o| {
            o.as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .ok_or_else(|| invalid("every option is a non-empty string"))
        })
        .collect::<Result<_, _>>()?;
    if options.is_empty() {
        return Err(invalid("give at least one answer"));
    }
    if !options
        .iter()
        .any(|o| o.eq_ignore_ascii_case(CHAT_ABOUT_THIS))
    {
        options.push(CHAT_ABOUT_THIS.into());
    }
    if options.len() > MAX_OPTIONS {
        return Err(invalid(format!(
            "at most {} answers plus \"{CHAT_ABOUT_THIS}\"",
            MAX_OPTIONS - 1
        )));
    }
    Ok(Question {
        question: question.to_string(),
        detail: detail.to_string(),
        options,
    })
}

#[async_trait]
impl Tool for AskTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn permission(&self, _input: &Value) -> Result<Option<PermissionRequest>, ToolError> {
        Ok(None)
    }

    async fn call(
        &self,
        call_id: &str,
        input: Value,
        ctx: &DispatchContext,
    ) -> Result<String, ToolError> {
        let question = parse(&input)?;
        let options = question.options.clone();
        match ctx.ask(call_id.to_string(), question).await {
            Some(Answer::Chose { index }) => match options.get(index) {
                Some(option) => Ok(format!("The developer chose: {option}")),
                None => Err(invalid(format!(
                    "answer {index} is not one of the {} options",
                    options.len()
                ))),
            },
            Some(Answer::Said { text }) => Ok(format!("The developer said: {text}")),
            None => Err(ToolError::Unanswered),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::dispatch_context;
    use aldwin_core::{Event, PendingReply};

    #[tokio::test]
    async fn chat_about_this_is_always_offered_and_a_choice_comes_back_as_its_text() {
        let tool = AskTool::new();
        let (ctx, mut events, pending) = dispatch_context();

        let call = tool.call("c1", json!({"question": "Limit anonymous requests too?", "detail": "why", "options": ["Yes", "No"]}), &ctx);
        let answer = async {
            let Some(Event::QuestionAsked { call_id, question }) = events.recv().await else {
                panic!("expected a question")
            };
            assert_eq!(question.options, vec!["Yes", "No", CHAT_ABOUT_THIS]);
            let Some(PendingReply::Answer(tx)) = pending.lock().unwrap().remove(&call_id) else {
                panic!("no pending answer")
            };
            tx.send(Answer::Chose { index: 1 }).unwrap();
        };
        let (out, ()) = tokio::join!(call, answer);
        assert_eq!(out.unwrap(), "The developer chose: No");
    }

    #[tokio::test]
    async fn what_the_developer_typed_comes_back_verbatim() {
        let tool = AskTool::new();
        let (ctx, mut events, pending) = dispatch_context();
        let call = tool.call("c1", json!({"question": "Q?", "options": ["Yes"]}), &ctx);
        let answer = async {
            let Some(Event::QuestionAsked { call_id, .. }) = events.recv().await else {
                panic!()
            };
            let Some(PendingReply::Answer(tx)) = pending.lock().unwrap().remove(&call_id) else {
                panic!()
            };
            tx.send(Answer::Said {
                text: "Only for keyed requests".into(),
            })
            .unwrap();
        };
        let (out, ()) = tokio::join!(call, answer);
        assert_eq!(out.unwrap(), "The developer said: Only for keyed requests");
    }

    #[tokio::test]
    async fn a_malformed_question_asks_nothing() {
        let tool = AskTool::new();
        let (ctx, mut events, _p) = dispatch_context();
        for input in [
            json!({}),
            json!({"question": "Q?", "options": []}),
            json!({"question": "", "options": ["a"]}),
            json!({"question": "Q?", "options": ["a", "b", "c", "d", "e"]}),
        ] {
            assert!(matches!(
                tool.call("c1", input, &ctx).await,
                Err(ToolError::InvalidInput { .. })
            ));
        }
        assert!(events.try_recv().is_err());
    }
}
