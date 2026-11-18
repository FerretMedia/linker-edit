// The "linker" MCP server agents get: Linker-Edit's own Resolve tools, transcription and the library. It runs as
// the agent's child process (`linker-edit mcp`) and hands each call to the Linker-Edit server, which owns the
// one connection to Resolve, asks the user before changes, and owns the GPU queue and the transcript cache.
// Plain JSON-RPC lines on stdio, as the TypeScript wrote them: the same tools, schemas and answers.

use std::path::Path;
use std::sync::Arc;

use base64::Engine as _;
use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::Mutex;
use tokio::task::JoinSet;

use super::api::Api;

/// A Resolve tool (resolve/tools.ts's RESOLVE_TOOLS), as the linker server lists it.
#[derive(Debug, Clone)]
pub struct LinkerResolveTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    /// kind === "read".
    pub read_only: bool,
    /// In CORE_TOOLS: Claude loads it up front; the rest wait behind its tool search.
    pub core: bool,
}

/// The tools the server lists, by the module that owns them. The Resolve, library, study and collections
/// ports own these lists; `catalog()` puts them together.
#[derive(Debug, Clone, Default)]
pub struct LinkerToolCatalog {
    /// RESOLVE_TOOLS, sent to /api/resolve-tool.
    pub resolve: Vec<LinkerResolveTool>,
    /// LIBRARY_TOOLS (full MCP tool definitions), sent to /api/library-tool.
    pub library: Vec<Value>,
    /// STUDY_TOOLS, sent to /api/study-tool.
    pub study: Vec<Value>,
    /// BRIEF_TOOLS, sent to /api/brief-tool.
    pub brief: Vec<Value>,
}

/// The tools of the other ports. Until the hub wires them in, only transcribe is listed.
// Hub: fill from crate::resolve::tools (RESOLVE_TOOLS, CORE_TOOLS), crate::library::tools (LIBRARY_TOOLS),
// crate::study (STUDY_TOOLS) and crate::collections::tools (BRIEF_TOOLS). CI checks that tools/list has
// "transcribe" and "create_timeline".
pub fn catalog() -> LinkerToolCatalog {
    LinkerToolCatalog::default()
}

pub const TRANSCRIBE: &str = "transcribe";

pub fn transcribe_tool() -> Value {
    json!({
        "name": TRANSCRIBE,
        "description": "Transcribe the speech in media files on this machine, with word timings. Pass absolute file paths (a clip's file_path from list_media). Transcripts are cached, so asking again for an unchanged file returns at once. Returns each transcript with [start-end] times in seconds from the start of the file, and the path of a JSON file with per-word times.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "files": { "type": "array", "items": { "type": "string" }, "description": "Absolute paths of video or audio files." },
                "language": {
                    "type": "string",
                    "description": "Spoken language as a Whisper code such as \"en\" or \"nl\". Leave out to detect it per file.",
                },
            },
            "required": ["files"],
        },
        "annotations": { "readOnlyHint": true },
    })
}

fn tool_name(tool: &Value) -> Option<&str> {
    tool.get("name").and_then(Value::as_str)
}

impl LinkerToolCatalog {
    /// What tools/list answers, in the TypeScript's order.
    pub fn tools(&self) -> Vec<Value> {
        let mut tools: Vec<Value> = self
            .resolve
            .iter()
            .map(|tool| {
                let mut listed = json!({
                    "name": tool.name,
                    "description": tool.description,
                    "inputSchema": tool.input_schema,
                    "annotations": { "readOnlyHint": tool.read_only, "destructiveHint": false },
                });
                // Claude loads this server's tools up front; these wait behind its tool search.
                if !tool.core {
                    listed["_meta"] = json!({ "anthropic/alwaysLoad": false });
                }
                listed
            })
            .collect();
        tools.push(transcribe_tool());
        tools.extend(self.library.iter().cloned());
        tools.extend(self.study.iter().cloned());
        tools.extend(self.brief.iter().cloned());
        tools
    }

