use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::ToolError;
use crate::lsp::{self, LspClient};
use crate::paths::Workspace;
use crate::registry::{Tool, ToolDescriptor};
use crate::staging::Staging;
use aldwin_core::DispatchContext;

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Op {
    Definition,
    References,
    Hover,
    Implementations,
    WorkspaceSymbols,
}

/// LSP-backed code intelligence returning JSON locations and signatures,
/// never prose (aldwin-tools.md). One sandboxed server per language, spawned
/// on first use and killed when the tool is dropped.
///
/// The server must see staged edits over disk, as `read` does: it reads disk
/// itself, which holds nothing before an approve.
pub struct ExplainTool {
    descriptor: ToolDescriptor,
    workspace: Workspace,
    staging: Arc<Staging>,
    clients: tokio::sync::Mutex<HashMap<&'static str, LspClient>>,
    /// Every file sent to a server. Kept after an approve or discard, so the
    /// next call resends disk rather than leaving the staged text.
    shown: tokio::sync::Mutex<BTreeSet<PathBuf>>,
}

impl ExplainTool {
    pub fn new(workspace: Workspace, staging: Arc<Staging>) -> Self {
        Self {
            descriptor: ToolDescriptor {
                name: "explain".into(),
                description: "LSP-backed code intelligence: definition, references, hover, implementations, workspace_symbols. \
                               Returns structured location/signature data as JSON, not prose. Positions, in and out, \
                               are 1-based: `line` is the line number as `grep -n` prints it, and `character` counts \
                               from 1 at the start of the line (in UTF-16 units, so an emoji counts as two), \
                               landing on any character inside the identifier. A position it returns can be passed \
                               straight back. Prefer it to a text search when the question is about one symbol — \
                               where it is defined, who calls it, what implements it — since it resolves imports, \
                               re-exports and same-named items that a search confuses. A `run` of `grep` is the \
                               tool for text, comments and non-code files, though while edits are staged it \
                               opens the review first."
                    .into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "op":        { "type": "string", "enum": ["definition", "references", "hover", "implementations", "workspace_symbols"] },
                        "path":      { "type": "string" },
                        "line":      { "type": "integer", "minimum": 1 },
                        "character": { "type": "integer", "minimum": 1 },
                        "query":     { "type": "string" },
                    },
                    "required": ["op"],
                }),
                observes_disk: false,
            },
            workspace,
            staging,
            clients: tokio::sync::Mutex::new(HashMap::new()),
            shown: tokio::sync::Mutex::new(BTreeSet::new()),
        }
    }

    /// For `language_id`: the files a server should hold with their text as
    /// the agent sees it (staged, shown before, `queried`), and the shown
    /// files now gone, which are forgotten here. Errors only if `queried`
    /// cannot be read.
    async fn view(
        &self,
        language_id: &str,
        queried: Option<&Path>,
    ) -> Result<(Vec<(PathBuf, String)>, Vec<PathBuf>), ToolError> {
        let mut shown = self.shown.lock().await;
        shown.extend(
            self.staging
                .changeset()
                .files
                .into_iter()
                .filter_map(|f| self.workspace.resolve(&f.path).ok()),
        );
        shown.extend(queried.map(Path::to_path_buf));
        // A loop, not a chain: each file is read with an `.await`.
        let (mut view, mut gone) = (Vec::new(), Vec::new());
        for path in shown.iter() {
            if lsp::language_for_path(path).map(|s| s.language_id) != Some(language_id) {
                continue;
            }
            let text = match self.staging.current(path) {
                Some(staged) => staged,
                None => match tokio::fs::read_to_string(path).await {
                    Ok(text) => text,
                    Err(source) if Some(path.as_path()) == queried => {
                        return Err(ToolError::Io {
                            path: path.clone(),
                            source,
                        })
                    }
                    Err(_) => {
                        gone.push(path.clone());
                        continue;
                    }
                },
            };
            view.push((path.clone(), text));
        }
        for path in &gone {
            shown.remove(path);
        }
        Ok((view, gone))
    }

    /// Sends `client` the files of `view` and closes the gone ones.
    async fn sync(
        &self,
        client: &LspClient,
        language_id: &str,
        queried: Option<&Path>,
    ) -> Result<(), ToolError> {
        let (view, gone) = self.view(language_id, queried).await?;
        for (shown, text) in view {
            client
                .sync_document(&lsp::file_uri(&shown), language_id, &text)
                .await?;
        }
        for path in gone {
            client.close_document(&lsp::file_uri(&path)).await?;
        }
        Ok(())
    }

    /// The live client for `server`; a dead one is respawned, never reused.
    async fn client_for(&self, server: &lsp::LanguageServer) -> Result<LspClient, ToolError> {
        let mut clients = self.clients.lock().await;
        if let Some(client) = clients.get(server.language_id).filter(|c| !c.is_closed()) {
            return Ok(client.clone());
        }
        let client = LspClient::spawn(server.command, server.args, &self.workspace).await?;
        clients.insert(server.language_id, client.clone());
        Ok(client)
    }
}

