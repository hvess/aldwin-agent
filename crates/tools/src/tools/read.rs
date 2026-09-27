use std::sync::Arc;

use aldwin_core::DispatchContext;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::error::ToolError;
use crate::paths::Workspace;
use crate::registry::{Tool, ToolDescriptor};
use crate::staging::Staging;

/// Reads a file: the staged version if this turn edited it, else disk.
pub struct ReadTool {
    descriptor: ToolDescriptor,
    workspace: Workspace,
    staging: Arc<Staging>,
}

impl ReadTool {
    pub fn new(workspace: Workspace, staging: Arc<Staging>) -> Self {
        Self {
            descriptor: ToolDescriptor {
                name:         "read".into(),
                description:  "Read a UTF-8 text file, given a path relative to the project root. \
                               A file you have staged an edit to this turn reads back with the edit applied."
                    .into(),
                input_schema: json!({
                    "type": "object",
                    "properties": { "path": { "type": "string" } },
                    "required": ["path"],
                }),
                observes_disk: false,
            },
            workspace,
            staging,
        }
    }
}

fn path_arg(input: &Value) -> Result<String, ToolError> {
    input
        .get("path")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| ToolError::InvalidInput {
            tool: "read".into(),
            message: "missing \"path\" string field".into(),
        })
}

#[async_trait]
impl Tool for ReadTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn call(
        &self,
        _call_id: &str,
        input: Value,
        _ctx: &DispatchContext,
    ) -> Result<String, ToolError> {
        let path_str = path_arg(&input)?;
        let path = self.workspace.resolve(&path_str)?;
        if let Some(staged) = self.staging.current(&path) {
            return Ok(staged);
        }
        tokio::fs::read_to_string(&path)
            .await
            .map_err(|source| ToolError::Io { path, source })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::dispatch_context;
    use tempfile::tempdir;

    fn tool(dir: &tempfile::TempDir) -> (ReadTool, Arc<Staging>) {
        let staging = Arc::new(Staging::new(Workspace::new(dir.path())));
        (
            ReadTool::new(Workspace::new(dir.path()), staging.clone()),
            staging,
        )
    }

    #[tokio::test]
    async fn reads_an_existing_file() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("hello.txt"), "hi there").unwrap();
        let (tool, _) = tool(&dir);
        let (ctx, _e, _p) = dispatch_context();
        assert_eq!(
            tool.call("c1", json!({"path": "hello.txt"}), &ctx)
                .await
                .unwrap(),
            "hi there"
        );
    }

    #[tokio::test]
    async fn a_staged_edit_is_what_reads_back() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("hello.txt");
        std::fs::write(&path, "hi there").unwrap();
        let (tool, staging) = tool(&dir);
        staging
            .edit(path.canonicalize().unwrap(), "hello.txt", |_| {
                Ok("hi, staged".into())
            })
            .await
            .unwrap();
        let (ctx, _e, _p) = dispatch_context();
        assert_eq!(
            tool.call("c1", json!({"path": "hello.txt"}), &ctx)
                .await
                .unwrap(),
            "hi, staged"
        );
    }

    #[tokio::test]
    async fn absolute_and_dot_dot_paths_cannot_escape_the_workspace() {
        let dir = tempdir().unwrap();
        let (tool, _) = tool(&dir);
        let (ctx, _e, _p) = dispatch_context();
        for path in ["/etc/passwd", "../../../../etc/passwd"] {
            let err = tool
                .call("c1", json!({"path": path}), &ctx)
                .await
                .unwrap_err();
            assert!(
                matches!(err, ToolError::PathEscapesWorkspace { .. }),
                "{path}"
            );
        }
    }

    #[tokio::test]
    async fn missing_file_is_a_structured_io_error() {
        let dir = tempdir().unwrap();
        let (tool, _) = tool(&dir);
        let (ctx, _e, _p) = dispatch_context();
        let err = tool
            .call("c1", json!({"path": "missing.txt"}), &ctx)
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Io { .. }));
    }
}
