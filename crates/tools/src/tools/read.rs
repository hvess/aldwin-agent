use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::error::ToolError;
use crate::gate::ApprovalGate;
use crate::registry::{PermissionRequest, Tool, ToolDescriptor, ToolSource};
use aldwin_permissions::Class;

pub struct ReadTool {
    descriptor:   ToolDescriptor,
    project_root: PathBuf,
}

impl ReadTool {
    pub fn new(project_root: PathBuf) -> Self {
        Self {
            descriptor: ToolDescriptor {
                name:         "read".into(),
                description:  "Read a UTF-8 text file, given a path relative to the project root.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": { "path": { "type": "string" } },
                    "required": ["path"],
                }),
                edit_class: false,
                source:     ToolSource::Builtin,
            },
            project_root,
        }
    }
}

fn path_arg(input: &Value) -> Result<String, ToolError> {
    input
        .get("path")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| ToolError::InvalidInput { tool: "read".into(), message: "missing \"path\" string field".into() })
}

#[async_trait]
impl Tool for ReadTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    /// Matched against `read:`-kind grant patterns (e.g. `read:./**`) using
    /// the raw path as given by the model, not the resolved absolute path —
    /// that's what the pattern grammar examples in aldwin-permissions.md
    /// assume.
    /// `read` is a read whatever it is pointed at — the class is a property
    /// of the tool here, not something a caller declares.
    fn permission(&self, input: &Value) -> Result<PermissionRequest, ToolError> {
        Ok(PermissionRequest {
            program: "read".into(),
            class:   Class::Read,
            argv:    vec![path_arg(input)?],
        })
    }

    async fn call(&self, _call_id: &str, input: Value, _gate: &dyn ApprovalGate) -> Result<String, ToolError> {
        let path_str = path_arg(&input)?;
        let path = crate::paths::resolve_in_project(&self.project_root, &path_str)?;
        tokio::fs::read_to_string(&path).await.map_err(|source| ToolError::Io { path, source })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn reads_an_existing_file() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("hello.txt"), "hi there").unwrap();
        let tool = ReadTool::new(dir.path().to_path_buf());

        let out = tool.call("c1", json!({"path": "hello.txt"}), &crate::test_support::ALWAYS_APPROVE).await.unwrap();
        assert_eq!(out, "hi there");
    }

    #[tokio::test]
    async fn absolute_path_cannot_escape_the_project_root() {
        let dir = tempdir().unwrap();
        let tool = ReadTool::new(dir.path().to_path_buf());

        let err = tool.call("c1", json!({"path": "/etc/passwd"}), &crate::test_support::ALWAYS_APPROVE).await.unwrap_err();
        assert!(matches!(err, ToolError::PathEscapesProject { .. }));
    }

    #[tokio::test]
    async fn dot_dot_cannot_escape_the_project_root() {
        let dir = tempdir().unwrap();
        let tool = ReadTool::new(dir.path().to_path_buf());

        let err = tool.call("c1", json!({"path": "../../../../etc/passwd"}), &crate::test_support::ALWAYS_APPROVE).await.unwrap_err();
        assert!(matches!(err, ToolError::PathEscapesProject { .. }));
    }

    #[tokio::test]
    async fn missing_file_is_a_structured_io_error() {
        let dir = tempdir().unwrap();
        let tool = ReadTool::new(dir.path().to_path_buf());

        let err = tool.call("c1", json!({"path": "missing.txt"}), &crate::test_support::ALWAYS_APPROVE).await.unwrap_err();
        assert!(matches!(err, ToolError::Io { .. }));
    }

    #[test]
    fn missing_path_field_is_invalid_input() {
        let tool = ReadTool::new(PathBuf::from("."));
        let err = tool.permission(&json!({})).unwrap_err();
        assert!(matches!(err, ToolError::InvalidInput { .. }));
    }

    #[test]
    fn the_permission_request_is_always_a_read_of_the_given_path() {
        let tool = ReadTool::new(PathBuf::from("."));
        let request = tool.permission(&json!({"path": "./src/main.rs"})).unwrap();
        assert_eq!(request.program, "read");
        assert_eq!(request.class, Class::Read);
        assert_eq!(request.argv, vec!["./src/main.rs".to_string()]);
    }
}
