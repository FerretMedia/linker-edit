// Linker-Edit's one connection to Resolve: Resolve's own MCP server (ResolveMCP), started as a child
// process and spoken to over stdio. Everything Linker-Edit does in Resolve is a Python script from
// crates/server/src/resolve/scripts run through its run_script tool: the status check, history saves, previews,
// opening projects and the agents' Resolve tools. Calls go one at a time, in order.
//
// MCP is spoken by hand here, as in the TypeScript: newline-delimited JSON-RPC with three methods (initialize,
// tools/list, tools/call). An MCP client library would hide what this needs to own: killing and restarting the
// process when a call hangs, the exit code and last stderr line in the error, and one call at a time.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use regex::Regex;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::oneshot;

use super::locate::MISSING_RESOLVE_MCP;
use crate::assets;

/// Resolve's MCP server stops a script after this many seconds, its maximum.
pub const MAX_SCRIPT_SECONDS: u64 = 60;

/// How long a plain tool call may take when the caller has no better idea (the TypeScript's default).
pub const DEFAULT_TOOL_TIMEOUT: Duration = Duration::from_secs(30);

/// refused: the script said no, with a message for the user or agent. timeout: the script ran out of time.
/// error: the script crashed. unavailable: there is no answer from Resolve's MCP server at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveErrorKind {
    Refused,
    Timeout,
    Error,
    Unavailable,
}

/// An error from Resolve. It travels inside anyhow::Error; callers that branch on the kind use [`error_kind`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ResolveError {
    pub kind: ResolveErrorKind,
    pub message: String,
}

impl ResolveError {
    pub fn new(kind: ResolveErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

/// The kind of a ResolveError, or None for any other error (the TypeScript's `instanceof ResolveError`).
pub fn error_kind(error: &anyhow::Error) -> Option<ResolveErrorKind> {
    error.downcast_ref::<ResolveError>().map(|error| error.kind)
}

/// The connection to Resolve's MCP server. A trait so tests can stand in for Resolve.
#[async_trait]
pub trait ResolveConnection: Send + Sync {
    /// The ResolveMCP program, or None when this computer has none.
    fn command(&self) -> Option<&str>;
    /// Calls one of ResolveMCP's tools and returns its text.
    async fn call_tool(&self, name: &str, args: Value, timeout: Duration)
    -> anyhow::Result<String>;
    /// Runs scripts/<name>.py with `ARGS` set to args, and returns its `result` as JSON.
    /// [`ResolveConnection::run_script`] reads it into a type.
    async fn run_script_value(
        &self,
        name: &str,
        args: Value,
        timeout_seconds: u64,
    ) -> anyhow::Result<Value>;
    fn close(&self);
}

impl<'a> dyn ResolveConnection + 'a {
    /// Runs scripts/<name>.py with `ARGS` set to args, and reads its `result` as T.
    pub async fn run_script<T: DeserializeOwned>(
        &self,
        name: &str,
        args: Value,
        timeout_seconds: u64,
    ) -> anyhow::Result<T> {
        let value = self.run_script_value(name, args, timeout_seconds).await?;
        serde_json::from_value(value).map_err(|error| {
            anyhow::anyhow!("Resolve's {name} script returned something unexpected: {error}")
        })
    }
}

static SCRIPT_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new("^[a-z_]+$").unwrap());

fn script_file(name: &str) -> anyhow::Result<&'static str> {
    if !SCRIPT_NAME.is_match(name) {
        anyhow::bail!("Bad script name {name}.");
    }
    assets::resolve_script(&format!("{name}.py"))
        .ok_or_else(|| anyhow::anyhow!("There is no Resolve script {name}."))
}

/// The script ResolveMCP runs: ARGS, the shared helpers, and the script itself.
pub fn script_source(name: &str, args: &Value) -> anyhow::Result<String> {
    let mut parts = vec!["common"];
    if name == "list_projects" || name == "open_project" {
        parts.push("projects_common");
    }
    parts.push(name);
    let args = if args.is_null() { &json!({}) } else { args };
    // A JSON string literal is a valid Python string literal too.
    let literal = serde_json::to_string(&serde_json::to_string(args)?)?;
    let mut out = vec![format!("import json\nARGS = json.loads({literal})")];
    for part in parts {
        out.push(script_file(part)?.to_string());
    }
    Ok(out.join("\n\n"))
}