    /// The API route for one of the other modules' tools.
    fn route(&self, name: &str) -> Option<&'static str> {
        let has = |tools: &[Value]| tools.iter().any(|tool| tool_name(tool) == Some(name));
        if self.resolve.iter().any(|tool| tool.name == name) {
            Some("/api/resolve-tool")
        } else if has(&self.library) {
            Some("/api/library-tool")
        } else if has(&self.study) {
            Some("/api/study-tool")
        } else if has(&self.brief) {
            Some("/api/brief-tool")
        } else {
            None
        }
    }
}

/// An image file the Linker-Edit server made on this computer, as MCP image content for the agent to see.
fn image_content(file: &str) -> Option<Value> {
    let path = Path::new(file);
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    let mime_type = match extension.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        _ => return None,
    };
    let data = std::fs::read(path).ok()?;
    Some(json!({
        "type": "image",
        "data": base64::engine::general_purpose::STANDARD.encode(data),
        "mimeType": mime_type,
    }))
}

struct Reply {
    text: String,
    is_error: bool,
    images: Vec<String>,
}

impl Reply {
    fn error(text: impl Into<String>) -> Self {
        Reply {
            text: text.into(),
            is_error: true,
            images: vec![],
        }
    }
}

async fn post(api: &Api, route: &str, body: Value) -> Reply {
    let unreachable = |error: &dyn std::fmt::Display| {
        Reply::error(format!("Linker-Edit isn't reachable: {error}"))
    };
    let (status, text) = match api.post(route, &body).await {
        Ok(response) => response,
        Err(error) => return unreachable(&error),
    };
    let parsed: Value = match serde_json::from_str(&text) {
        Ok(parsed) => parsed,
        Err(error) => return unreachable(&error),
    };
    let field = |key: &str| parsed.get(key).and_then(Value::as_str).map(str::to_string);
    if status != 200 {
        return Reply::error(field("error").unwrap_or(text));
    }
    Reply {
        text: field("text").unwrap_or_default(),
        is_error: parsed.get("isError") == Some(&Value::Bool(true)),
        images: parsed
            .get("images")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
    }
}

/// JavaScript's String(value), for a tool name that may be anything.
fn js_string(value: Option<&Value>) -> String {
    match value {
        None => "undefined".into(),
        Some(Value::Null) => "null".into(),
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| match item {
                Value::Null => String::new(),
                item => js_string(Some(item)),
            })
            .collect::<Vec<_>>()
            .join(","),
        Some(Value::Object(_)) => "[object Object]".into(),
        Some(value) => value.to_string(),
    }
}

async fn call_tool(
    catalog: &LinkerToolCatalog,
    api: &Api,
    name: Option<&Value>,
    args: Value,
) -> Reply {
    let name_text = name.and_then(Value::as_str);
    if name_text == Some(TRANSCRIBE) {
        let files: Vec<&str> = args
            .get("files")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        if files.is_empty() {
            return Reply::error("Pass at least one file path in files.");
        }
        let mut body = json!({ "files": files });
        if let Some(language) = args.get("language").and_then(Value::as_str) {
            body["language"] = json!(language);
        }
        return post(api, "/api/transcribe", body).await;
    }
    match name_text.and_then(|name| catalog.route(name)) {
        Some(route) => post(api, route, json!({ "tool": name, "input": args })).await,
        None => Reply::error(format!("Unknown tool {}.", js_string(name))),
    }
}

/// The answer to one message, or None for a notification.
async fn handle(
    catalog: &LinkerToolCatalog,
    api: &Api,
    version: &str,
    message: Value,
) -> Option<Value> {
    let id = message.get("id")?.clone();
    let params = message.get("params");
    let method = message.get("method");
    let answer = match method.and_then(Value::as_str) {
        Some("initialize") => json!({
            "result": {
                "protocolVersion": params
                    .and_then(|params| params.get("protocolVersion"))
                    .filter(|version| !version.is_null())
                    .cloned()
                    .unwrap_or_else(|| json!("2025-06-18")),
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "linker-edit", "version": version },
            },
        }),
        Some("ping") => json!({ "result": {} }),
        Some("tools/list") => json!({ "result": { "tools": catalog.tools() } }),
        Some("tools/call") => {
            let name = params.and_then(|params| params.get("name"));
            let args = params
                .and_then(|params| params.get("arguments"))
                .filter(|args| !args.is_null())
                .cloned()
                .unwrap_or_else(|| json!({}));
            let reply = call_tool(catalog, api, name, args).await;
            let mut content = vec![json!({ "type": "text", "text": reply.text })];
            content.extend(reply.images.iter().filter_map(|file| image_content(file)));
            json!({ "result": { "content": content, "isError": reply.is_error } })
        }
        _ => json!({
            "error": { "code": -32601, "message": format!("Method not found: {}", js_string(method)) },
        }),
    };
    let mut message = json!({ "id": id });
    if let (Some(message), Value::Object(mut answer)) = (message.as_object_mut(), answer) {
        message.append(&mut answer);
    }
    Some(message)
}

