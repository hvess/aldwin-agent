use std::sync::Arc;

use aldwin_core::DispatchContext;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::error::ToolError;
use crate::output::{within_cap, OUTPUT_CAP_BYTES};
use crate::paths::Workspace;
use crate::registry::{Tool, ToolDescriptor};
use crate::staging::Staging;

/// Reads a file: the staged version if it has staged edits, else disk.
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
                               A file with staged edits reads back with them applied, until the review writes or discards them. \
                               `offset` and `limit` read part of it by line number. At most about 50 KB comes back, cut at \
                               a line end (a single longer line is cut, and only `run` shows the rest of it); when less than \
                               the whole file comes back, a last line in brackets names the \
                               lines shown, the file's length and the `offset` to read on from."
                    .into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "path":   { "type": "string" },
                        "offset": { "type": "integer", "minimum": 1,
                                    "description": "The first line to read, counting from 1. Defaults to 1." },
                        "limit":  { "type": "integer", "minimum": 1,
                                    "description": "How many lines to read at most. Defaults to the rest of the file." },
                    },
                    "required": ["path"],
                }),
                observes_disk: false,
            },
            workspace,
            staging,
        }
    }
}

fn invalid(message: impl Into<String>) -> ToolError {
    ToolError::InvalidInput {
        tool: "read".into(),
        message: message.into(),
    }
}

fn path_arg(input: &Value) -> Result<String, ToolError> {
    input
        .get("path")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| invalid("missing \"path\" string field"))
}

/// A line-count argument: absent, or a whole number from 1.
fn count_arg(input: &Value, name: &str) -> Result<Option<usize>, ToolError> {
    match input.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .filter(|&n| n >= 1)
            .and_then(|n| usize::try_from(n).ok())
            .map(Some)
            .ok_or_else(|| invalid(format!("\"{name}\" must be a whole number from 1"))),
    }
}

/// Lines `offset..` of `text`, at most `limit` of them and whole lines
/// within `OUTPUT_CAP_BYTES`; a first line longer than the cap is cut. Less
/// than the whole file ends with a line naming what was shown.
fn window(text: &str, offset: usize, limit: Option<usize>) -> Result<String, ToolError> {
    let total = text.lines().count();
    if offset > total.max(1) {
        return Err(invalid(format!(
            "\"offset\" {offset} is past the end: the file has {total} lines"
        )));
    }
    let lines = text
        .split_inclusive('\n')
        .skip(offset - 1)
        .take(limit.unwrap_or(usize::MAX));
    // A loop, not a chain: it keeps a running byte total and cuts only a
    // first line that is longer than the cap on its own.
    let (mut out, mut shown, mut cut) = (String::new(), 0, false);
    for line in lines {
        if out.len() + line.len() > OUTPUT_CAP_BYTES {
            if shown == 0 {
                out.push_str(within_cap(line));
                (shown, cut) = (1, true);
            }
            break;
        }
        out.push_str(line);
        shown += 1;
    }
    let last = offset + shown - 1;
    if offset == 1 && last >= total && !cut {
        return Ok(out);
    }
    let mut note = format!("lines {offset}–{last} of {total}");
    if cut {
        note.push_str(&format!(", the last cut at {OUTPUT_CAP_BYTES} bytes"));
    }
    if last < total {
        note.push_str(&format!("; to read on, give offset {}", last + 1));
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&format!("[{note}]\n"));
    Ok(out)
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
        let offset = count_arg(&input, "offset")?.unwrap_or(1);
        let limit = count_arg(&input, "limit")?;
        let path = self.workspace.resolve(&path_str)?;
        let text = match self.staging.current(&path) {
            Some(staged) => staged,
            None => tokio::fs::read_to_string(&path)
                .await
                .map_err(|source| ToolError::Io { path, source })?,
        };
        window(&text, offset, limit)
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

    fn numbered(n: usize) -> String {
        (1..=n).map(|i| format!("line {i}\n")).collect()
    }

    #[test]
    fn a_whole_file_within_the_cap_comes_back_as_it_is() {
        for text in ["", "no newline", "a\nb\n", "a\r\nb"] {
            assert_eq!(window(text, 1, None).unwrap(), text);
        }
    }

    #[test]
    fn a_range_names_its_lines_and_where_to_read_on() {
        assert_eq!(
            window(&numbered(10), 3, Some(2)).unwrap(),
            "line 3\nline 4\n[lines 3–4 of 10; to read on, give offset 5]\n"
        );
        assert_eq!(
            window(&numbered(10), 9, Some(5)).unwrap(),
            "line 9\nline 10\n[lines 9–10 of 10]\n"
        );
        assert!(window(&numbered(10), 11, None).is_err(), "past the end");
    }

    #[test]
    fn a_large_file_stops_at_the_last_whole_line_within_the_cap() {
        let text = numbered(20_000);
        let out = window(&text, 1, None).unwrap();
        let (body, note) = out.trim_end().rsplit_once('\n').unwrap();
        assert!(out.len() <= OUTPUT_CAP_BYTES + 80);
        let last = body.lines().count();
        assert!(body.ends_with(&format!("line {last}")), "whole lines only");
        assert_eq!(
            note,
            format!(
                "[lines 1–{last} of 20000; to read on, give offset {}]",
                last + 1
            )
        );
        assert!(window(&text, last + 1, None)
            .unwrap()
            .starts_with(&format!("line {}\n", last + 1)));
    }

    #[test]
    fn a_line_longer_than_the_cap_is_cut_and_says_so() {
        let text = format!("{}\nnext\n", "é".repeat(OUTPUT_CAP_BYTES));
        let out = window(&text, 1, None).unwrap();
        assert!(out.ends_with(&format!(
            "[lines 1–1 of 2, the last cut at {OUTPUT_CAP_BYTES} bytes; to read on, give offset 2]\n"
        )));
    }

    #[tokio::test]
    async fn a_range_of_a_staged_file_reads_the_staged_lines() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("f.txt");
        std::fs::write(&path, numbered(5)).unwrap();
        let (tool, staging) = tool(&dir);
        staging
            .edit(path.canonicalize().unwrap(), "f.txt", |text| {
                Ok(text.unwrap_or_default().replace("line 3", "line three"))
            })
            .await
            .unwrap();
        let (ctx, _e, _p) = dispatch_context();
        let out = tool
            .call(
                "c1",
                json!({"path": "f.txt", "offset": 3, "limit": 1}),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(
            out,
            "line three\n[lines 3–3 of 5; to read on, give offset 4]\n"
        );
        let err = tool
            .call("c1", json!({"path": "f.txt", "offset": 0}), &ctx)
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidInput { .. }));
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