static TIMED_OUT: LazyLock<Regex> = LazyLock::new(|| Regex::new("(?i)^Script timed out").unwrap());
static REFUSED: LazyLock<Regex> = LazyLock::new(|| Regex::new("^Refused: (.*)$").unwrap());

/// run_script's text: {"result": …} on success, {"error": traceback} on failure, plus any printed "output".
pub fn parse_script_result(text: &str) -> Result<Value, ResolveError> {
    let Ok(body) = serde_json::from_str::<Value>(text) else {
        return Err(script_error(text));
    };
    let Value::Object(body) = body else {
        return Ok(Value::Null);
    };
    if let Some(error) = body.get("error") {
        let text = match error {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        };
        return Err(script_error(&text));
    }
    Ok(body.get("result").cloned().unwrap_or(Value::Null))
}

fn script_error(text: &str) -> ResolveError {
    let trimmed = text.trim();
    if TIMED_OUT.is_match(trimmed) {
        return ResolveError::new(ResolveErrorKind::Timeout, trimmed);
    }
    // A traceback's last line names the exception; our own refusals carry a message for people.
    let last = trimmed
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .rfind(|line| !line.trim().is_empty())
        .unwrap_or(trimmed);
    if let Some(refused) = REFUSED.captures(last) {
        return ResolveError::new(ResolveErrorKind::Refused, &refused[1]);
    }
    let message = if last.is_empty() {
        "Resolve's script failed without saying why."
    } else {
        last
    };
    ResolveError::new(ResolveErrorKind::Error, message)
}

/// The run_script call for a script, its timeout capped at Resolve's 60 seconds; shared by every connection.
pub fn run_script_call(
    name: &str,
    args: &Value,
    timeout_seconds: u64,
) -> anyhow::Result<(Value, Duration)> {
    let seconds = timeout_seconds.clamp(1, MAX_SCRIPT_SECONDS);
    let args = json!({ "script": script_source(name, args)?, "timeout": seconds });
    Ok((args, Duration::from_secs(seconds + 20)))
}

#[derive(Debug, Default, Deserialize)]
struct Response {
    #[serde(default)]
    result: Option<ResponseResult>,
    #[serde(default)]
    error: Option<ResponseError>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResponseResult {
    #[serde(default)]
    content: Option<Vec<Content>>,
    #[serde(default)]
    is_error: Option<bool>,
    #[serde(default)]
    tools: Option<Vec<ToolEntry>>,
}

#[derive(Debug, Deserialize)]
struct Content {
    r#type: String,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ToolEntry {
    name: String,
}

#[derive(Debug, Deserialize)]
struct ResponseError {
    #[serde(default)]
    message: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ConnectionOptions {
    pub command: Option<PathBuf>,
    /// Extra arguments, for tests that run a stand-in server through Python.
    pub args: Vec<String>,
    /// Extra environment for the server, for tests.
    pub env: Vec<(String, String)>,
}

type Waiter = oneshot::Sender<Result<Response, ResolveError>>;

struct Process {
    generation: u64,
    stdin: Arc<tokio::sync::Mutex<ChildStdin>>,
    /// Dropping or sending stops the process.
    kill: Option<oneshot::Sender<()>>,
    ready: bool,
}

#[derive(Default)]
struct State {
    process: Option<Process>,
    pending: HashMap<u64, Waiter>,
    generation: u64,
}

struct Inner {
    command: Option<String>,
    options: ConnectionOptions,
    state: Mutex<State>,
    next_id: AtomicU64,
}

/// The connection to Resolve's own MCP server: [`create_resolve_connection`].
pub struct ResolveMcpConnection {
    inner: Arc<Inner>,
    // One call at a time: Resolve does one thing at a time, and a stuck call is killed with its process.
    queue: tokio::sync::Mutex<()>,
}

/// Starts nothing yet: the server starts on the first call, and again after it stopped or hung.
pub fn create_resolve_connection(options: ConnectionOptions) -> ResolveMcpConnection {
    ResolveMcpConnection {
        inner: Arc::new(Inner {
            command: options
                .command
                .as_ref()
                .map(|command| command.to_string_lossy().into_owned()),
            options,
            state: Mutex::new(State::default()),
            next_id: AtomicU64::new(0),
        }),
        queue: tokio::sync::Mutex::new(()),
    }
}

fn unavailable(message: impl Into<String>) -> anyhow::Error {
    ResolveError::new(ResolveErrorKind::Unavailable, message).into()
}

impl Inner {
    fn reset(&self, reason: &str) {
        let (dead, pending) = {
            let mut state = self.state.lock().unwrap();
            (state.process.take(), std::mem::take(&mut state.pending))
        };
        if let Some(mut dead) = dead
            && let Some(kill) = dead.kill.take()
        {
            let _ = kill.send(());
        }
        for (_, waiter) in pending {
            let _ = waiter.send(Err(ResolveError::new(
                ResolveErrorKind::Unavailable,
                reason,
            )));
        }
    }