async fn send<W: AsyncWrite + Unpin>(output: &Mutex<W>, mut message: Value) {
    let mut line = json!({ "jsonrpc": "2.0" });
    if let (Some(line), Some(message)) = (line.as_object_mut(), message.as_object_mut()) {
        line.append(message);
    }
    let mut output = output.lock().await;
    let _ = output.write_all(format!("{line}\n").as_bytes()).await;
    let _ = output.flush().await;
}

/// Serves the linker tools on these streams until the input ends and every call has been answered.
pub async fn serve_linker_mcp<R, W>(
    catalog: Arc<LinkerToolCatalog>,
    api: Api,
    version: String,
    input: R,
    output: W,
) -> anyhow::Result<()>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let output = Arc::new(Mutex::new(output));
    let mut calls = JoinSet::new();
    let mut lines = input.lines();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let message = match serde_json::from_str::<Value>(&line) {
            Ok(Value::Object(message)) => Value::Object(message),
            // JSON.parse failing, or reading .id of null, answered the TypeScript's way.
            Ok(Value::Null) | Err(_) => {
                send(
                    &output,
                    json!({ "id": null, "error": { "code": -32700, "message": "Parse error" } }),
                )
                .await;
                continue;
            }
            Ok(_) => continue,
        };
        let (catalog, api, version, output) = (
            catalog.clone(),
            api.clone(),
            version.clone(),
            output.clone(),
        );
        // Calls run side by side; an approval can wait for the user while other tools answer.
        calls.spawn(async move {
            if let Some(answer) = handle(&catalog, &api, &version, message).await {
                send(&output, answer).await;
            }
        });
    }
    while calls.join_next().await.is_some() {}
    Ok(())
}

