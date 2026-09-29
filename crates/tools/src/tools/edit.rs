use std::sync::Arc;

use aldwin_core::DispatchContext;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::error::ToolError;
use crate::paths::Workspace;
use crate::registry::{Tool, ToolDescriptor};
use crate::staging::Staging;

/// Stages one replacement in [`Staging`]. Never writes to disk: only an
/// approve at the review does (ADR 0009 §4).
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
                               Nothing is written to disk — everything staged is shown to the developer as one \
                               review, and they approve it, discard it, or leave comments on lines; after comments \
                               it stays staged, so build on it rather than staging it again. \
                               To create a new file, or fill an empty one, give an empty `before`. \
                               Stage every edit a change needs, then run what checks it; the review opens \
                               before the run. Read the file first so `before` matches exactly."
                    .into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "path":   { "type": "string" },
                        "before": { "type": "string", "description": "Exact text to replace; empty to create a new file or fill an empty one." },
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

/// The 1-based line each occurrence of `before` starts on, in one pass: a
/// short `before` can occur thousands of times in a large file. `before`
/// must not be empty: it would match between every character.
fn match_lines(text: &str, before: &str) -> Vec<usize> {
    let (mut line, mut counted) = (1, 0);
    text.match_indices(before)
        .map(|(at, _)| {
            line += text[counted..at].matches('\n').count();
            counted = at;
            line
        })
        .collect()
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
                // Only an empty `before` creates a missing file.
                None if args.before.is_empty() => Ok(args.after.clone()),
                None => Err(ToolError::Io {
                    path: path.clone(),
                    source: std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "no such file (give an empty `before` to create it)",
                    ),
                }),
                // An empty file has nothing to see, so it is filled like a
                // missing one; any other would be replaced unseen.
                Some("") if args.before.is_empty() => Ok(args.after.clone()),
                Some(_) if args.before.is_empty() => Err(ToolError::InvalidInput {
                    tool: "edit".into(),
                    message: format!(
                        "{rel} already has content, and an empty `before` only fills a new or empty file. \
                         Read it and give the text to replace"
                    ),
                }),
                Some(current) => {
                    let lines = match_lines(current, &args.before);
                    if lines.len() != 1 {
                        return Err(ToolError::AmbiguousMatch {
                            path: path.clone(),
                            lines,
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
        assert!(matches!(&err, ToolError::AmbiguousMatch { lines, .. } if lines.is_empty()));
        assert!(err.to_string().contains("Read the file again"));
        let err = tool
            .call(
                "c1",
                json!({"path": "f.rs", "before": "x", "after": "y"}),
                &ctx,
            )
            .await
            .unwrap_err();
        assert!(matches!(&err, ToolError::AmbiguousMatch { lines, .. } if *lines == [1, 2]));
        assert!(err.to_string().contains("starting on lines 1, 2"));
        assert!(staging.is_empty(), "a failed edit stages nothing");
    }

    #[test]
    fn every_match_gets_its_line_and_the_message_names_the_first_ten() {
        let text = "fn a() {\n}\n".repeat(50);
        let lines = match_lines(&text, "}");
        assert_eq!(lines, (1..=50).map(|i| i * 2).collect::<Vec<_>>());
        let message = ToolError::AmbiguousMatch {
            path: "f.rs".into(),
            lines,
        }
        .to_string();
        assert!(message.contains("occurs 50 times, starting on lines 2, 4, 6"));
        assert!(message.contains("18, 20 and 40 more."), "{message}");
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

    #[tokio::test]
    async fn an_empty_before_against_a_file_with_content_is_refused() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("f.rs"), "x\n").unwrap();
        let (tool, staging) = tool(&dir);
        let (ctx, _e, _p) = dispatch_context();

        let err = tool
            .call(
                "c1",
                json!({"path": "f.rs", "before": "", "after": "y"}),
                &ctx,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidInput { .. }));
        assert!(
            err.to_string().contains("f.rs already has content"),
            "{err}"
        );
        assert!(staging.is_empty(), "nothing is staged over the file");
    }

    #[tokio::test]
    async fn an_empty_before_fills_an_empty_file() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("f.rs"), "").unwrap();
        let (tool, staging) = tool(&dir);
        let (ctx, _e, _p) = dispatch_context();

        tool.call(
            "c1",
            json!({"path": "f.rs", "before": "", "after": "fn x() {}\n"}),
            &ctx,
        )
        .await
        .unwrap();
        assert_eq!(staging.changeset().files[0].after, "fn x() {}\n");
    }

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
