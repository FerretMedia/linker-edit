// Our own stand-in for @anthropic-ai/claude-agent-sdk's query(): spawns Claude Code and speaks its stream-json
// protocol over stdio, as the SDK 0.3.316 (sdk.mjs: ProcessTransport and Query) does for the options claude.rs
// uses. Approvals (can_use_tool) and the guardrail hook (hook_callback) come back as control requests on
// stdout; the answers go to stdin. Everything else on stdout is an SDK message, passed on unchanged.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context as _, anyhow, bail};
use serde_json::{Map, Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::cli::{self, AGENT_SDK_VERSION};
use super::types::BoxFuture;

/// The answer to a can_use_tool request, the SDK's PermissionResult.
#[derive(Debug, Clone, PartialEq)]
pub enum PermissionResult {
    Allow { updated_input: Value },
    Deny { message: String },
}

impl PermissionResult {
    fn to_json(&self) -> Value {
        match self {
            PermissionResult::Allow { updated_input } => {
                json!({ "behavior": "allow", "updatedInput": updated_input })
            }
            PermissionResult::Deny { message } => json!({ "behavior": "deny", "message": message }),
        }
    }
}

/// canUseTool(toolName, input).
pub type CanUseTool =
    Arc<dyn Fn(String, Value) -> BoxFuture<'static, PermissionResult> + Send + Sync>;
/// A hook: (input, tool_use_id) to the hook's output, such as `{}` or `{ hookSpecificOutput }`.
pub type HookCallback =
    Arc<dyn Fn(Value, Option<String>) -> BoxFuture<'static, Value> + Send + Sync>;

#[derive(Clone)]
pub struct HookMatcher {
    pub matcher: Option<String>,
    pub hooks: Vec<HookCallback>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionMode {
    Default,
    BypassPermissions,
}

impl PermissionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            PermissionMode::Default => "default",
            PermissionMode::BypassPermissions => "bypassPermissions",
        }
    }
}

/// The SDK's Options, as far as Linker-Edit uses them.
#[derive(Clone, Default)]
pub struct QueryOptions {
    pub cwd: Option<PathBuf>,
    /// A session id to continue.
    pub resume: Option<String>,
    /// The SDK's abortController.
    pub abort: CancellationToken,
    /// systemPrompt `{ type: "preset", preset: "claude_code", append }`: Claude Code's own prompt, then this.
    pub append_system_prompt: Option<String>,
    /// mcpServers, each server's config as the SDK writes it into --mcp-config.
    pub mcp_servers: Map<String, Value>,
    pub disallowed_tools: Vec<String>,
    /// settings, an object passed as --settings.
    pub settings: Option<Value>,
    /// hooks by event name ("PreToolUse"), in order: callback ids are numbered in this order, like the SDK.
    pub hooks: Vec<(String, Vec<HookMatcher>)>,
    pub permission_mode: Option<PermissionMode>,
    pub allow_dangerously_skip_permissions: bool,
    pub can_use_tool: Option<CanUseTool>,
    pub model: Option<String>,
    pub setting_sources: Option<Vec<String>>,
    pub strict_mcp_config: bool,
}

/// The SDK messages of one query, ending with an error when the run failed or was aborted.
pub type Messages = mpsc::Receiver<anyhow::Result<Value>>;

/// The slice of the Agent SDK's query() a session uses; replays and tests stand in for it.
pub trait RunQuery: Send + Sync {
    fn query(&self, prompt: String, options: QueryOptions) -> Messages;
}

impl<F> RunQuery for F
where
    F: Fn(String, QueryOptions) -> Messages + Send + Sync,
{
    fn query(&self, prompt: String, options: QueryOptions) -> Messages {
        self(prompt, options)
    }
}

/// A Messages that yields these and ends, for tests and stand-ins.
pub fn messages(items: Vec<Value>) -> Messages {
    let (tx, rx) = mpsc::channel(items.len().max(1));
    for item in items {
        let _ = tx.try_send(Ok(item));
    }
    rx
}

/// The SDK's flag forms: `--name=value`, and `--name value` for a value that doesn't start with "-".
fn flag(name: &str, value: impl std::fmt::Display) -> String {
    format!("{name}={value}")
}

fn flag_pair(args: &mut Vec<String>, name: &str, value: String) {
    if value.len() > 1 && value.starts_with('-') {
        args.push(flag(name, value));
    } else {
        args.push(name.to_string());
        args.push(value);
    }
}

