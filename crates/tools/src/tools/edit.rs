use std::sync::Arc;

use aldwin_core::DispatchContext;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::error::ToolError;
use crate::paths::Workspace;
use crate::registry::{Tool, ToolDescriptor};
use crate::staging::Staging;

/// Stage a single edit (path, before, after). Nothing is written here: the
/// change lands in [`Staging`], and the review — at the end of the turn, or
/// before any run that would observe it — is what writes it (ADR 0009 §4).
pub struct EditTool {
    descriptor: ToolDescriptor,
    workspace: Workspace,
    staging: Arc<Staging>,
}

impl EditTool {
    pub fn new(workspace: Workspace, staging: Arc<Staging>) -> Self {
        Self {
            descriptor: ToolDescriptor {
                name:         "edit".into(),
                description:  "Stage a change to a file: replace one exact occurrence of `before` with `after`. \
                               Nothing is written to disk — every edit staged in a turn is shown to the developer \
                               as one review, and they approve it, discard it, or leave comments on lines. \
                               To create a new file, give an empty `before`. \
                               Stage every edit a change needs, then run what checks it; the review opens \
                               before the run. Read the file first so `before` matches exactly."
                    .into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "path":   { "type": "string" },
                        "before": { "type": "string", "description": "Exact text to replace; empty to create the file." },
                        "after":  { "type": "string" },
                    },
                    "required": ["path", "before", "after"],
                }),
                observes_disk: false,
            },
            workspace,
            staging,
        }
    }
}

struct EditArgs {
    path: String,
    before: String,
    after: String,
}

fn edit_args(input: &Value) -> Result<EditArgs, ToolError> {
    let field = |name: &'static str| -> Result<String, ToolError> {
        input
            .get(name)
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| ToolError::InvalidInput {
                tool: "edit".into(),
                message: format!("missing {name:?} string field"),
            })
    };
    Ok(EditArgs {
        path: field("path")?,
        before: field("before")?,
        after: field("after")?,
    })
}

#[async_trait]
impl Tool for EditTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn call(
        &self,
        _call_id: &str,
        input: Value,
        _ctx: &DispatchContext,
    ) -> Result<String, ToolError> {
        let args = edit_args(&input)?;
        let path = self.workspace.resolve(&args.path)?;
        let rel = args.path.clone();

        self.staging
            .edit(path.clone(), &rel, |current| match current {
                // A file that does not exist is created — but only when the
                // call said so with an empty `before`; a non-empty `before`
                // against a missing file is a mistake about the file.
                None if args.before.is_empty() => Ok(args.after.clone()),
                None => Err(ToolError::Io {
                    path: path.clone(),
                    source: std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "no such file (give an empty `before` to create it)",
                    ),
                }),
                Some(current) => {
                    let count = if args.before.is_empty() {
                        0
                    } else {
                        current.matches(args.before.as_str()).count()
                    };
                    if count != 1 {
                        return Err(ToolError::AmbiguousMatch {
                            path: path.clone(),
                            count,
                        });
                    }
                    Ok(current.replacen(&args.before, &args.after, 1))
                }
            })
            .await?;
        Ok(format!(
            "staged an edit to {rel}; the developer reviews it before it is written"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::dispatch_context;
    use tempfile::tempdir;

    fn tool(dir: &tempfile::TempDir) -> (EditTool, Arc<Staging>) {
        let staging = Arc::new(Staging::new(Workspace::new(dir.path())));
        (
            EditTool::new(Workspace::new(dir.path()), staging.clone()),
            staging,
        )
    }

    #[tokio::test]
    async fn an_edit_is_staged_and_the_disk_is_untouched() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("f.rs");
        std::fs::write(&path, "fn a() {}\nfn b() {}\n").unwrap();
        let (tool, staging) = tool(&dir);
        let (ctx, _e, _p) = dispatch_context();

        let out = tool
            .call(
                "c1",
                json!({"path": "f.rs", "before": "fn a() {}", "after": "fn a() { hi(); }"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(out.starts_with("staged an edit to f.rs"));

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "fn a() {}\nfn b() {}\n",
            "nothing is written by the tool"
        );
        assert_eq!(
            staging.changeset().files[0].after,
            "fn a() { hi(); }\nfn b() {}\n"
        );
    }

    #[tokio::test]
    async fn absolute_path_cannot_escape_the_workspace() {
        let dir = tempdir().unwrap();
        let (tool, _) = tool(&dir);
        let (ctx, _e, _p) = dispatch_context();
        let err = tool
            .call(
                "c1",
                json!({"path": "/etc/passwd", "before": "root", "after": "x"}),
                &ctx,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::PathEscapesWorkspace { .. }));
    }

    #[tokio::test]
    async fn zero_or_many_matches_is_a_structured_error() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("f.rs"), "x\nx\n").unwrap();
        let (tool, staging) = tool(&dir);
        let (ctx, _e, _p) = dispatch_context();

        let err = tool
            .call(
                "c1",
                json!({"path": "f.rs", "before": "missing", "after": "y"}),
                &ctx,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::AmbiguousMatch { count: 0, .. }));
        let err = tool
            .call(
                "c1",
                json!({"path": "f.rs", "before": "x", "after": "y"}),
                &ctx,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::AmbiguousMatch { count: 2, .. }));
        assert!(staging.is_empty(), "a failed edit stages nothing");
    }

    #[tokio::test]
    async fn an_empty_before_creates_a_file_and_a_full_one_against_nothing_does_not() {
        let dir = tempdir().unwrap();
        let (tool, staging) = tool(&dir);
        let (ctx, _e, _p) = dispatch_context();

        let err = tool
            .call(
                "c1",
                json!({"path": "new.rs", "before": "fn", "after": "x"}),
                &ctx,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Io { .. }));

        tool.call(
            "c2",
            json!({"path": "new.rs", "before": "", "after": "fn x() {}\n"}),
            &ctx,
        )
        .await
        .unwrap();
        let cs = staging.changeset();
        assert_eq!(cs.files[0].before, None);
        assert_eq!(cs.files[0].after, "fn x() {}\n");
        assert!(!dir.path().join("new.rs").exists());
    }

    /// Two edits to one file in one step: the second sees the first, which
    /// is what lets a model make several changes to a file in one turn.
    #[tokio::test]
    async fn a_second_edit_to_the_same_file_builds_on_the_first() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("f.rs"), "a\nb\n").unwrap();
        let (tool, staging) = tool(&dir);
        let (ctx, _e, _p) = dispatch_context();

        tool.call(
            "c1",
            json!({"path": "f.rs", "before": "a", "after": "A"}),
            &ctx,
        )
        .await
        .unwrap();
        tool.call(
            "c2",
            json!({"path": "f.rs", "before": "b", "after": "B"}),
            &ctx,
        )
        .await
        .unwrap();
        assert_eq!(staging.changeset().files[0].after, "A\nB\n");
    }
}