fn invalid(message: impl Into<String>) -> ToolError {
    ToolError::InvalidInput {
        tool: "explain".into(),
        message: message.into(),
    }
}

fn required_str<'a>(input: &'a Value, field: &'static str) -> Result<&'a str, ToolError> {
    input
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(format!("missing {field:?} string field")))
}

fn required_u64(input: &Value, field: &'static str) -> Result<u64, ToolError> {
    input
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid(format!("missing {field:?} integer field")))
}

/// A 1-based `line` or `character` from the input, as the 0-based value LSP
/// takes. 1-based to match the locations this tool returns.
fn position_field(input: &Value, field: &'static str) -> Result<u64, ToolError> {
    required_u64(input, field)?
        .checked_sub(1)
        .ok_or_else(|| invalid(format!("{field:?} is 1-based; 0 is not a position")))
}

#[async_trait]
impl Tool for ExplainTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn call(
        &self,
        _call_id: &str,
        input: Value,
        _ctx: &DispatchContext,
    ) -> Result<String, ToolError> {
        let op_value = input
            .get("op")
            .cloned()
            .ok_or_else(|| invalid("missing \"op\" field"))?;
        let op: Op = serde_json::from_value(op_value)
            .map_err(|e| invalid(format!("invalid \"op\": {e}")))?;

        if op == Op::WorkspaceSymbols {
            let query = required_str(&input, "query")?;
            let server = lsp::language_by_id("rust").expect("rust is always configured");
            let client = self.client_for(server).await?;
            self.sync(&client, server.language_id, None).await?;
            let result = client
                .request("workspace/symbol", json!({ "query": query }))
                .await?;
            return Ok(format_symbols(result));
        }

        let path_str = required_str(&input, "path")?;
        let line = position_field(&input, "line")?;
        let character = position_field(&input, "character")?;
        let path = self.workspace.resolve(path_str)?;
        let server = lsp::language_for_path(&path).ok_or_else(|| {
            invalid(format!(
                "no language server configured for {}",
                path.display()
            ))
        })?;
        let client = self.client_for(server).await?;

        self.sync(&client, server.language_id, Some(&path)).await?;
        let uri = lsp::file_uri(&path);

        let position = json!({ "line": line, "character": character });
        let text_document = json!({ "uri": uri });

        let result = match op {
            Op::Definition => client.request("textDocument/definition", json!({ "textDocument": text_document, "position": position })).await?,
            Op::Implementations => {
                client.request("textDocument/implementation", json!({ "textDocument": text_document, "position": position })).await?
            }
            Op::References => {
                client
                    .request(
                        "textDocument/references",
                        json!({ "textDocument": text_document, "position": position, "context": { "includeDeclaration": true } }),
                    )
                    .await?
            }
            Op::Hover => client.request("textDocument/hover", json!({ "textDocument": text_document, "position": position })).await?,
            Op::WorkspaceSymbols => unreachable!("handled above"),
        };

        Ok(match op {
            Op::Hover => format_hover(result),
            _ => format_locations(result),
        })
    }
}

// Responses are read as raw `Value`, not typed: `textDocument/definition`
// alone may return `Location | Location[] | LocationLink[] | null`.

#[derive(serde::Serialize, serde::Deserialize)]
struct LocationOut {
    path: String,
    line: u64,
    character: u64,
}

fn format_locations(result: Value) -> String {
    let items: Vec<Value> = match result {
        Value::Null => vec![],
        Value::Array(items) => items,
        obj @ Value::Object(_) => vec![obj],
        _ => vec![],
    };

    let locations: Vec<LocationOut> = items.iter().filter_map(location_from_value).collect();
    serde_json::to_string(&locations).unwrap_or_else(|_| "[]".to_string())
}