/// Claude Code's arguments, in the order ProcessTransport builds them.
pub fn claude_args(options: &QueryOptions) -> Vec<String> {
    let mut args: Vec<String> = [
        "--output-format",
        "stream-json",
        "--verbose",
        "--input-format",
        "stream-json",
    ]
    .map(String::from)
    .to_vec();
    if let Some(model) = &options.model {
        args.push(flag("--model", model));
    }
    if options.can_use_tool.is_some() {
        args.push("--permission-prompt-tool=stdio".into());
    }
    if let Some(resume) = &options.resume {
        args.push(flag("--resume", resume));
    }
    if !options.disallowed_tools.is_empty() {
        args.push(flag(
            "--disallowedTools",
            options.disallowed_tools.join(","),
        ));
    }
    if !options.mcp_servers.is_empty() {
        let config = json!({ "mcpServers": options.mcp_servers });
        flag_pair(&mut args, "--mcp-config", config.to_string());
    }
    if let Some(sources) = &options.setting_sources {
        args.push(flag("--setting-sources", sources.join(",")));
    }
    if options.strict_mcp_config {
        args.push("--strict-mcp-config".into());
    }
    if let Some(mode) = options.permission_mode {
        args.push(flag("--permission-mode", mode.as_str()));
    }
    if options.allow_dangerously_skip_permissions {
        args.push("--allow-dangerously-skip-permissions".into());
    }
    // extraArgs, where the SDK puts settings, come last.
    if let Some(settings) = &options.settings {
        flag_pair(&mut args, "--settings", settings.to_string());
    }
    args
}

fn truthy(value: Option<&str>) -> bool {
    matches!(
        value
            .map(|value| value.trim().to_ascii_lowercase())
            .as_deref(),
        Some("1" | "true" | "yes" | "on")
    )
}

/// Claude Code's environment: ours, marked as the TypeScript SDK's, as query() and ProcessTransport set it.
pub fn claude_env(mut env: Vec<(String, String)>) -> Vec<(String, String)> {
    let has = |env: &Vec<(String, String)>, name: &str| {
        env.iter().any(|(key, _)| key.eq_ignore_ascii_case(name))
    };
    // Without an env option the SDK clears these from its own.
    for name in [
        "CLAUDE_CODE_QUESTION_EXTENDED",
        "CLAUDE_CODE_QUESTION_OPTIONAL_DESCRIPTIONS",
        "CLAUDE_CODE_AGENT_PROGRESS_SUMMARIES",
    ] {
        env.retain(|(key, _)| !key.eq_ignore_ascii_case(name));
    }
    if !env.iter().any(|(key, _)| key == "CLAUDE_CODE_ENTRYPOINT") {
        env.push(("CLAUDE_CODE_ENTRYPOINT".into(), "sdk-ts".into()));
    }
    if !env.iter().any(|(key, _)| key == "CLAUDE_AGENT_SDK_VERSION") {
        env.push(("CLAUDE_AGENT_SDK_VERSION".into(), AGENT_SDK_VERSION.into()));
    }
    if !has(&env, "CLAUDE_CODE_SDK_READS_SESSION_STATE") {
        env.push(("CLAUDE_CODE_SDK_READS_SESSION_STATE".into(), "1".into()));
    }
    env.retain(|(key, _)| key != "NODE_OPTIONS");
    let debug = truthy(
        env.iter()
            .find(|(key, _)| key == "DEBUG_CLAUDE_AGENT_SDK")
            .map(|(_, value)| value.as_str()),
    );
    env.retain(|(key, _)| key != "DEBUG");
    if debug {
        env.push(("DEBUG".into(), "1".into()));
    }
    env
}

/// The initialize control request, and the hook callbacks by the ids it gives them.
pub fn initialize_request(options: &QueryOptions) -> (Value, HashMap<String, HookCallback>) {
    let mut request = Map::new();
    request.insert("subtype".into(), json!("initialize"));
    let mut callbacks = HashMap::new();
    if !options.hooks.is_empty() {
        let mut payload = Map::new();
        for (event, matchers) in &options.hooks {
            if matchers.is_empty() {
                continue;
            }
            let entries: Vec<Value> = matchers
                .iter()
                .map(|matcher| {
                    let ids: Vec<String> = matcher
                        .hooks
                        .iter()
                        .map(|hook| {
                            let id = format!("hook_{}", callbacks.len());
                            callbacks.insert(id.clone(), hook.clone());
                            id
                        })
                        .collect();
                    let mut entry = Map::new();
                    if let Some(pattern) = &matcher.matcher {
                        entry.insert("matcher".into(), json!(pattern));
                    }
                    entry.insert("hookCallbackIds".into(), json!(ids));
                    Value::Object(entry)
                })
                .collect();
            payload.insert(event.clone(), Value::Array(entries));
        }
        request.insert("hooks".into(), Value::Object(payload));
    }
    if let Some(append) = &options.append_system_prompt {
        request.insert("appendSystemPrompt".into(), json!(append));
    }
    (Value::Object(request), callbacks)
}