    async fn request(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> anyhow::Result<Response> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        let (sender, receiver) = oneshot::channel();
        let stdin = {
            let mut state = self.state.lock().unwrap();
            let Some(process) = &state.process else {
                return Err(unavailable("Resolve's MCP server isn't running."));
            };
            let stdin = process.stdin.clone();
            state.pending.insert(id, sender);
            stdin
        };
        let line = format!(
            "{}\n",
            json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
        );
        // A write to a server that stopped fails; its exit answers the call.
        let _ = stdin.lock().await.write_all(line.as_bytes()).await;
        match tokio::time::timeout(timeout, receiver).await {
            Ok(Ok(Ok(response))) => Ok(response),
            Ok(Ok(Err(error))) => Err(error.into()),
            Ok(Err(_)) => Err(unavailable("Resolve's MCP server isn't running.")),
            Err(_) => {
                self.state.lock().unwrap().pending.remove(&id);
                // The call may still be running in Resolve; a fresh server process is the only clean way on.
                self.reset("Resolve didn't answer in time.");
                Err(unavailable(
                    "Resolve didn't answer in time. Is a dialog open in Resolve?",
                ))
            }
        }
    }

    async fn notify(&self, method: &str) {
        let stdin = self
            .state
            .lock()
            .unwrap()
            .process
            .as_ref()
            .map(|process| process.stdin.clone());
        if let Some(stdin) = stdin {
            let line = format!("{}\n", json!({ "jsonrpc": "2.0", "method": method }));
            let _ = stdin.lock().await.write_all(line.as_bytes()).await;
        }
    }

    fn spawn(self: &Arc<Self>, command: &str) -> anyhow::Result<()> {
        let mut builder = Command::new(command);
        builder
            .args(&self.options.args)
            .envs(self.options.env.iter().map(|(key, value)| (key, value)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        builder.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let mut child = builder.spawn().map_err(|error| {
            unavailable(format!("Couldn't start Resolve's MCP server: {error}"))
        })?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let (kill, killed) = oneshot::channel();
        let generation = {
            let mut state = self.state.lock().unwrap();
            state.generation += 1;
            state.process = Some(Process {
                generation: state.generation,
                stdin: Arc::new(tokio::sync::Mutex::new(stdin)),
                kill: Some(kill),
                ready: false,
            });
            state.generation
        };

        let stderr_tail = Arc::new(Mutex::new(String::new()));
        let tail = stderr_tail.clone();
        let stderr_task = tokio::spawn(async move {
            let mut stderr = stderr;
            let mut buffer = [0u8; 4096];
            while let Ok(read) = stderr.read(&mut buffer).await {
                if read == 0 {
                    break;
                }
                let mut tail = tail.lock().unwrap();
                tail.push_str(&String::from_utf8_lossy(&buffer[..read]));
                let excess = tail.chars().count().saturating_sub(2000);
                if excess > 0 {
                    let cut = tail
                        .char_indices()
                        .nth(excess)
                        .map_or(tail.len(), |(at, _)| at);
                    tail.drain(..cut);
                }
            }
        });

        let inner = self.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(message) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                let Some(id) = message.get("id").and_then(Value::as_u64) else {
                    continue;
                };
                let waiter = inner.state.lock().unwrap().pending.remove(&id);
                if let Some(waiter) = waiter {
                    let response = serde_json::from_value(message).unwrap_or_default();
                    let _ = waiter.send(Ok(response));
                }
            }
        });