fn location_from_value(item: &Value) -> Option<LocationOut> {
    let obj = item.as_object()?;
    let (uri, range) = if let Some(uri) = obj.get("uri") {
        (uri.as_str()?, obj.get("range")?)
    } else {
        // LocationLink shape.
        let uri = obj.get("targetUri")?.as_str()?;
        let range = obj
            .get("targetSelectionRange")
            .or_else(|| obj.get("targetRange"))?;
        (uri, range)
    };
    let start = range.get("start")?;
    // LSP positions are 0-based; output is 1-based.
    let line = start.get("line")?.as_u64()? + 1;
    let character = start.get("character")?.as_u64()? + 1;
    Some(LocationOut {
        path: lsp::path_from_uri(uri),
        line,
        character,
    })
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SymbolOut {
    name: String,
    path: String,
    line: u64,
    character: u64,
}

fn format_symbols(result: Value) -> String {
    let items = match result {
        Value::Array(items) => items,
        _ => vec![],
    };
    let symbols: Vec<SymbolOut> = items
        .iter()
        .filter_map(|item| {
            let obj = item.as_object()?;
            let name = obj.get("name")?.as_str()?.to_string();
            let location = obj.get("location")?;
            let loc = location_from_value(location)?;
            Some(SymbolOut {
                name,
                path: loc.path,
                line: loc.line,
                character: loc.character,
            })
        })
        .collect();
    serde_json::to_string(&symbols).unwrap_or_else(|_| "[]".to_string())
}

#[derive(serde::Serialize, serde::Deserialize)]
struct HoverOut {
    signature: String,
}

fn format_hover(result: Value) -> String {
    let signature = if result.is_null() {
        String::new()
    } else {
        extract_hover_text(result.get("contents").unwrap_or(&Value::Null))
    };
    serde_json::to_string(&HoverOut { signature })
        .unwrap_or_else(|_| r#"{"signature":""}"#.to_string())
}

fn extract_hover_text(contents: &Value) -> String {
    match contents {
        Value::String(s) => s.clone(),
        Value::Object(o) => o
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        Value::Array(items) => items
            .iter()
            .map(extract_hover_text)
            .collect::<Vec<_>>()
            .join("\n---\n"),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::dispatch_context;

    fn explain_in(workspace: Workspace) -> ExplainTool {
        let staging = Arc::new(Staging::new(workspace.clone()));
        ExplainTool::new(workspace, staging)
    }

    /// Regression: after an edit the server answered about disk.
    #[tokio::test]
    async fn the_server_is_shown_staged_edits_over_the_disk() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = Workspace::new(dir.path());
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "fn old() {}\n").unwrap();
        std::fs::write(dir.path().join("src/other.rs"), "fn other() {}\n").unwrap();
        let staging = Arc::new(Staging::new(workspace.clone()));
        let tool = ExplainTool::new(workspace.clone(), staging.clone());
        let lib = workspace.resolve("src/lib.rs").unwrap();
        staging
            .edit(lib.clone(), "src/lib.rs", |_| Ok("fn new() {}\n".into()))
            .await
            .unwrap();

        // Asked about another file, the server is still shown the staged one.
        let other = workspace.resolve("src/other.rs").unwrap();
        let made = workspace.resolve("src/made.rs").unwrap();
        staging
            .edit(made.clone(), "src/made.rs", |_| Ok("fn made() {}\n".into()))
            .await
            .unwrap();

        let (view, _) = tool.view("rust", Some(&other)).await.unwrap();
        assert!(view.contains(&(lib.clone(), "fn new() {}\n".to_string())));
        assert!(view.contains(&(other.clone(), "fn other() {}\n".to_string())));
        assert!(view.contains(&(made.clone(), "fn made() {}\n".to_string())));

        // A symbol search names no file and still sees staged edits.
        let (view, _) = tool.view("rust", None).await.unwrap();
        assert!(view.contains(&(lib.clone(), "fn new() {}\n".to_string())));

        // After a discard: disk is resent, and a file only the edit made is
        // reported gone exactly once.
        staging.discard();
        let (view, gone) = tool.view("rust", Some(&other)).await.unwrap();
        assert!(view.contains(&(lib, "fn old() {}\n".to_string())));
        assert_eq!(gone, [made]);
        let (_, gone) = tool.view("rust", Some(&other)).await.unwrap();
        assert!(gone.is_empty());
    }

    #[tokio::test]
    async fn a_file_that_cannot_be_read_fails_only_when_it_is_the_one_asked_about() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = Workspace::new(dir.path());
        let tool = explain_in(workspace.clone());
        let missing = workspace.resolve("src/missing.rs").unwrap();
        assert!(matches!(
            tool.view("rust", Some(&missing)).await,
            Err(ToolError::Io { .. })
        ));
    }

    #[test]
    fn format_locations_normalises_single_location() {
        let out = format_locations(
            json!({"uri": "file:///a.rs", "range": {"start": {"line": 4, "character": 2}, "end": {"line": 4, "character": 8}}}),
        );
        let parsed: Vec<LocationOut> = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].path, "/a.rs");
        assert_eq!(parsed[0].line, 5); // 0-indexed 4 -> 1-indexed 5
        assert_eq!(parsed[0].character, 3);
    }

    #[test]
    fn format_locations_normalises_location_link_shape() {
        let out = format_locations(json!([{
            "targetUri": "file:///b.rs",
            "targetRange": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
            "targetSelectionRange": {"start": {"line": 1, "character": 4}, "end": {"line": 1, "character": 10}},
        }]));
        let parsed: Vec<LocationOut> = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed[0].path, "/b.rs");
        assert_eq!(parsed[0].line, 2);
        assert_eq!(parsed[0].character, 5);
    }

    #[test]
    fn format_locations_null_is_an_empty_array() {
        assert_eq!(format_locations(Value::Null), "[]");
    }

    #[test]
    fn format_hover_extracts_markup_content() {
        let out =
            format_hover(json!({"contents": {"kind": "markdown", "value": "fn foo() -> i32"}}));
        let parsed: HoverOut = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed.signature, "fn foo() -> i32");
    }

    #[test]
    fn format_hover_joins_marked_string_array() {
        let out = format_hover(json!({"contents": ["a", {"value": "b"}]}));
        let parsed: HoverOut = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed.signature, "a\n---\nb");
    }

    #[test]
    fn format_symbols_extracts_name_and_location() {
        let out = format_symbols(json!([{
            "name": "MyStruct",
            "location": {"uri": "file:///c.rs", "range": {"start": {"line": 9, "character": 0}, "end": {"line": 9, "character": 8}}},
        }]));
        let parsed: Vec<SymbolOut> = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed[0].name, "MyStruct");
        assert_eq!(parsed[0].path, "/c.rs");
        assert_eq!(parsed[0].line, 10);
    }

    #[tokio::test]
    async fn missing_op_is_invalid_input() {
        let tool = explain_in(Workspace::new("."));
        let err = tool
            .call("c1", json!({}), &dispatch_context().0)
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidInput { .. }));
    }

    #[tokio::test]
    async fn definition_without_path_is_invalid_input() {
        let tool = explain_in(Workspace::new("."));
        let err = tool
            .call(
                "c1",
                json!({"op": "definition", "line": 1, "character": 1}),
                &dispatch_context().0,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidInput { .. }));
    }

    #[tokio::test]
    async fn unconfigured_language_is_invalid_input_not_an_lsp_error() {
        let tool = explain_in(Workspace::new("."));
        let err = tool
            .call(
                "c1",
                json!({"op": "definition", "path": "README.md", "line": 1, "character": 1}),
                &dispatch_context().0,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidInput { .. }));
    }

    /// Regression: input was taken as 0-based while output was 1-based, so a
    /// returned location passed back landed one line off.
    #[test]
    fn a_1_based_position_is_sent_to_the_server_0_based() {
        let input = json!({"line": 1, "character": 26});
        assert_eq!(position_field(&input, "line").unwrap(), 0);
        assert_eq!(position_field(&input, "character").unwrap(), 25);
    }

    #[tokio::test]
    async fn a_zero_position_is_invalid_input_since_positions_are_1_based() {
        let tool = explain_in(Workspace::new("."));
        let err = tool
            .call(
                "c1",
                json!({"op": "definition", "path": "src/lib.rs", "line": 0, "character": 1}),
                &dispatch_context().0,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("1-based"), "{err}");
    }

    /// Against real rust-analyzer. Ignored: indexing takes seconds. Run with
    /// `cargo test -p aldwin-tools --lib tools::explain -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn real_rust_analyzer_resolves_a_definition() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname=\"fixture\"\nversion=\"0.1.0\"\nedition=\"2021\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn callee() -> i32 { 1 }\npub fn caller() -> i32 { callee() }\n",
        )
        .unwrap();

        let tool = explain_in(Workspace::new(dir.path()));

        // Poll, not a fixed sleep: indexing time varies with load.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            let out = tool
                .call(
                    "c1",
                    json!({"op": "definition", "path": "src/lib.rs", "line": 2, "character": 26}),
                    &dispatch_context().0,
                )
                .await
                .unwrap();
            let parsed: Vec<LocationOut> = serde_json::from_str(&out).unwrap();
            if parsed
                .iter()
                .any(|l| l.path.ends_with("src/lib.rs") && l.line == 1)
            {
                break;
            }
            if std::time::Instant::now() > deadline {
                panic!("rust-analyzer never resolved the definition in time; last response: {out}");
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }
}