/// The prompt as the SDK writes a string prompt to stdin.
pub fn user_message(prompt: &str) -> Value {
    json!({
        "type": "user",
        "session_id": "",
        "message": { "role": "user", "content": [{ "type": "text", "text": prompt }] },
        "parent_tool_use_id": null,
    })
}

/// Runs Claude Code: the real query().
#[derive(Debug, Clone)]
pub struct ClaudeCli {
    program: Option<PathBuf>,
}

impl ClaudeCli {
    /// The bundled Claude Code, or the one on PATH (cli::find_claude).
    pub fn new() -> Self {
        ClaudeCli {
            program: cli::find_claude(),
        }
    }

    pub fn at(program: PathBuf) -> Self {
        ClaudeCli {
            program: Some(program),
        }
    }
}

impl Default for ClaudeCli {
    fn default() -> Self {
        Self::new()
    }
}

impl RunQuery for ClaudeCli {
    fn query(&self, prompt: String, options: QueryOptions) -> Messages {
        let (tx, rx) = mpsc::channel(64);
        let program = self.program.clone();
        tokio::spawn(async move {
            if let Err(error) = drive(program, prompt, options, tx.clone()).await {
                let _ = tx.send(Err(error)).await;
            }
        });
        rx
    }
}

const ABORTED: &str = "Claude Code process aborted by user";
/// How long the SDK waits after a result for the session to go idle, as background work can still ask.
const RUN_END_CEILING: Duration = Duration::from_secs(600);
/// What the SDK keeps of stderr for its errors.
const STDERR_TAIL: usize = 2048;

/// The SDK's bookkeeping for when a single-prompt run is over (stdin closes then, and Claude Code exits),
/// and for the error text a failed exit is reported with.
#[derive(Default)]
struct Run {
    session_state: Option<String>,
    result_received: bool,
    run_ended: bool,
    ceiling: Option<Instant>,
    last_error_result_text: Option<String>,
    error_text_before_model_less_turns: Option<String>,
    model_less_turn_ended: bool,
}

impl Run {
    fn end(&mut self, write: &mpsc::UnboundedSender<Option<String>>) {
        if self.run_ended {
            return;
        }
        self.run_ended = true;
        self.ceiling = None;
        let _ = write.send(None);
    }

    fn arm_ceiling(&mut self) {
        self.ceiling = None;
        if !self.run_ended {
            self.ceiling = Some(Instant::now() + RUN_END_CEILING);
        }
    }

    /// Notes a message; false when it is the host's own and isn't passed on.
    fn note(
        &mut self,
        message: &Value,
        bidirectional: bool,
        write: &mpsc::UnboundedSender<Option<String>>,
    ) -> bool {
        let kind = message.get("type").and_then(Value::as_str).unwrap_or("");
        let subtype = message.get("subtype").and_then(Value::as_str).unwrap_or("");
        if kind == "system"
            && matches!(
                subtype,
                "post_turn_summary" | "task_summary" | "session_metadata"
            )
            || kind == "active_goal"
            || kind == "autocompact_state"
        {
            return true;
        }
        if kind == "result" {
            let is_error = message.get("is_error").and_then(Value::as_bool) == Some(true);
            let text = if !is_error {
                None
            } else if subtype == "success" {
                message
                    .get("result")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            } else {
                let errors = message.get("errors").and_then(Value::as_array);
                Some(
                    errors
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(str::trim)
                        .filter(|error| !error.is_empty())
                        .collect::<Vec<_>>()
                        .join("; "),
                )
            };
            self.last_error_result_text = text.filter(|text| !text.is_empty());
            self.model_less_turn_ended = subtype == "success"
                && !is_error
                && message.get("num_turns").and_then(Value::as_f64) == Some(0.0)
                && message.get("result").and_then(Value::as_str) == Some("");
            if !self.model_less_turn_ended {
                self.error_text_before_model_less_turns = self.last_error_result_text.clone();
            }
            self.result_received = true;
            if self
                .session_state
                .as_deref()
                .is_none_or(|state| state == "idle")
                || !bidirectional
            {
                self.end(write);
            } else {
                self.arm_ceiling();
            }
        } else if kind == "system" && subtype == "session_state_changed" {
            let state = message.get("state").and_then(Value::as_str).unwrap_or("");
            self.session_state = Some(state.to_string());
            if state == "idle" {
                if self.result_received {
                    self.end(write);
                }
            } else if !self.run_ended {
                if state == "requires_action" {
                    self.ceiling = None;
                } else if self.result_received {
                    self.arm_ceiling();
                }
            }
            if message.get("sdk_host_only") == Some(&Value::Bool(true)) {
                return false;
            }
        } else {
            self.last_error_result_text = None;
            if kind == "system" && subtype == "init" {
                self.model_less_turn_ended = false;
            } else if kind != "system" || subtype == "compact_boundary" {
                self.error_text_before_model_less_turns = None;
                self.model_less_turn_ended = false;
            }
            if (kind == "assistant" || kind == "stream_event")
                && message.get("parent_tool_use_id") == Some(&Value::Null)
                && !self.run_ended
            {
                self.ceiling = None;
            }
        }
        true
    }
}