        let inner = self.clone();
        tokio::spawn(async move {
            let status = wait_or_kill(&mut child, killed).await;
            let Some(status) = status else { return };
            // What it said last, once its stderr is read to the end.
            let _ = tokio::time::timeout(Duration::from_millis(500), stderr_task).await;
            let tail = stderr_tail.lock().unwrap().clone();
            let detail = tail
                .trim()
                .split('\n')
                .next_back()
                .unwrap_or("")
                .to_string();
            let code = status
                .code()
                .map_or("null".to_string(), |code| code.to_string());
            let current = inner
                .state
                .lock()
                .unwrap()
                .process
                .as_ref()
                .map(|process| process.generation);
            if current == Some(generation) {
                let ending = if detail.is_empty() {
                    ".".to_string()
                } else {
                    format!(": {detail}")
                };
                inner.reset(&format!(
                    "Resolve's MCP server stopped (exit {code}){ending}"
                ));
            }
        });
        Ok(())
    }

    async fn start(self: &Arc<Self>) -> anyhow::Result<()> {
        let Some(command) = self.command.clone() else {
            return Err(unavailable(MISSING_RESOLVE_MCP));
        };
        if self
            .state
            .lock()
            .unwrap()
            .process
            .as_ref()
            .is_some_and(|process| process.ready)
        {
            return Ok(());
        }
        let started = async {
            self.spawn(&command)?;
            let client = json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "linker-edit", "version": "1" } });
            self.request("initialize", client, Duration::from_secs(30)).await?;
            self.notify("notifications/initialized").await;
            let listed = self.request("tools/list", json!({}), Duration::from_secs(30)).await?;
            let tools = listed.result.and_then(|result| result.tools).unwrap_or_default();
            if !tools.iter().any(|tool| tool.name == "run_script") {
                return Err(unavailable(
                    "This Resolve's MCP server has no run_script tool. Update DaVinci Resolve Studio.",
                ));
            }
            if let Some(process) = self.state.lock().unwrap().process.as_mut() {
                process.ready = true;
            }
            Ok(())
        }
        .await;
        // A failed start is retried on the next call rather than remembered.
        if started.is_err() {
            self.reset("Resolve's MCP server didn't start.");
        }
        started
    }

    async fn call_now(
        self: &Arc<Self>,
        name: &str,
        args: Value,
        timeout: Duration,
    ) -> anyhow::Result<String> {
        self.start().await?;
        let response = self
            .request(
                "tools/call",
                json!({ "name": name, "arguments": args }),
                timeout,
            )
            .await?;
        if let Some(error) = response.error {
            let message = error
                .message
                .unwrap_or_else(|| format!("Resolve's {name} failed."));
            return Err(ResolveError::new(ResolveErrorKind::Error, message).into());
        }
        let result = response.result.unwrap_or_default();
        let text = result
            .content
            .unwrap_or_default()
            .into_iter()
            .map(|part| {
                if part.r#type == "text" {
                    part.text.unwrap_or_default()
                } else {
                    format!("[{}]", part.r#type)
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        if result.is_error == Some(true) {
            let message = if text.is_empty() {
                format!("Resolve's {name} failed.")
            } else {
                text
            };
            return Err(ResolveError::new(ResolveErrorKind::Error, message).into());
        }
        Ok(text)
    }
}

/// Waits for the child to exit and returns its status, or kills it when asked (or when the sender is dropped)
/// and returns None.
async fn wait_or_kill(
    child: &mut Child,
    killed: oneshot::Receiver<()>,
) -> Option<std::process::ExitStatus> {
    tokio::select! {
        status = child.wait() => status.ok(),
        _ = killed => {
            let _ = child.kill().await;
            None
        }
    }
}

#[async_trait]
impl ResolveConnection for ResolveMcpConnection {
    fn command(&self) -> Option<&str> {
        self.inner.command.as_deref()
    }

    async fn call_tool(
        &self,
        name: &str,
        args: Value,
        timeout: Duration,
    ) -> anyhow::Result<String> {
        let _turn = self.queue.lock().await;
        self.inner.call_now(name, args, timeout).await
    }

    async fn run_script_value(
        &self,
        name: &str,
        args: Value,
        timeout_seconds: u64,
    ) -> anyhow::Result<Value> {
        let (args, timeout) = run_script_call(name, &args, timeout_seconds)?;
        let text = self.call_tool("run_script", args, timeout).await?;
        Ok(parse_script_result(&text)?)
    }

    fn close(&self) {
        self.inner.reset("Linker-Edit closed the connection.");
    }
}

impl Drop for ResolveMcpConnection {
    fn drop(&mut self) {
        self.inner.reset("Linker-Edit closed the connection.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolve::testing::{PYTHON, has_python};

    const FAKE: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/resolve/fixtures/fake_resolve_mcp.py"
    );

    fn connect(env: &[(&str, &str)]) -> ResolveMcpConnection {
        create_resolve_connection(ConnectionOptions {
            command: Some(PYTHON.into()),
            args: vec!["-u".into(), FAKE.into()],
            env: env
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect(),
        })
    }

    #[derive(Debug, Deserialize)]
    struct Status {
        calls: u64,
        pid: u64,
    }

    async fn status(connection: &ResolveMcpConnection) -> Status {
        let text = connection
            .call_tool("get_resolve_status", json!({}), DEFAULT_TOOL_TIMEOUT)
            .await
            .unwrap();
        serde_json::from_str(&text).unwrap()
    }

    fn failure<T: std::fmt::Debug>(
        result: anyhow::Result<T>,
    ) -> (Option<ResolveErrorKind>, String) {
        let error = result.unwrap_err();
        (error_kind(&error), error.to_string())
    }

    #[test]
    fn script_source_passes_the_input_as_json_python_reads_ahead_of_the_helpers_and_the_script() {
        let source = script_source("status", &json!({ "name": "AI - \"Cut\"\n", "n": 2 })).unwrap();
        assert!(source.starts_with(
            r#"import json
ARGS = json.loads("{\"name\":\"AI - \\\"Cut\\\"\\n\",\"n\":2}")"#
        ));
        assert!(source.contains("class Refused(Exception)"));
        assert!(source.trim_end().ends_with("result = out"));
        assert!(
            script_source("../secrets", &json!({}))
                .unwrap_err()
                .to_string()
                .contains("Bad script name")
        );
    }

    #[test]
    fn parse_script_result_reads_results() {
        for (text, expected) in [
            (r#"{"result": {"a": 1}}"#, json!({ "a": 1 })),
            (r#"{"output": "hi\n", "result": 2}"#, json!(2)),
            (r#"{"output": "hi\n"}"#, Value::Null),
        ] {
            assert_eq!(parse_script_result(text).unwrap(), expected, "{text}");
        }
    }

    #[test]
    fn parse_script_result_turns_failures_into_errors_of_their_kind() {
        use ResolveErrorKind::*;
        for (text, kind, message) in [
            (
                r#"{"error": "Traceback (most recent call last):\n  File \"<script>\", line 3\nRefused: \"Edit\" isn't an AI timeline."}"#,
                Refused,
                r#""Edit" isn't an AI timeline."#,
            ),
            (
                r#"{"error": "Traceback (most recent call last):\r\n  ...\r\nAttributeError: 'NoneType' object has no attribute 'GetName'\r\n"}"#,
                Error,
                "AttributeError: 'NoneType' object has no attribute 'GetName'",
            ),
            (
                r#"{"error": "Script timed out after 60s. Avoid long iterations; fetch only what you need."}"#,
                Timeout,
                "Script timed out after 60s. Avoid long iterations; fetch only what you need.",
            ),
            (
                "Error: Resolve is not running",
                Error,
                "Error: Resolve is not running",
            ),
        ] {
            assert_eq!(
                parse_script_result(text).unwrap_err(),
                ResolveError::new(kind, message),
                "{text}"
            );
        }
    }

    #[tokio::test]
    async fn starts_resolves_mcp_server_once_and_runs_scripts_through_run_script_capped_at_60_seconds()
     {
        if !has_python() {
            return;
        }
        let connection = connect(&[]);
        let length = script_source("status", &json!({})).unwrap().chars().count();
        assert_eq!(
            connection
                .run_script_value("status", json!({}), 600)
                .await
                .unwrap(),
            json!({ "length": length, "timeout": 60 })
        );
        let first = status(&connection).await;
        let second = status(&connection).await;
        assert_eq!([first.calls, second.calls, second.pid], [1, 2, first.pid]);
    }

    #[tokio::test]
    async fn runs_calls_one_at_a_time_in_order() {
        if !has_python() {
            return;
        }
        let connection = connect(&[]);
        let (a, b, c) = tokio::join!(
            status(&connection),
            status(&connection),
            status(&connection)
        );
        assert_eq!([a.calls, b.calls, c.calls], [1, 2, 3]);
    }

    #[tokio::test]
    async fn gives_up_on_a_call_that_never_answers_and_starts_afresh_for_the_next() {
        if !has_python() {
            return;
        }
        let connection = connect(&[]);
        let before = status(&connection).await;
        let (kind, message) = failure(
            connection
                .call_tool(
                    "run_script",
                    json!({ "script": "HANG" }),
                    Duration::from_millis(300),
                )
                .await,
        );
        assert_eq!(kind, Some(ResolveErrorKind::Unavailable));
        assert!(message.contains("didn't answer in time"), "{message}");
        assert_ne!(status(&connection).await.pid, before.pid);
    }

    #[tokio::test]
    async fn starts_afresh_after_the_server_stops() {
        if !has_python() {
            return;
        }
        let connection = connect(&[]);
        let before = status(&connection).await;
        let (kind, message) = failure(
            connection
                .call_tool(
                    "run_script",
                    json!({ "script": "DIE" }),
                    DEFAULT_TOOL_TIMEOUT,
                )
                .await,
        );
        assert_eq!(kind, Some(ResolveErrorKind::Unavailable));
        assert!(message.contains("stopped (exit 3)"), "{message}");
        assert_ne!(status(&connection).await.pid, before.pid);
    }

    #[tokio::test]
    async fn says_when_resolve_has_no_mcp_server_or_one_without_run_script() {
        let missing = create_resolve_connection(ConnectionOptions::default());
        let (kind, message) = failure(missing.run_script_value("status", json!({}), 30).await);
        assert_eq!(kind, Some(ResolveErrorKind::Unavailable));
        assert!(message.contains("21.1.2"), "{message}");
        if !has_python() {
            return;
        }
        let old = connect(&[("FAKE_NO_RUN_SCRIPT", "1")]);
        let (kind, message) = failure(old.run_script_value("status", json!({}), 30).await);
        assert_eq!(kind, Some(ResolveErrorKind::Unavailable));
        assert!(message.contains("no run_script"), "{message}");
    }

    #[tokio::test]
    async fn says_why_the_server_couldnt_start() {
        let broken = create_resolve_connection(ConnectionOptions {
            command: Some("linker-edit-no-such-program".into()),
            ..Default::default()
        });
        let (kind, message) = failure(
            broken
                .call_tool("get_resolve_status", json!({}), DEFAULT_TOOL_TIMEOUT)
                .await,
        );
        assert_eq!(kind, Some(ResolveErrorKind::Unavailable));
        assert!(
            message.starts_with("Couldn't start Resolve's MCP server: "),
            "{message}"
        );
    }
}
