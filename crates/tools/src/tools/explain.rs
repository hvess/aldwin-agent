use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::ToolError;
use crate::gate::ApprovalGate;
use crate::lsp::{self, LspClient};
use crate::registry::{PermissionRequest, Tool, ToolDescriptor, ToolSource};
use aldwin_permissions::Class;

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Op {
    Definition,
    References,
    Hover,
    Implementations,
    WorkspaceSymbols,
}

/// LSP-backed code intelligence. Output is structured location and
/// signature data only (JSON text) — no prose summaries, per
/// aldwin-tools.md. Servers spawn lazily per-language on first use and
/// persist in `clients` for the tool's (i.e. the session's) lifetime.
pub struct ExplainTool {
    descriptor:   ToolDescriptor,
    project_root: PathBuf,
    clients:      tokio::sync::Mutex<HashMap<&'static str, Arc<LspClient>>>,
}

impl ExplainTool {
    pub fn new(project_root: PathBuf) -> Self {
        Self {
            descriptor: ToolDescriptor {
                name: "explain".into(),
                description: "LSP-backed code intelligence: definition, references, hover, implementations, workspace_symbols. \
                               Returns structured location/signature data as JSON, not prose."
                    .into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "op":        { "type": "string", "enum": ["definition", "references", "hover", "implementations", "workspace_symbols"] },
                        "path":      { "type": "string" },
                        "line":      { "type": "integer", "minimum": 0 },
                        "character": { "type": "integer", "minimum": 0 },
                        "query":     { "type": "string" },
                    },
                    "required": ["op"],
                }),
                edit_class: false,
                source:     ToolSource::Builtin,
            },
            project_root,
            clients: tokio::sync::Mutex::new(HashMap::new()),
        }
    }

    /// Best-effort graceful shutdown of every spawned server — call this
    /// from the process's own shutdown sequence (aldwin-cli). Each
    /// client's process is also `kill_on_drop`, so this isn't the only
    /// thing standing between a spawned server and process exit.
    pub async fn shutdown_all(&self) {
        let clients = self.clients.lock().await;
        for client in clients.values() {
            client.shutdown().await;
        }
    }

    async fn client_for_language(&self, language_id: &'static str, command: &'static str, args: &'static [&'static str]) -> Result<Arc<LspClient>, ToolError> {
        let mut clients = self.clients.lock().await;
        if let Some(client) = clients.get(language_id) {
            return Ok(client.clone());
        }
        let client = Arc::new(LspClient::spawn(command, args, &self.project_root).await?);
        clients.insert(language_id, client.clone());
        Ok(client)
    }
}

fn invalid(message: impl Into<String>) -> ToolError {
    ToolError::InvalidInput { tool: "explain".into(), message: message.into() }
}

fn required_str<'a>(input: &'a Value, field: &'static str) -> Result<&'a str, ToolError> {
    input.get(field).and_then(Value::as_str).ok_or_else(|| invalid(format!("missing {field:?} string field")))
}

fn required_u64(input: &Value, field: &'static str) -> Result<u64, ToolError> {
    input.get(field).and_then(Value::as_u64).ok_or_else(|| invalid(format!("missing {field:?} integer field")))
}

#[async_trait]
impl Tool for ExplainTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    /// Like `read`, `explain` only ever observes. `argv` carries whichever
    /// of the two shapes the call used, so the prompt can show what is being
    /// looked at.
    fn permission(&self, input: &Value) -> Result<PermissionRequest, ToolError> {
        let subject = if let Some(path) = input.get("path").and_then(Value::as_str) {
            path.to_string()
        } else if let Some(query) = input.get("query").and_then(Value::as_str) {
            query.to_string()
        } else {
            return Err(invalid("requires either \"path\" or \"query\""));
        };
        Ok(PermissionRequest { program: "explain".into(), class: Class::Read, argv: vec![subject] })
    }

    async fn call(&self, _call_id: &str, input: Value, _gate: &dyn ApprovalGate) -> Result<String, ToolError> {
        let op_value = input.get("op").cloned().ok_or_else(|| invalid("missing \"op\" field"))?;
        let op: Op = serde_json::from_value(op_value).map_err(|e| invalid(format!("invalid \"op\": {e}")))?;

        if op == Op::WorkspaceSymbols {
            let query = required_str(&input, "query")?;
            let server = lsp::language_by_id("rust").expect("rust is always configured");
            let client = self.client_for_language(server.language_id, server.command, server.args).await?;
            let result = client.request("workspace/symbol", json!({ "query": query })).await?;
            return Ok(format_symbols(result));
        }

        let path_str = required_str(&input, "path")?;
        let line = required_u64(&input, "line")?;
        let character = required_u64(&input, "character")?;
        let path = crate::paths::resolve_in_project(&self.project_root, path_str)?;
        let server = lsp::language_for_path(&path).ok_or_else(|| invalid(format!("no language server configured for {}", path.display())))?;
        let client = self.client_for_language(server.language_id, server.command, server.args).await?;

        let uri = format!("file://{}", path.display());
        let text = tokio::fs::read_to_string(&path).await.map_err(|source| ToolError::Io { path: path.clone(), source })?;
        client.ensure_open(&uri, server.language_id, &text).await?;

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

// ── Response normalisation ──────────────────────────────────────────────
//
// LSP responses here are handled as raw `serde_json::Value` rather than
// strict typed shapes: `textDocument/definition` alone can legally return
// `Location | Location[] | LocationLink[] | null` depending on server and
// client capabilities, and pinning that down with an untagged enum is more
// fragile than just reading the fields that are actually present.

#[derive(serde::Serialize, serde::Deserialize)]
struct LocationOut {
    path:      String,
    line:      u64,
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
        let range = obj.get("targetSelectionRange").or_else(|| obj.get("targetRange"))?;
        (uri, range)
    };
    let start = range.get("start")?;
    // LSP positions are 0-indexed; +1 for the conventional path:line:col a
    // developer or model actually wants to read.
    let line = start.get("line")?.as_u64()? + 1;
    let character = start.get("character")?.as_u64()? + 1;
    Some(LocationOut { path: uri_to_path(uri), line, character })
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SymbolOut {
    name:      String,
    path:      String,
    line:      u64,
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
            Some(SymbolOut { name, path: loc.path, line: loc.line, character: loc.character })
        })
        .collect();
    serde_json::to_string(&symbols).unwrap_or_else(|_| "[]".to_string())
}