fn control_line(value: Value) -> Option<String> {
    Some(format!("{value}\n"))
}

/// Answers one control request from Claude Code, as Query.processControlRequest does.
fn answer_control_request(
    message: Value,
    can_use_tool: Option<CanUseTool>,
    hooks: Arc<HashMap<String, HookCallback>>,
    write: mpsc::UnboundedSender<Option<String>>,
) {
    tokio::spawn(async move {
        let request_id = message.get("request_id").cloned().unwrap_or(Value::Null);
        let request = message.get("request").cloned().unwrap_or(Value::Null);
        let text = |key: &str| request.get(key).and_then(Value::as_str).map(str::to_string);
        let subtype = text("subtype").unwrap_or_default();
        let outcome: Result<Value, String> = match subtype.as_str() {
            "can_use_tool" => match can_use_tool {
                None => Err("canUseTool callback is not provided.".into()),
                Some(can_use_tool) => {
                    let input = request.get("input").cloned().unwrap_or(Value::Null);
                    let mut answer = can_use_tool(text("tool_name").unwrap_or_default(), input)
                        .await
                        .to_json();
                    if let Some(id) = request.get("tool_use_id") {
                        answer["toolUseID"] = id.clone();
                    }
                    Ok(answer)
                }
            },
            "hook_callback" => {
                let id = text("callback_id").unwrap_or_default();
                match hooks.get(&id) {
                    None => Err(format!("No hook callback found for ID: {id}")),
                    Some(hook) => {
                        let input = request.get("input").cloned().unwrap_or(Value::Null);
                        Ok(hook(input, text("tool_use_id")).await)
                    }
                }
            }
            "mcp_message" => Err(format!(
                "SDK MCP server not found: {}",
                text("server_name").unwrap_or_default()
            )),
            "elicitation" => Ok(json!({ "action": "decline" })),
            // Left unanswered, as the SDK leaves them for whoever can.
            "request_user_dialog"
            | "remote_tool_call"
            | "remote_plumbing_call"
            | "remote_tools_probe"
            | "remote_tools_reannounce" => return,
            other => Err(format!("Unsupported control request subtype: {other}")),
        };
        let response = match outcome {
            Ok(response) => {
                json!({ "subtype": "success", "request_id": request_id, "response": response })
            }
            Err(error) => json!({ "subtype": "error", "request_id": request_id, "error": error }),
        };
        let _ = write.send(control_line(
            json!({ "type": "control_response", "response": response }),
        ));
    });
}

fn keep_tail(tail: &mut String) {
    let len = tail.chars().count();
    if len > 2 * STDERR_TAIL {
        *tail = tail.chars().skip(len - STDERR_TAIL).collect();
    }
}

/// Like the SDK's close() on abort: stdin first, then the process if it hasn't gone after two seconds.
fn abort(
    mut child: tokio::process::Child,
    write: &mpsc::UnboundedSender<Option<String>>,
) -> anyhow::Error {
    let _ = write.send(None);
    tokio::spawn(async move {
        if tokio::time::timeout(Duration::from_secs(2), child.wait())
            .await
            .is_err()
        {
            let _ = child.kill().await;
        }
    });
    anyhow!(ABORTED)
}

