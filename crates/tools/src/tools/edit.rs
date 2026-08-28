use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::diff;
use crate::error::ToolError;
use crate::gate::ApprovalGate;
use crate::registry::{Tool, ToolDescriptor, ToolSource};

/// Propose a single edit (path, before, after). Always per-call approval —
/// the gate lives inside this future, not the dispatcher (see
/// amundsen-tools.md's Decisions). `edit_class: true` means the generic
/// dispatcher permission check is never even consulted for this tool.
pub struct EditTool {
    descriptor:   ToolDescriptor,
    project_root: PathBuf,
}

impl EditTool {
    pub fn new(project_root: PathBuf) -> Self {
        Self {
            descriptor: ToolDescriptor {
                name:         "edit".into(),
                description:  "Replace one exact occurrence of `before` with `after` in a file, after developer approval.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "path":   { "type": "string" },
                        "before": { "type": "string" },
                        "after":  { "type": "string" },
                    },
                    "required": ["path", "before", "after"],
                }),
                edit_class: true,
                source:     ToolSource::Builtin,
            },
            project_root,
        }
    }
}

struct EditArgs {
    path:   String,
    before: String,
    after:  String,
}

fn edit_args(input: &Value) -> Result<EditArgs, ToolError> {
    let field = |name: &'static str| -> Result<String, ToolError> {
        input
            .get(name)
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| ToolError::InvalidInput { tool: "edit".into(), message: format!("missing {name:?} string field") })
    };
    Ok(EditArgs { path: field("path")?, before: field("before")?, after: field("after")? })
}

#[async_trait]
impl Tool for EditTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    /// Never consulted — `edit_class: true` routes this tool around the
    /// generic four-tier check entirely.
    fn permission_target(&self, _input: &Value) -> Result<String, ToolError> {
        Ok(String::new())
    }

    async fn call(&self, call_id: &str, input: Value, gate: &dyn ApprovalGate) -> Result<String, ToolError> {
        let args = edit_args(&input)?;
        let path = self.project_root.join(&args.path);

        let current = tokio::fs::read_to_string(&path).await.map_err(|source| ToolError::Io { path: path.clone(), source })?;

        let count = current.matches(args.before.as_str()).count();
        if count != 1 {
            return Err(ToolError::AmbiguousMatch { path, count });
        }

        let updated = current.replacen(&args.before, &args.after, 1);
        let rendered_diff = diff::unified(&args.path, &args.before, &args.after);

        if !gate.request_approval(call_id.to_string(), rendered_diff).await {
            return Err(ToolError::Denied);
        }

        tokio::fs::write(&path, &updated).await.map_err(|source| ToolError::Io { path, source })?;
        Ok(format!("edited {}", args.path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{dispatch_context, ALWAYS_APPROVE, ALWAYS_DENY};
    use amundsen_core::Event;
    use tempfile::tempdir;

    fn write(dir: &tempfile::TempDir, name: &str, content: &str) -> PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, content).unwrap();
        path
    }

    #[tokio::test]
    async fn approved_edit_writes_the_replacement() {
        let dir = tempdir().unwrap();
        write(&dir, "f.rs", "fn a() {}\nfn b() {}\n");
        let tool = EditTool::new(dir.path().to_path_buf());

        let out = tool
            .call("c1", json!({"path": "f.rs", "before": "fn a() {}", "after": "fn a() { println!(\"hi\"); }"}), &ALWAYS_APPROVE)
            .await
            .unwrap();
        assert_eq!(out, "edited f.rs");

        let on_disk = std::fs::read_to_string(dir.path().join("f.rs")).unwrap();
        assert_eq!(on_disk, "fn a() { println!(\"hi\"); }\nfn b() {}\n");
    }

    #[tokio::test]
    async fn denied_edit_leaves_the_file_untouched() {
        let dir = tempdir().unwrap();
        write(&dir, "f.rs", "fn a() {}\n");
        let tool = EditTool::new(dir.path().to_path_buf());

        let err = tool.call("c1", json!({"path": "f.rs", "before": "fn a() {}", "after": "fn a() { x(); }"}), &ALWAYS_DENY).await.unwrap_err();
        assert!(matches!(err, ToolError::Denied));

        let on_disk = std::fs::read_to_string(dir.path().join("f.rs")).unwrap();
        assert_eq!(on_disk, "fn a() {}\n");
    }

    #[tokio::test]
    async fn zero_matches_is_a_structured_error_not_a_prompt() {
        let dir = tempdir().unwrap();
        write(&dir, "f.rs", "fn a() {}\n");
        let tool = EditTool::new(dir.path().to_path_buf());

        let err = tool.call("c1", json!({"path": "f.rs", "before": "fn missing() {}", "after": "x"}), &ALWAYS_APPROVE).await.unwrap_err();
        assert!(matches!(err, ToolError::AmbiguousMatch { count: 0, .. }));
    }

    #[tokio::test]
    async fn multiple_matches_is_ambiguous() {
        let dir = tempdir().unwrap();
        write(&dir, "f.rs", "x\nx\n");
        let tool = EditTool::new(dir.path().to_path_buf());

        let err = tool.call("c1", json!({"path": "f.rs", "before": "x", "after": "y"}), &ALWAYS_APPROVE).await.unwrap_err();
        assert!(matches!(err, ToolError::AmbiguousMatch { count: 2, .. }));
    }

    /// Full round trip against the real `DispatchContext`, not the fixed
    /// fake — proves the tool actually drives `ToolApprovalRequested` /
    /// resolves via the `approvals` map the way `Agent`'s command loop does.
    #[tokio::test]
    async fn drives_the_real_approval_round_trip() {
        let dir = tempdir().unwrap();
        write(&dir, "f.rs", "old\n");
        let tool = EditTool::new(dir.path().to_path_buf());

        let (ctx, mut events, approvals, _prompts) = dispatch_context();

        let call = tool.call("call-1", json!({"path": "f.rs", "before": "old", "after": "new"}), &ctx);
        let resolve = async {
            match events.recv().await.unwrap() {
                Event::ToolApprovalRequested { call_id, diff, .. } => {
                    assert_eq!(call_id, "call-1");
                    assert!(diff.contains("-old"));
                    assert!(diff.contains("+new"));
                    let tx = approvals.lock().unwrap().remove(&call_id).unwrap();
                    tx.send(true).unwrap();
                }
                other => panic!("unexpected event: {other:?}"),
            }
        };

        let (result, ()) = tokio::join!(call, resolve);
        assert_eq!(result.unwrap(), "edited f.rs");
    }
}