#[derive(serde::Serialize, serde::Deserialize)]
struct HoverOut {
    signature: String,
}

fn format_hover(result: Value) -> String {
    let signature = if result.is_null() { String::new() } else { extract_hover_text(result.get("contents").unwrap_or(&Value::Null)) };
    serde_json::to_string(&HoverOut { signature }).unwrap_or_else(|_| r#"{"signature":""}"#.to_string())
}

fn extract_hover_text(contents: &Value) -> String {
    match contents {
        Value::String(s) => s.clone(),
        Value::Object(o) => o.get("value").and_then(Value::as_str).unwrap_or_default().to_string(),
        Value::Array(items) => items.iter().map(extract_hover_text).collect::<Vec<_>>().join("\n---\n"),
        _ => String::new(),
    }
}

fn uri_to_path(uri: &str) -> String {
    uri.strip_prefix("file://").unwrap_or(uri).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_target_prefers_path_then_query() {
        let tool = ExplainTool::new(PathBuf::from("."));
        let by_path = tool.permission(&json!({"path": "src/main.rs"})).unwrap();
        assert_eq!(by_path.argv, vec!["src/main.rs".to_string()]);
        assert_eq!(by_path.class, Class::Read);
        assert_eq!(tool.permission(&json!({"query": "MyStruct"})).unwrap().argv, vec!["MyStruct".to_string()]);
        assert!(tool.permission(&json!({})).is_err());
    }

    #[test]
    fn format_locations_normalises_single_location() {
        let out = format_locations(json!({"uri": "file:///a.rs", "range": {"start": {"line": 4, "character": 2}, "end": {"line": 4, "character": 8}}}));
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
        let out = format_hover(json!({"contents": {"kind": "markdown", "value": "fn foo() -> i32"}}));
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
        let tool = ExplainTool::new(PathBuf::from("."));
        let err = tool.call("c1", json!({}), &crate::test_support::ALWAYS_APPROVE).await.unwrap_err();
        assert!(matches!(err, ToolError::InvalidInput { .. }));
    }

    #[tokio::test]
    async fn definition_without_path_is_invalid_input() {
        let tool = ExplainTool::new(PathBuf::from("."));
        let err = tool.call("c1", json!({"op": "definition", "line": 0, "character": 0}), &crate::test_support::ALWAYS_APPROVE).await.unwrap_err();
        assert!(matches!(err, ToolError::InvalidInput { .. }));
    }

    #[tokio::test]
    async fn unconfigured_language_is_invalid_input_not_an_lsp_error() {
        let tool = ExplainTool::new(PathBuf::from("."));
        let err = tool
            .call("c1", json!({"op": "definition", "path": "README.md", "line": 0, "character": 0}), &crate::test_support::ALWAYS_APPROVE)
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidInput { .. }));
    }

    /// Full round trip against real rust-analyzer, including waiting for it
    /// to index a tiny fixture crate enough to answer `textDocument/
    /// definition` correctly. Ignored by default — indexing (even for a
    /// trivial crate) can take several seconds, too slow/flaky for a
    /// default test run. Run explicitly with:
    ///   cargo test -p aldwin-tools --lib tools::explain -- --ignored
    #[tokio::test]
    #[ignore]
    async fn real_rust_analyzer_resolves_a_definition() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname=\"fixture\"\nversion=\"0.1.0\"\nedition=\"2021\"\n").unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "pub fn callee() -> i32 { 1 }\npub fn caller() -> i32 { callee() }\n").unwrap();

        let tool = ExplainTool::new(dir.path().to_path_buf());

        // Poll until rust-analyzer has indexed enough to answer, rather than
        // a fixed sleep — indexing time varies with machine load.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            let out = tool
                .call("c1", json!({"op": "definition", "path": "src/lib.rs", "line": 1, "character": 25}), &crate::test_support::ALWAYS_APPROVE)
                .await
                .unwrap();
            let parsed: Vec<LocationOut> = serde_json::from_str(&out).unwrap();
            if parsed.iter().any(|l| l.path.ends_with("src/lib.rs") && l.line == 1) {
                break;
            }
            if std::time::Instant::now() > deadline {
                panic!("rust-analyzer never resolved the definition in time; last response: {out}");
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }

        tool.shutdown_all().await;
    }
}