async fn drive(
    program: Option<PathBuf>,
    prompt: String,
    options: QueryOptions,
    tx: mpsc::Sender<anyhow::Result<Value>>,
) -> anyhow::Result<()> {
    let program = program.ok_or_else(|| {
        anyhow!("Couldn't find Claude Code (claude) next to Linker-Edit or on PATH.")
    })?;
    if options.abort.is_cancelled() {
        bail!(ABORTED);
    }
    let env: Vec<(String, String)> = std::env::vars_os()
        .filter_map(|(key, value)| Some((key.into_string().ok()?, value.into_string().ok()?)))
        .collect();
    let mut command = cli::command(&program);
    command
        .args(claude_args(&options))
        .env_clear()
        .envs(claude_env(env))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    if let Some(cwd) = &options.cwd {
        command.current_dir(cwd);
    }
    let mut child = command
        .spawn()
        .map_err(|error| anyhow!("Failed to spawn Claude Code process: {error}"))?;

    // One writer owns stdin; None closes it, which ends a single-prompt run.
    let (write, mut writes) = mpsc::unbounded_channel::<Option<String>>();
    let mut stdin = child.stdin.take().context("Claude Code has no stdin")?;
    tokio::spawn(async move {
        while let Some(Some(line)) = writes.recv().await {
            if stdin.write_all(line.as_bytes()).await.is_err() || stdin.flush().await.is_err() {
                break;
            }
        }
    });
    let stderr_tail = Arc::new(Mutex::new(String::new()));
    let stderr_reader = child.stderr.take().map(|mut stderr| {
        let tail = stderr_tail.clone();
        tokio::spawn(async move {
            let mut buffer = [0u8; 4096];
            while let Ok(read) = stderr.read(&mut buffer).await {
                if read == 0 {
                    break;
                }
                let mut tail = tail.lock().unwrap();
                tail.push_str(&String::from_utf8_lossy(&buffer[..read]));
                keep_tail(&mut tail);
            }
        })
    });
    let stdout = child.stdout.take().context("Claude Code has no stdout")?;

    let (initialize, hooks) = initialize_request(&options);
    let bidirectional = !hooks.is_empty() || options.can_use_tool.is_some();
    let hooks = Arc::new(hooks);
    let request_id = uuid::Uuid::new_v4().simple().to_string();
    let _ = write.send(control_line(
        json!({ "request_id": request_id, "type": "control_request", "request": initialize }),
    ));
    let _ = write.send(control_line(user_message(&prompt)));

    let mut run = Run::default();
    let mut lines = BufReader::new(stdout).lines();
    loop {
        let ceiling = run.ceiling;
        let line = tokio::select! {
            _ = options.abort.cancelled() => return Err(abort(child, &write)),
            _ = tokio::time::sleep_until(ceiling.unwrap_or_else(Instant::now)), if ceiling.is_some() => {
                run.end(&write);
                continue;
            }
            line = lines.next_line() => line.context("Couldn't read Claude Code's output")?,
        };
        let Some(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            tracing::debug!("Non-JSON stdout from Claude Code: {line}");
            continue;
        };
        match message.get("type").and_then(Value::as_str) {
            Some("control_response") => {
                if message["response"]["subtype"] == "error" {
                    tracing::warn!(
                        "Claude Code refused a control request: {}",
                        message["response"]["error"]
                    );
                }
                continue;
            }
            Some("control_request") => {
                answer_control_request(
                    message,
                    options.can_use_tool.clone(),
                    hooks.clone(),
                    write.clone(),
                );
                continue;
            }
            // Our callbacks take no signal, so a cancelled request is answered when it settles, as the TS was.
            Some("control_cancel_request" | "keep_alive" | "transcript_mirror") => continue,
            _ => {}
        }
        if !run.note(&message, bidirectional, &write) {
            continue;
        }
        if tx.send(Ok(message)).await.is_err() {
            // Nobody reads any more; dropping the child kills it.
            return Ok(());
        }
    }

    let status = tokio::select! {
        _ = options.abort.cancelled() => None,
        status = child.wait() => Some(status.context("Claude Code didn't exit cleanly")?),
    };
    let Some(status) = status else {
        return Err(abort(child, &write));
    };
    if let Some(reader) = stderr_reader {
        let _ = tokio::time::timeout(Duration::from_millis(200), reader).await;
    }
    if !status.success() {
        let text = run.last_error_result_text.or(if run.model_less_turn_ended {
            run.error_text_before_model_less_turns
        } else {
            None
        });
        if let Some(text) = text {
            bail!("Claude Code returned an error result: {text}");
        }
        let tail = stderr_tail.lock().unwrap().trim().to_string();
        let tail: String = tail
            .chars()
            .skip(tail.chars().count().saturating_sub(STDERR_TAIL))
            .collect();
        let tail = if tail.is_empty() {
            String::new()
        } else {
            format!(". stderr: {tail}")
        };
        match status.code() {
            Some(code) => bail!("Claude Code process exited with code {code}{tail}"),
            None => bail!("Claude Code process terminated by a signal{tail}"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::claude::{ClaudeSessionOptions, create_claude_session};
    use crate::providers::replay::read_fixture;
    use crate::providers::testing::{
        FakeCliRun, collecting_sink, fake_cli, fixtures, test_config, test_gate,
    };
    use crate::providers::types::{ItemKind, ProviderSession};
    use linker_edit_protocol::{ApprovalMode, ToolStatus};

    async fn run_to_end(mut run: Messages) -> (Vec<Value>, Option<String>) {
        let mut messages = Vec::new();
        while let Some(message) = run.recv().await {
            match message {
                Ok(message) => messages.push(message),
                Err(error) => return (messages, Some(error.to_string())),
            }
        }
        (messages, None)
    }

    fn options(fake: &FakeCliRun) -> QueryOptions {
        QueryOptions {
            cwd: Some(fake.dir.path().to_path_buf()),
            ..QueryOptions::default()
        }
    }

    /// The control_response the fake read for one of its requests.
    fn response(fake: &FakeCliRun, id: &str) -> Value {
        fake.stdin()
            .into_iter()
            .find(|line| line["type"] == "control_response" && line["response"]["request_id"] == id)
            .unwrap()["response"]
            .clone()
    }

    #[tokio::test]
    async fn speaks_the_sdks_stream_json_protocol_with_claude_code() {
        let fake = FakeCliRun::new(json!([
            { "answer": { "commands": [] } },
            { "read": 1 },
            { "out": { "type": "system", "subtype": "init", "session_id": "s1", "mcp_servers": [] } },
            { "request": { "subtype": "can_use_tool", "tool_name": "Write", "input": { "file_path": "a.md" }, "tool_use_id": "toolu_1" }, "id": "r1" },
            { "request": { "subtype": "hook_callback", "callback_id": "hook_0", "input": { "hook_event_name": "PreToolUse", "tool_name": "mcp__resolve__delete_lut" }, "tool_use_id": "toolu_2" }, "id": "r2" },
            { "request": { "subtype": "mcp_message", "server_name": "x", "message": {} }, "id": "r3" },
            { "line": "not JSON, which the SDK skips too" },
            { "out": { "type": "keep_alive" } },
            { "out": { "type": "assistant", "parent_tool_use_id": null, "message": { "content": [] }, "session_id": "s1" } },
            { "out": { "type": "result", "subtype": "success", "is_error": false, "result": "OK", "session_id": "s1" } },
            { "read_to_end": true },
        ]));
        let can_use_tool: CanUseTool = Arc::new(|tool, input| {
            Box::pin(async move {
                if tool == "Write" {
                    PermissionResult::Deny {
                        message: "No.".into(),
                    }
                } else {
                    PermissionResult::Allow {
                        updated_input: input,
                    }
                }
            })
        });
        let hook: HookCallback = Arc::new(|input, id| {
            Box::pin(async move { json!({ "seen": input["tool_name"], "id": id }) })
        });
        let matcher = |hook: &HookCallback| HookMatcher {
            matcher: Some("mcp__resolve__.*".into()),
            hooks: vec![hook.clone()],
        };
        let mcp_servers =
            json!({ "linker": { "type": "stdio", "command": "linker-edit", "args": ["mcp"] } });
        let run = ClaudeCli::at(fake_cli()).query(
            "check".into(),
            QueryOptions {
                resume: Some("earlier".into()),
                append_system_prompt: Some("Edit well.".into()),
                mcp_servers: mcp_servers.as_object().unwrap().clone(),
                disallowed_tools: vec![
                    "mcp__resolve__delete_lut".into(),
                    "mcp__resolve__run_script_unsafe".into(),
                ],
                settings: Some(json!({ "autoCompactWindow": 300_000 })),
                hooks: vec![
                    ("PreToolUse".into(), vec![matcher(&hook)]),
                    ("PostToolUse".into(), vec![matcher(&hook)]),
                ],
                permission_mode: Some(PermissionMode::BypassPermissions),
                allow_dangerously_skip_permissions: true,
                can_use_tool: Some(can_use_tool),
                ..options(&fake)
            },
        );
        let (messages, error) = run_to_end(run).await;
        assert_eq!(error, None);
        let kinds: Vec<&str> = messages
            .iter()
            .map(|message| message["type"].as_str().unwrap())
            .collect();
        assert_eq!(kinds, ["system", "assistant", "result"]);

        let log = fake.log();
        assert_eq!(
            log[0]["argv"],
            json!([
                "--output-format",
                "stream-json",
                "--verbose",
                "--input-format",
                "stream-json",
                "--permission-prompt-tool=stdio",
                "--resume=earlier",
                "--disallowedTools=mcp__resolve__delete_lut,mcp__resolve__run_script_unsafe",
                "--mcp-config",
                json!({ "mcpServers": mcp_servers }).to_string(),
                "--permission-mode=bypassPermissions",
                "--allow-dangerously-skip-permissions",
                "--settings",
                "{\"autoCompactWindow\":300000}",
            ])
        );
        let env = &log[0]["env"];
        let ours = |name: &str, otherwise: &str| {
            std::env::var(name).unwrap_or_else(|_| otherwise.to_string())
        };
        assert_eq!(
            env["CLAUDE_CODE_ENTRYPOINT"],
            ours("CLAUDE_CODE_ENTRYPOINT", "sdk-ts")
        );
        assert_eq!(
            env["CLAUDE_AGENT_SDK_VERSION"],
            ours("CLAUDE_AGENT_SDK_VERSION", AGENT_SDK_VERSION)
        );
        assert_eq!(env["CLAUDE_CODE_SDK_READS_SESSION_STATE"], "1");
        assert!(env.get("NODE_OPTIONS").is_none());

        let stdin = fake.stdin();
        assert_eq!(stdin[0]["type"], "control_request");
        assert_eq!(
            stdin[0]["request"],
            json!({
                "subtype": "initialize",
                "hooks": {
                    "PreToolUse": [{ "matcher": "mcp__resolve__.*", "hookCallbackIds": ["hook_0"] }],
                    "PostToolUse": [{ "matcher": "mcp__resolve__.*", "hookCallbackIds": ["hook_1"] }],
                },
                "appendSystemPrompt": "Edit well.",
            })
        );
        assert_eq!(stdin[1], user_message("check"));
        assert_eq!(
            response(&fake, "r1"),
            json!({ "subtype": "success", "request_id": "r1", "response": { "behavior": "deny", "message": "No.", "toolUseID": "toolu_1" } })
        );
        assert_eq!(
            response(&fake, "r2")["response"],
            json!({ "seen": "mcp__resolve__delete_lut", "id": "toolu_2" })
        );
        assert_eq!(
            response(&fake, "r3"),
            json!({ "subtype": "error", "request_id": "r3", "error": "SDK MCP server not found: x" })
        );
        // The prompt was the only one, so stdin closes after the result and Claude Code exits.
        assert_eq!(log.last().unwrap(), &json!({ "eof": true }));
    }

    #[tokio::test]
    async fn a_claude_session_runs_a_recorded_turn_through_claude_code() {
        // Claude Code as the fixture recorded it, asking canUseTool for each tool call as it does in ask mode.
        let mut steps = vec![json!({ "answer": {} }), json!({ "read": 1 })];
        let fixture = read_fixture(&fixtures().join("claude-turn.jsonl")).unwrap();
        for message in &fixture {
            steps.push(json!({ "out": message }));
            for block in message["message"]["content"]
                .as_array()
                .into_iter()
                .flatten()
            {
                if message["type"] == "assistant" && block["type"] == "tool_use" {
                    steps.push(json!({
                        "request": { "subtype": "can_use_tool", "tool_name": block["name"], "input": block["input"], "tool_use_id": block["id"] },
                        "id": block["id"],
                    }));
                }
            }
        }
        steps.push(json!({ "read_to_end": true }));
        let fake = FakeCliRun::new(Value::Array(steps));
        let mut config = test_config();
        config.workspace_dir = fake.dir.path().to_path_buf();
        let claude = create_claude_session(
            config,
            Arc::new(|| ApprovalMode::Ask),
            test_gate(),
            ClaudeSessionOptions {
                run_query: Arc::new(ClaudeCli::at(fake_cli())),
                ..ClaudeSessionOptions::new(|| "Edit well.".into())
            },
        );
        let sink = collecting_sink();
        claude
            .send("check", sink.clone(), CancellationToken::new())
            .await
            .unwrap();

        let asked: Vec<String> = sink.approvals().into_iter().map(|(tool, _)| tool).collect();
        assert_eq!(asked, ["mcp__resolve__run_script", "Write"]);
        let tools: Vec<_> = sink
            .items()
            .into_iter()
            .filter(|item| item.kind == Some(ItemKind::Tool))
            .collect();
        assert_eq!(tools.len(), 6);
        assert!(
            tools
                .iter()
                .all(|tool| tool.status == Some(ToolStatus::Done))
        );
        assert!(
            sink.items()
                .pop()
                .unwrap()
                .text
                .unwrap()
                .contains("Resolve is reachable")
        );
        assert_eq!(
            claude.resume_id().as_deref(),
            fixture[0]["session_id"].as_str()
        );
        let used = sink.contexts().pop().unwrap().unwrap();
        assert_eq!(
            (used.used_tokens, used.max_tokens),
            (44_961, Some(1_000_000))
        );
        // Every tool call got its answer; Linker-Edit's own tools are allowed without asking.
        let answers: Vec<Value> = fake
            .stdin()
            .into_iter()
            .filter(|line| line["type"] == "control_response")
            .map(|line| line["response"]["response"]["behavior"].clone())
            .collect();
        assert_eq!(answers, vec![json!("allow"); 6]);
    }

    #[tokio::test]
    async fn reports_a_failed_run_with_its_error_result_or_exit_code() {
        let fake = FakeCliRun::new(json!([
            { "answer": {} },
            { "read": 1 },
            { "out": { "type": "result", "subtype": "success", "is_error": true, "result": "Invalid API key", "session_id": "s" } },
            { "read_to_end": true },
            { "exit": 1 },
        ]));
        let (messages, error) =
            run_to_end(ClaudeCli::at(fake_cli()).query("go".into(), options(&fake))).await;
        assert_eq!(messages.len(), 1);
        assert_eq!(
            error.as_deref(),
            Some("Claude Code returned an error result: Invalid API key")
        );

        let fake = FakeCliRun::new(json!([{ "stderr": "No conversation found" }, { "exit": 2 }]));
        let (_, error) =
            run_to_end(ClaudeCli::at(fake_cli()).query("go".into(), options(&fake))).await;
        assert_eq!(
            error.as_deref(),
            Some("Claude Code process exited with code 2. stderr: No conversation found")
        );

        let (_, error) =
            run_to_end(ClaudeCli { program: None }.query("go".into(), options(&fake))).await;
        assert!(error.unwrap().starts_with("Couldn't find Claude Code"));
    }

    #[tokio::test]
    async fn stops_claude_code_when_the_turn_is_aborted() {
        let fake = FakeCliRun::new(json!([
            { "answer": {} },
            { "read": 1 },
            { "out": { "type": "system", "subtype": "init", "session_id": "s", "mcp_servers": [] } },
            { "read_to_end": true },
        ]));
        let abort = CancellationToken::new();
        let mut run = ClaudeCli::at(fake_cli()).query(
            "go".into(),
            QueryOptions {
                abort: abort.clone(),
                ..options(&fake)
            },
        );
        assert_eq!(run.recv().await.unwrap().unwrap()["subtype"], "init");
        abort.cancel();
        assert_eq!(
            run.recv().await.unwrap().unwrap_err().to_string(),
            "Claude Code process aborted by user"
        );
        assert!(run.recv().await.is_none());
    }

    #[tokio::test]
    async fn keeps_stdin_open_after_the_result_while_claude_code_is_still_working() {
        // Background work can still ask for a tool after the result; stdin closes once the session is idle.
        let fake = FakeCliRun::new(json!([
            { "answer": {} },
            { "read": 1 },
            { "out": { "type": "system", "subtype": "session_state_changed", "state": "running" } },
            { "out": { "type": "result", "subtype": "success", "is_error": false, "result": "", "session_id": "s" } },
            { "request": { "subtype": "can_use_tool", "tool_name": "Read", "input": {} }, "id": "late" },
            { "out": { "type": "system", "subtype": "session_state_changed", "state": "idle" } },
            { "read_to_end": true },
        ]));
        let allow: CanUseTool = Arc::new(|_, input| {
            Box::pin(async move {
                PermissionResult::Allow {
                    updated_input: input,
                }
            })
        });
        let run = ClaudeCli::at(fake_cli()).query(
            "go".into(),
            QueryOptions {
                can_use_tool: Some(allow),
                ..options(&fake)
            },
        );
        let (messages, error) = run_to_end(run).await;
        assert_eq!((messages.len(), error), (3, None));
        assert_eq!(response(&fake, "late")["response"]["behavior"], "allow");
        assert_eq!(fake.log().last().unwrap(), &json!({ "eof": true }));
    }

    /// One real turn with the Claude Code on this computer: no MCP servers or settings, no tools allowed, in an
    /// empty folder. Costs a few tokens: `cargo test -p linker-edit-server live_claude -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore = "runs the real Claude Code"]
    async fn live_claude_turn() {
        let dir = tempfile::tempdir().unwrap();
        let deny: CanUseTool = Arc::new(|_, _| {
            Box::pin(async {
                PermissionResult::Deny {
                    message: "No tools in this test.".into(),
                }
            })
        });
        let hook: HookCallback = Arc::new(|_, _| Box::pin(async { json!({}) }));
        let run = ClaudeCli::new().query(
            "Reply with OK and nothing else.".into(),
            QueryOptions {
                cwd: Some(dir.path().to_path_buf()),
                model: Some("haiku".into()),
                setting_sources: Some(vec![]),
                strict_mcp_config: true,
                append_system_prompt: Some("Answer in one word.".into()),
                hooks: vec![(
                    "PreToolUse".into(),
                    vec![HookMatcher {
                        matcher: None,
                        hooks: vec![hook],
                    }],
                )],
                permission_mode: Some(PermissionMode::Default),
                can_use_tool: Some(deny),
                ..QueryOptions::default()
            },
        );
        let (messages, error) = run_to_end(run).await;
        for message in &messages {
            println!(
                "{}",
                message.to_string().chars().take(300).collect::<String>()
            );
        }
        assert_eq!(error, None);
        let init = messages.iter().find(|m| m["subtype"] == "init").unwrap();
        assert_eq!(init["mcp_servers"], json!([]));
        let result = messages.iter().find(|m| m["type"] == "result").unwrap();
        assert_eq!(result["subtype"], "success");
        assert!(result["result"].as_str().unwrap().contains("OK"));
    }
}