/// `linker-edit mcp`: the server on stdin and stdout, calling the API that LINKER_EDIT_API and
/// LINKER_EDIT_TOKEN name.
pub fn run_linker_mcp(catalog: LinkerToolCatalog) -> anyhow::Result<()> {
    let version = std::env::var("LINKER_EDIT_VERSION")
        .ok()
        .filter(|version| !version.is_empty())
        .unwrap_or_else(|| "0.0.0".into());
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(serve_linker_mcp(
            Arc::new(catalog),
            Api::from_env(),
            version,
            tokio::io::BufReader::new(tokio::io::stdin()),
            tokio::io::stdout(),
        ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;
    use tokio::io::{AsyncReadExt, BufReader};
    use tokio::net::TcpListener;

    type Seen = Arc<StdMutex<Vec<(String, Option<String>, Value)>>>;

    /// A stand-in Linker-Edit server that answers every POST with reply(route, body).
    async fn api(reply: impl Fn(&str, &Value) -> Value + Send + Sync + 'static) -> (Api, Seen) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen: Seen = Arc::default();
        let (reply, record) = (Arc::new(reply), seen.clone());
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let (reply, record) = (reply.clone(), record.clone());
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    let mut buffer = [0u8; 4096];
                    let (head, length) = loop {
                        let read = socket.read(&mut buffer).await.unwrap();
                        request.extend_from_slice(&buffer[..read]);
                        let text = String::from_utf8_lossy(&request).to_string();
                        if let Some(end) = text.find("\r\n\r\n") {
                            let head = text[..end].to_string();
                            let length = head
                                .lines()
                                .find_map(|line| {
                                    let (name, value) = line.split_once(':')?;
                                    name.eq_ignore_ascii_case("content-length")
                                        .then(|| value.trim().parse::<usize>().ok())?
                                })
                                .unwrap_or(0);
                            break (head, end + 4 + length);
                        }
                    };
                    while request.len() < length {
                        let read = socket.read(&mut buffer).await.unwrap();
                        request.extend_from_slice(&buffer[..read]);
                    }
                    let body: Value =
                        serde_json::from_slice(&request[head.len() + 4..length]).unwrap();
                    let route = head.split(' ').nth(1).unwrap().to_string();
                    let auth = head.lines().find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("authorization")
                            .then(|| value.trim().to_string())
                    });
                    let answer = reply(&route, &body).to_string();
                    record.lock().unwrap().push((route, auth, body));
                    let response = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{answer}",
                        answer.len()
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                });
            }
        });
        (Api::new(format!("http://127.0.0.1:{port}"), "t0ken"), seen)
    }

    fn tool(name: &str, read_only: bool, core: bool) -> LinkerResolveTool {
        LinkerResolveTool {
            name: name.into(),
            description: format!("{name} does it."),
            input_schema: json!({ "type": "object" }),
            read_only,
            core,
        }
    }

    fn named(name: &str, read_only: bool) -> Value {
        json!({ "name": name, "description": "", "inputSchema": { "type": "object" }, "annotations": { "readOnlyHint": read_only } })
    }

    fn test_catalog() -> LinkerToolCatalog {
        LinkerToolCatalog {
            resolve: vec![
                tool("view_frames", true, true),
                tool("create_timeline", false, true),
                tool("add_clips", false, true),
                tool("render_timeline", false, true),
                tool("set_color", false, false),
            ],
            library: vec![
                named("library_search", true),
                named("library_import", false),
            ],
            study: vec![named("study_media", true), named("compare_cut", true)],
            brief: vec![
                named("save_brief", false),
                named("add_lesson", false),
                named("save_example", false),
            ],
        }
    }

    /// The linker MCP server in front of `api`; request(method, params) answers with the result.
    struct Linker {
        input: tokio::io::DuplexStream,
        output: tokio::io::Lines<BufReader<tokio::io::DuplexStream>>,
        id: u64,
    }

    impl Linker {
        fn start(api: Api) -> Self {
            let (input, server_input) = tokio::io::duplex(1 << 16);
            let (server_output, output) = tokio::io::duplex(1 << 20);
            tokio::spawn(serve_linker_mcp(
                Arc::new(test_catalog()),
                api,
                "1.2.3".into(),
                BufReader::new(server_input),
                server_output,
            ));
            Linker {
                input,
                output: BufReader::new(output).lines(),
                id: 0,
            }
        }

        async fn request(&mut self, method: &str, params: Value) -> Value {
            self.id += 1;
            let line =
                json!({ "jsonrpc": "2.0", "id": self.id, "method": method, "params": params });
            self.input
                .write_all(format!("{line}\n").as_bytes())
                .await
                .unwrap();
            let answer: Value =
                serde_json::from_str(&self.output.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(answer["id"], json!(self.id));
            answer
        }
    }

    #[tokio::test]
    async fn lists_every_resolve_tool_with_its_read_only_hint_transcribe_and_the_librarys_tools() {
        let (api, _) = api(|_, _| json!({})).await;
        let mut linker = Linker::start(api);
        let answer = linker.request("tools/list", json!({})).await;
        let tools = answer["result"]["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().filter_map(tool_name).collect();
        assert_eq!(
            names,
            [
                "view_frames",
                "create_timeline",
                "add_clips",
                "render_timeline",
                "set_color",
                "transcribe",
                "library_search",
                "library_import",
                "study_media",
                "compare_cut",
                "save_brief",
                "add_lesson",
                "save_example"
            ]
        );
        let hint = |name: &str| {
            tools
                .iter()
                .find(|tool| tool_name(tool) == Some(name))
                .unwrap()["annotations"]["readOnlyHint"]
                .clone()
        };
        assert_eq!(hint("library_search"), json!(true));
        assert_eq!(hint("library_import"), json!(false));
        assert_eq!(hint("view_frames"), json!(true));
        assert_eq!(hint("render_timeline"), json!(false));
        assert_eq!(hint("transcribe"), json!(true));
        // Rarely used tools wait behind Claude's tool search.
        let deferred: Vec<&str> = tools
            .iter()
            .filter(|tool| tool["_meta"]["anthropic/alwaysLoad"] == json!(false))
            .filter_map(tool_name)
            .collect();
        assert_eq!(deferred, ["set_color"]);

        let init = linker
            .request("initialize", json!({ "protocolVersion": "2025-11-25" }))
            .await;
        assert_eq!(
            init["result"],
            json!({ "protocolVersion": "2025-11-25", "capabilities": { "tools": {} }, "serverInfo": { "name": "linker-edit", "version": "1.2.3" } })
        );
        assert_eq!(
            linker.request("nope", json!({})).await["error"],
            json!({ "code": -32601, "message": "Method not found: nope" })
        );
    }

    #[tokio::test]
    async fn passes_resolve_tools_to_the_linker_edit_server_and_sends_the_images_it_names() {
        let dir = tempfile::tempdir().unwrap();
        let image = dir.path().join("frame.jpg");
        std::fs::write(&image, [0xff, 0xd8, 0xff, 0xd9]).unwrap();
        let image = image.display().to_string();
        let gone = format!("{image}.gone");
        let (api, seen) = api(move |_, body| {
            if body["tool"] == "view_frames" {
                json!({ "text": "{\"frames\":1}", "isError": false, "images": [image, gone] })
            } else {
                json!({ "text": "No.", "isError": true })
            }
        })
        .await;
        let mut linker = Linker::start(api);

        let frames = linker
            .request(
                "tools/call",
                json!({ "name": "view_frames", "arguments": { "frames": [1] } }),
            )
            .await;
        assert_eq!(
            frames["result"],
            json!({
                "content": [
                    { "type": "text", "text": "{\"frames\":1}" },
                    { "type": "image", "data": "/9j/2Q==", "mimeType": "image/jpeg" },
                ],
                "isError": false,
            })
        );
        let refused = linker
            .request(
                "tools/call",
                json!({ "name": "create_timeline", "arguments": { "name": "Cut" } }),
            )
            .await;
        assert_eq!(
            refused["result"],
            json!({ "content": [{ "type": "text", "text": "No." }], "isError": true })
        );
        linker
            .request(
                "tools/call",
                json!({ "name": "library_search", "arguments": { "kind": "music", "query": "upbeat" } }),
            )
            .await;
        let bearer = Some("Bearer t0ken".to_string());
        assert_eq!(
            *seen.lock().unwrap(),
            vec![
                (
                    "/api/resolve-tool".to_string(),
                    bearer.clone(),
                    json!({ "tool": "view_frames", "input": { "frames": [1] } })
                ),
                (
                    "/api/resolve-tool".to_string(),
                    bearer.clone(),
                    json!({ "tool": "create_timeline", "input": { "name": "Cut" } })
                ),
                (
                    "/api/library-tool".to_string(),
                    bearer,
                    json!({ "tool": "library_search", "input": { "kind": "music", "query": "upbeat" } })
                ),
            ]
        );

        let unknown = linker
            .request("tools/call", json!({ "name": "rm_rf" }))
            .await;
        assert_eq!(
            unknown["result"],
            json!({ "content": [{ "type": "text", "text": "Unknown tool rm_rf." }], "isError": true })
        );
        let no_files = linker
            .request(
                "tools/call",
                json!({ "name": "transcribe", "arguments": { "files": [] } }),
            )
            .await;
        assert_eq!(
            no_files["result"]["content"][0]["text"],
            json!("Pass at least one file path in files.")
        );
    }

    #[tokio::test]
    async fn says_when_the_linker_edit_server_isnt_reachable() {
        // A port nothing listens on: bound, then closed.
        let port = TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let mut linker = Linker::start(Api::new(format!("http://127.0.0.1:{port}"), "t0ken"));
        let answer = linker
            .request(
                "tools/call",
                json!({ "name": "transcribe", "arguments": { "files": ["C:/a.wav"] } }),
            )
            .await;
        assert_eq!(answer["result"]["isError"], json!(true));
        assert!(
            answer["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .starts_with("Linker-Edit isn't reachable: ")
        );
    }
}
