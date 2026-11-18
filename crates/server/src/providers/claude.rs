// A Claude conversation: one Claude Code run per turn (claude_cli.rs), continued by session id. Turns the SDK
// messages into chat items, keeps the context meter, and puts Linker-Edit's guardrails in front of Resolve.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use linker_edit_protocol::{ApprovalMode, ContextUsage, Tone, ToolStatus};
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

use super::claude_cli::{
    CanUseTool, ClaudeCli, HookCallback, HookMatcher, PermissionMode, PermissionResult,
    QueryOptions, RunQuery,
};
use super::shorten::{MAX_TOOL_OUTPUT_CHARS, shorten_tool_output};
use super::types::{
    BoxFuture, CLAUDE_DISALLOWED_RESOLVE_TOOLS, ItemKind, ItemPatch, LINKER_MCP_NAME,
    ProviderSession, RESOLVE_MCP_NAME, RESOLVE_RUN_SCRIPT, TRUNCATE_MAX, ToolDecision, ToolGate,
    TurnSink, truncate,
};
use crate::agents::commands::linker_mcp_command;
use crate::config::AppConfig;

pub type ApprovalModeFn = Arc<dyn Fn() -> ApprovalMode + Send + Sync>;

pub struct ClaudeSessionOptions {
    pub run_query: Arc<dyn RunQuery>,
    /// A session id from an earlier run, to continue that conversation.
    pub resume_id: Option<String>,
    /// Milliseconds since the epoch, as Date.now().
    pub now: Arc<dyn Fn() -> i64 + Send + Sync>,
    /// The agent's rules for the project, read when each turn starts.
    pub rules: Arc<dyn Fn() -> String + Send + Sync>,
}

impl ClaudeSessionOptions {
    /// Claude Code itself, the clock, and `rules` (rules.rs's EDITING_RULES where the project has none of its
    /// own; the TypeScript defaulted to those, which the Resolve port owns).
    pub fn new(rules: impl Fn() -> String + Send + Sync + 'static) -> Self {
        ClaudeSessionOptions {
            run_query: Arc::new(ClaudeCli::new()),
            resume_id: None,
            now: Arc::new(|| chrono::Utc::now().timestamp_millis()),
            rules: Arc::new(rules),
        }
    }
}

pub const CONTEXT_OVERFLOW_NOTICE: &str = "The chat no longer fits in Claude's context. Compact it from the context meter, or start a new chat.";

fn tool_result_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .map(|part| match part.get("type").and_then(Value::as_str) {
                Some("text") => part
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                kind => format!("[{}]", kind.unwrap_or("undefined")),
            })
            .collect::<Vec<_>>()
            .join("\n"),
        content => content.to_string(),
    }
}

fn number(value: &Value) -> f64 {
    value.as_f64().unwrap_or(0.0)
}

/// How long the cache this request wrote lives; None when it wrote none, as when fully served from cache.
fn cache_ttl_ms(usage: &Value) -> Option<i64> {
    let creation = &usage["cache_creation"];
    if number(&creation["ephemeral_1h_input_tokens"]) > 0.0 {
        Some(60 * 60_000)
    } else if number(&creation["ephemeral_5m_input_tokens"]) > 0.0 {
        Some(5 * 60_000)
    } else {
        None
    }
}

/// new Date(ms).toISOString().
fn iso(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap_or_default()
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// toLocaleString("en-US") for a whole number: 250000 is "250,000".
fn with_commas(value: f64) -> String {
    let digits = format!("{}", value.abs().round() as u64);
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(digit);
    }
    if value < 0.0 { format!("-{out}") } else { out }
}

// Claude Code refuses bypassPermissions as root; supervised permissions still work there.
fn runs_as_root() -> bool {
    #[cfg(unix)]
    {
        // SAFETY: getuid has no preconditions and can't fail.
        unsafe { libc::getuid() == 0 }
    }
    #[cfg(not(unix))]
    {
        false
    }
}

/// Like T3 Code: full access is bypassPermissions, supervised is "default" with canUseTool asking. The mode
/// is read when a turn starts; Linker-Edit's own Resolve tools follow a switch straight away.
pub fn claude_permission_mode(mode: ApprovalMode) -> PermissionMode {
    if mode == ApprovalMode::Auto && !runs_as_root() {
        PermissionMode::BypassPermissions
    } else {
        PermissionMode::Default
    }
}

#[derive(Default)]
struct State {
    session_id: Option<String>,
    max_tokens: Option<u32>,
    used_tokens: Option<u32>,
    model: Option<String>,
    // A read served fully from cache still refreshes it for the TTL it was written with.
    ttl_ms: Option<i64>,
}

pub struct ClaudeSession {
    config: AppConfig,
    approval_mode: ApprovalModeFn,
    gate: Arc<dyn ToolGate>,
    run_query: Arc<dyn RunQuery>,
    now: Arc<dyn Fn() -> i64 + Send + Sync>,
    rules: Arc<dyn Fn() -> String + Send + Sync>,
    state: Mutex<State>,
}

pub fn create_claude_session(
    config: AppConfig,
    approval_mode: ApprovalModeFn,
    gate: Arc<dyn ToolGate>,
    options: ClaudeSessionOptions,
) -> ClaudeSession {
    ClaudeSession {
        config,
        approval_mode,
        gate,
        run_query: options.run_query,
        now: options.now,
        rules: options.rules,
        state: Mutex::new(State {
            session_id: options.resume_id,
            ..State::default()
        }),
    }
}

impl ClaudeSession {
    fn report_context(&self, state: &State, sink: &dyn TurnSink) {
        if let Some(used_tokens) = state.used_tokens {
            sink.context(Some(ContextUsage {
                used_tokens,
                max_tokens: state.max_tokens,
                cache_expires_at: state.ttl_ms.map(|ttl| iso((self.now)() + ttl)),
            }));
        }
    }

    fn handle(&self, message: &Value, sink: &dyn TurnSink) {
        let mut state = self.state.lock().unwrap();
        if let Some(id) = message.get("session_id").and_then(Value::as_str) {
            state.session_id = Some(id.to_string());
        }
        match message.get("type").and_then(Value::as_str) {
            Some("system") => match message.get("subtype").and_then(Value::as_str) {
                Some("init") => {
                    for server in message["mcp_servers"].as_array().into_iter().flatten() {
                        let name = server["name"].as_str().unwrap_or("");
                        let status = server["status"].as_str().unwrap_or("");
                        if (name == RESOLVE_MCP_NAME || name == LINKER_MCP_NAME)
                            && status != "connected"
                        {
                            sink.notice(format!("MCP server \"{name}\" is {status}."), Tone::Error);
                        }
                    }
                }
                Some("compact_boundary") => {
                    let before = with_commas(number(&message["compact_metadata"]["pre_tokens"]));
                    sink.notice(
                        format!("Compacted the chat to save context. It was {before} tokens."),
                        Tone::Info,
                    );
                    // post_tokens leaves out the system prompt and tools, so the size is unknown until the
                    // next request.
                    state.used_tokens = None;
                    state.ttl_ms = None;
                    sink.context(None);
                }
                _ => {}
            },
            Some("assistant") => {
                // Subagents have their own context; only the main conversation's fills this one.
                if message.get("parent_tool_use_id") == Some(&Value::Null) {
                    let inner = &message["message"];
                    state.model = inner["model"].as_str().map(str::to_string);
                    let usage = &inner["usage"];
                    let used = number(&usage["input_tokens"])
                        + number(&usage["cache_creation_input_tokens"])
                        + number(&usage["cache_read_input_tokens"])
                        + number(&usage["output_tokens"]);
                    state.used_tokens = Some(used as u32);
                    state.ttl_ms = cache_ttl_ms(usage).or(state.ttl_ms);
                    self.report_context(&state, sink);
                }
                let uuid = message["uuid"].as_str().unwrap_or("undefined");
                for (index, block) in message["message"]["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .enumerate()
                {
                    match block["type"].as_str() {
                        Some("text") => {
                            let text = block["text"].as_str().unwrap_or("");
                            if !text.trim().is_empty() {
                                sink.upsert(ItemPatch {
                                    id: format!("{uuid}:{index}"),
                                    kind: Some(ItemKind::Assistant),
                                    text: Some(text.to_string()),
                                    ..ItemPatch::default()
                                });
                            }
                        }
                        Some("tool_use") => sink.upsert(ItemPatch {
                            id: block["id"].as_str().unwrap_or("").to_string(),
                            kind: Some(ItemKind::Tool),
                            name: block["name"].as_str().map(str::to_string),
                            input: Some(block["input"].clone()),
                            status: Some(ToolStatus::Running),
                            ..ItemPatch::default()
                        }),
                        _ => {}
                    }
                }
            }
            Some("user") => {
                for block in message["message"]["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                {
                    if block["type"] == "tool_result" {
                        let failed = block["is_error"].as_bool() == Some(true);
                        sink.upsert(ItemPatch {
                            id: block["tool_use_id"].as_str().unwrap_or("").to_string(),
                            status: Some(if failed {
                                ToolStatus::Error
                            } else {
                                ToolStatus::Done
                            }),
                            output: Some(truncate(
                                &tool_result_text(&block["content"]),
                                TRUNCATE_MAX,
                            )),
                            ..ItemPatch::default()
                        });
                    }
                }
            }
            Some("result") => {
                // modelUsage also lists models that only ran subagents or the compaction itself.
                let window = state
                    .model
                    .as_ref()
                    .and_then(|model| message["modelUsage"][model]["contextWindow"].as_f64())
                    .filter(|window| *window != 0.0);
                if let Some(window) = window {
                    state.max_tokens = Some(window as u32);
                    self.report_context(&state, sink);
                }
                let subtype = message["subtype"].as_str().unwrap_or("undefined");
                let overflow = message["result"].as_str().is_some_and(|result| {
                    let result = result.to_lowercase();
                    result.contains("prompt is too long") || result.contains("context window")
                });
                if subtype != "success" {
                    sink.notice(format!("Claude stopped ({subtype})."), Tone::Error);
                } else if message["is_error"] == true && overflow {
                    sink.notice(CONTEXT_OVERFLOW_NOTICE.into(), Tone::Error);
                }
            }
            _ => {}
        }
    }

    fn mcp_servers(&self) -> Map<String, Value> {
        let mut servers = Map::new();
        // Resolve's own server, for its scripting docs and, after the user's OK, run_script.
        if let Some(command) = &self.config.resolve_mcp.command {
            servers.insert(
                RESOLVE_MCP_NAME.into(),
                json!({ "type": "stdio", "command": command.display().to_string(), "args": [] }),
            );
        }
        // Loaded up front, so a chat doesn't start with a tool search before it can touch Resolve.
        let linker = linker_mcp_command(&self.config);
        servers.insert(
            LINKER_MCP_NAME.into(),
            json!({ "type": "stdio", "command": linker.command, "args": linker.args, "env": linker.env, "alwaysLoad": true }),
        );
        servers
    }

    fn hooks(&self, sink: &Arc<dyn TurnSink>) -> Vec<(String, Vec<HookMatcher>)> {
        let matcher = Some(format!("mcp__{RESOLVE_MCP_NAME}__.*"));
        let (gate, sink) = (self.gate.clone(), sink.clone());
        // Runs in every permission mode, bypassPermissions included: refuses what the policy refuses, and
        // backs the project up before a script changes it.
        let pre_tool_use: HookCallback = Arc::new(move |input: Value, _| {
            let (gate, sink) = (gate.clone(), sink.clone());
            Box::pin(async move {
                if input["hook_event_name"] != "PreToolUse" {
                    return json!({});
                }
                let tool_name = input["tool_name"].as_str().unwrap_or("").to_string();
                let tool_input = match input.get("tool_input") {
                    Some(tool_input) if !tool_input.is_null() => tool_input.clone(),
                    _ => json!({}),
                };
                let mut reason = gate.refusal(&tool_name, &tool_input);
                if reason.is_none() && tool_name == RESOLVE_RUN_SCRIPT {
                    reason = sink.backup().await.err();
                }
                match reason {
                    Some(reason) => json!({ "hookSpecificOutput": {
                        "hookEventName": "PreToolUse",
                        "permissionDecision": "deny",
                        "permissionDecisionReason": reason,
                    } }),
                    None => json!({}),
                }
            })
        });
        let post_tool_use: HookCallback = Arc::new(|input: Value, _| {
            Box::pin(async move {
                if input["hook_event_name"] != "PostToolUse" {
                    return json!({});
                }
                match shorten_tool_output(&input["tool_response"], MAX_TOOL_OUTPUT_CHARS) {
                    None => json!({}),
                    Some(shortened) => json!({ "hookSpecificOutput": {
                        "hookEventName": "PostToolUse",
                        "updatedToolOutput": shortened,
                    } }),
                }
            })
        });
        vec![
            (
                "PreToolUse".into(),
                vec![HookMatcher {
                    matcher: matcher.clone(),
                    hooks: vec![pre_tool_use],
                }],
            ),
            (
                "PostToolUse".into(),
                vec![HookMatcher {
                    matcher,
                    hooks: vec![post_tool_use],
                }],
            ),
        ]
    }

    /// Asked in supervised mode, and under bypass only for what Claude Code still forwards, as T3 Code does.
    fn can_use_tool(&self, sink: &Arc<dyn TurnSink>) -> CanUseTool {
        let (gate, approval_mode, sink) =
            (self.gate.clone(), self.approval_mode.clone(), sink.clone());
        Arc::new(move |tool: String, input: Value| {
            let (gate, approval_mode, sink) = (gate.clone(), approval_mode.clone(), sink.clone());
            Box::pin(async move {
                let ask: BoxFuture<'_, bool> = {
                    let (tool, input) = (tool.clone(), input.clone());
                    Box::pin(async move { sink.request_approval(&tool, &input).await })
                };
                match gate
                    .decide_tool_call(&tool, &input, approval_mode(), ask)
                    .await
                {
                    ToolDecision::Allow => PermissionResult::Allow {
                        updated_input: input,
                    },
                    ToolDecision::Deny { message } => PermissionResult::Deny { message },
                }
            })
        })
    }
}

#[async_trait]
impl ProviderSession for ClaudeSession {
    async fn send(
        &self,
        text: &str,
        sink: Arc<dyn TurnSink>,
        signal: CancellationToken,
    ) -> anyhow::Result<()> {
        // A compaction and the turn after it share one signal; a child token stops with it and no further.
        let abort = signal.child_token();
        let mode = claude_permission_mode((self.approval_mode)());
        let options = QueryOptions {
            cwd: Some(self.config.workspace_dir.clone()),
            resume: self.state.lock().unwrap().session_id.clone(),
            abort,
            append_system_prompt: Some((self.rules)()),
            mcp_servers: self.mcp_servers(),
            disallowed_tools: CLAUDE_DISALLOWED_RESOLVE_TOOLS
                .iter()
                .map(|tool| tool.to_string())
                .collect(),
            settings: self
                .config
                .auto_compact_window
                .map(|window| json!({ "autoCompactWindow": window })),
            hooks: self.hooks(&sink),
            permission_mode: Some(mode),
            allow_dangerously_skip_permissions: mode == PermissionMode::BypassPermissions,
            can_use_tool: Some(self.can_use_tool(&sink)),
            ..QueryOptions::default()
        };
        let mut run = self.run_query.query(text.to_string(), options);
        while let Some(message) = run.recv().await {
            self.handle(&message?, &*sink);
        }
        Ok(())
    }

    fn can_compact(&self) -> bool {
        true
    }

    // Claude Code runs slash commands sent as the prompt; there is nothing to compact before the first turn.
    async fn compact(
        &self,
        sink: Arc<dyn TurnSink>,
        signal: CancellationToken,
    ) -> anyhow::Result<()> {
        if self.state.lock().unwrap().session_id.is_some() {
            self.send("/compact", sink, signal).await?;
        }
        Ok(())
    }

    fn resume_id(&self) -> Option<String> {
        self.state.lock().unwrap().session_id.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::claude_cli::{Messages, messages};
    use crate::providers::replay::{read_fixture, replay_query};
    use crate::providers::testing::{
        CollectingSink, collecting_sink, collecting_sink_answering, collecting_sink_with_backup,
        fixtures, test_config, test_gate,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn turn() -> std::path::PathBuf {
        fixtures().join("claude-turn.jsonl")
    }

    const NOW: i64 = 1_794_139_200_000; // 2026-11-08T12:00:00Z

    fn session(
        run_query: Arc<dyn RunQuery>,
        mode: ApprovalMode,
        resume_id: Option<&str>,
    ) -> ClaudeSession {
        session_with(test_config(), run_query, mode, resume_id)
    }

    fn session_with(
        config: AppConfig,
        run_query: Arc<dyn RunQuery>,
        mode: ApprovalMode,
        resume_id: Option<&str>,
    ) -> ClaudeSession {
        create_claude_session(
            config,
            Arc::new(move || mode),
            test_gate(),
            ClaudeSessionOptions {
                run_query,
                resume_id: resume_id.map(str::to_string),
                now: Arc::new(|| NOW),
                rules: Arc::new(|| "Edit well.".into()),
            },
        )
    }

    type Calls = Arc<Mutex<Vec<(String, QueryOptions)>>>;

    /// A stand-in query() that yields the given messages and records each call's prompt and options.
    fn scripted(turns: Vec<Vec<Value>>) -> (Calls, Arc<dyn RunQuery>) {
        let calls: Calls = Arc::default();
        let turns = Arc::new(Mutex::new(std::collections::VecDeque::from(turns)));
        let record = calls.clone();
        let run_query = move |prompt: String, options: QueryOptions| -> Messages {
            record.lock().unwrap().push((prompt, options));
            messages(turns.lock().unwrap().pop_front().unwrap_or_default())
        };
        (calls, Arc::new(run_query))
    }

    async fn send(session: &ClaudeSession, text: &str, sink: &Arc<CollectingSink>) {
        session
            .send(text, sink.clone(), CancellationToken::new())
            .await
            .unwrap();
    }

    fn tools(sink: &CollectingSink) -> Vec<ItemPatch> {
        sink.items()
            .into_iter()
            .filter(|item| item.kind == Some(ItemKind::Tool))
            .collect()
    }

    #[tokio::test]
    async fn turns_a_recorded_turn_into_chat_items_asking_only_for_changes() {
        let sink = collecting_sink();
        let claude = session(Arc::new(replay_query(turn(), 0)), ApprovalMode::Ask, None);
        send(&claude, "check", &sink).await;

        // Linker-Edit's own tools never ask here: the Linker-Edit server asks for their changes itself.
        let asked: Vec<String> = sink.approvals().into_iter().map(|(tool, _)| tool).collect();
        assert_eq!(asked, ["mcp__resolve__run_script", "Write"]);
        let tools = tools(&sink);
        let named: Vec<(String, ToolStatus)> = tools
            .iter()
            .map(|tool| (tool.name.clone().unwrap(), tool.status.unwrap()))
            .collect();
        let done = |name: &str| (name.to_string(), ToolStatus::Done);
        assert_eq!(
            named,
            [
                done("ToolSearch"),
                done("mcp__resolve__run_script"),
                done("mcp__linker__list_timelines"),
                done("mcp__linker__get_timeline"),
                done("Glob"),
                done("Write"),
            ]
        );
        assert!(tools.iter().all(|tool| tool.output.is_some()));
        let last = sink.items().pop().unwrap();
        assert_eq!(last.kind, Some(ItemKind::Assistant));
        assert!(last.text.unwrap().contains("Resolve is reachable"));
        assert!(
            !sink
                .items()
                .iter()
                .any(|item| item.kind == Some(ItemKind::Notice))
        );
    }

    #[tokio::test]
    async fn shows_a_denied_call_as_failed_with_the_denial() {
        let sink = collecting_sink_answering(|tool, _| tool != "Write");
        let claude = session(Arc::new(replay_query(turn(), 0)), ApprovalMode::Ask, None);
        send(&claude, "check", &sink).await;
        let write = tools(&sink)
            .into_iter()
            .find(|tool| tool.name.as_deref() == Some("Write"))
            .unwrap();
        assert_eq!(write.status, Some(ToolStatus::Error));
        assert_eq!(
            write.output.as_deref(),
            Some("The user declined this action.")
        );
    }

    #[tokio::test]
    async fn runs_a_resolve_script_without_asking_in_auto_mode_but_still_refuses_forbidden_ones_and_resolves_dangerous_tools()
     {
        let (calls, run_query) = scripted(vec![vec![]]);
        let sink = collecting_sink();
        send(&session(run_query, ApprovalMode::Auto, None), "go", &sink).await;
        let options = calls.lock().unwrap()[0].1.clone();
        let can_use_tool = options.can_use_tool.unwrap();
        let ask = |tool: &str, input: Value| can_use_tool(tool.to_string(), input);
        let allowed = |result: PermissionResult| matches!(result, PermissionResult::Allow { .. });

        assert!(allowed(
            ask(
                "mcp__resolve__run_script",
                json!({ "script": "result = project.GetName()" })
            )
            .await
        ));
        assert!(sink.approvals().is_empty());
        assert!(!allowed(
            ask(
                "mcp__resolve__run_script",
                json!({ "script": "resolve.GetProjectManager().LoadProject(\"Other\")" })
            )
            .await
        ));
        assert!(!allowed(
            ask(
                "mcp__resolve__run_script_unsafe",
                json!({ "script": "x = 1" })
            )
            .await
        ));
        assert!(allowed(
            ask(
                "mcp__resolve__search_scripting_api",
                json!({ "pattern": "Marker" })
            )
            .await
        ));
        assert!(sink.approvals().is_empty());
        assert!(
            options
                .disallowed_tools
                .contains(&"mcp__resolve__run_script_unsafe".to_string())
        );
        assert!(
            !options
                .disallowed_tools
                .contains(&"mcp__resolve__launch_resolve".to_string())
        );
    }

    #[tokio::test]
    async fn runs_with_the_permission_mode_t3_code_uses_for_each_approval_mode() {
        for (mode, permission_mode, skip) in [
            (ApprovalMode::Auto, PermissionMode::BypassPermissions, true),
            (ApprovalMode::Ask, PermissionMode::Default, false),
        ] {
            let (calls, run_query) = scripted(vec![vec![]]);
            send(&session(run_query, mode, None), "go", &collecting_sink()).await;
            let options = calls.lock().unwrap()[0].1.clone();
            assert_eq!(options.permission_mode, Some(permission_mode));
            assert_eq!(options.allow_dangerously_skip_permissions, skip);
        }
    }

    /// The PreToolUse hook, which bypassPermissions doesn't skip, as a function of (tool_name, tool_input).
    async fn pre_tool_use(
        sink: Arc<CollectingSink>,
    ) -> impl Fn(&str, Value) -> BoxFuture<'static, Value> {
        let (calls, run_query) = scripted(vec![vec![]]);
        send(&session(run_query, ApprovalMode::Auto, None), "go", &sink).await;
        let (event, matchers) = calls.lock().unwrap()[0].1.hooks[0].clone();
        assert_eq!(event, "PreToolUse");
        assert_eq!(matchers[0].matcher.as_deref(), Some("mcp__resolve__.*"));
        let hook = matchers[0].hooks[0].clone();
        move |tool_name: &str, tool_input: Value| {
            hook(
                json!({ "hook_event_name": "PreToolUse", "tool_name": tool_name, "tool_input": tool_input, "tool_use_id": "t", "session_id": "s", "transcript_path": "", "cwd": "" }),
                Some("t".into()),
            )
        }
    }

    fn denied(reason: &str) -> Value {
        json!({ "hookSpecificOutput": { "hookEventName": "PreToolUse", "permissionDecision": "deny", "permissionDecisionReason": reason } })
    }

    #[tokio::test]
    async fn its_hook_refuses_forbidden_scripts_and_resolves_denied_tools_without_backing_up() {
        let backups = Arc::new(AtomicUsize::new(0));
        let counted = backups.clone();
        let run = pre_tool_use(collecting_sink_with_backup(move || {
            counted.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }))
        .await;
        let refused = run(
            "mcp__resolve__run_script",
            json!({ "script": "pm.LoadProject('Other')" }),
        )
        .await;
        assert!(
            refused["hookSpecificOutput"]["permissionDecisionReason"]
                .as_str()
                .unwrap()
                .contains("doesn't let scripts")
        );
        assert_eq!(
            run("mcp__resolve__delete_lut", json!({})).await,
            denied("Linker-Edit doesn't let agents use delete_lut.")
        );
        assert_eq!(backups.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn its_hook_backs_the_project_up_before_a_script_and_lets_the_rest_through() {
        let backups = Arc::new(AtomicUsize::new(0));
        let counted = backups.clone();
        let run = pre_tool_use(collecting_sink_with_backup(move || {
            counted.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }))
        .await;
        assert_eq!(
            run(
                "mcp__resolve__run_script",
                json!({ "script": "result = 1" })
            )
            .await,
            json!({})
        );
        assert_eq!(
            run("mcp__resolve__launch_resolve", json!({})).await,
            json!({})
        );
        assert_eq!(
            run(
                "mcp__resolve__search_scripting_api",
                json!({ "pattern": "x" })
            )
            .await,
            json!({})
        );
        assert_eq!(backups.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn its_hook_refuses_a_script_when_the_backup_fails() {
        let run = pre_tool_use(collecting_sink_with_backup(|| {
            Err("Linker-Edit couldn't back up.".into())
        }))
        .await;
        assert_eq!(
            run(
                "mcp__resolve__run_script",
                json!({ "script": "result = 1" })
            )
            .await,
            denied("Linker-Edit couldn't back up.")
        );
    }

    #[tokio::test]
    async fn asks_nothing_in_auto_mode() {
        let sink = collecting_sink();
        let claude = session(Arc::new(replay_query(turn(), 0)), ApprovalMode::Auto, None);
        send(&claude, "check", &sink).await;
        assert!(sink.approvals().is_empty());
        assert!(
            tools(&sink)
                .iter()
                .all(|tool| tool.status != Some(ToolStatus::Error))
        );
    }

    #[tokio::test]
    async fn resumes_the_recorded_session_on_the_next_turn() {
        let seen: Arc<Mutex<Vec<QueryOptions>>> = Arc::default();
        let replay = replay_query(turn(), 0);
        let record = seen.clone();
        let run_query = move |prompt: String, options: QueryOptions| -> Messages {
            record.lock().unwrap().push(options.clone());
            replay.query(prompt, options)
        };
        let claude = session(Arc::new(run_query), ApprovalMode::Ask, None);
        send(&claude, "one", &collecting_sink()).await;
        send(&claude, "two", &collecting_sink()).await;
        let init = read_fixture(&turn()).unwrap().remove(0);
        let recorded = init["session_id"].as_str().unwrap().to_string();
        let seen = seen.lock().unwrap();
        let resumes: Vec<Option<String>> =
            seen.iter().map(|options| options.resume.clone()).collect();
        assert_eq!(resumes, [None, Some(recorded.clone())]);
        assert_eq!(claude.resume_id(), Some(recorded));
        let linker = linker_mcp_command(&test_config());
        assert_eq!(
            Value::Object(seen[0].mcp_servers.clone()),
            json!({
                "resolve": { "type": "stdio", "command": r"C:\Program Files\Blackmagic Design\DaVinci Resolve\ResolveMCP.exe", "args": [] },
                "linker": { "type": "stdio", "command": linker.command, "args": ["mcp"], "env": linker.env, "alwaysLoad": true },
            })
        );
    }

    #[tokio::test]
    async fn continues_a_conversation_from_before_a_restart() {
        let (calls, run_query) = scripted(vec![vec![]]);
        let claude = session(run_query, ApprovalMode::Ask, Some("earlier"));
        assert_eq!(claude.resume_id().as_deref(), Some("earlier"));
        send(&claude, "go on", &collecting_sink()).await;
        assert_eq!(
            calls.lock().unwrap()[0].1.resume.as_deref(),
            Some("earlier")
        );
    }

    #[tokio::test]
    async fn reports_how_full_the_context_is_with_the_models_window_and_when_the_cache_expires() {
        let sink = collecting_sink();
        let claude = session(Arc::new(replay_query(turn(), 0)), ApprovalMode::Ask, None);
        send(&claude, "check", &sink).await;
        assert_eq!(
            sink.contexts().pop().unwrap(),
            Some(ContextUsage {
                used_tokens: 44_961,
                max_tokens: Some(1_000_000),
                cache_expires_at: Some("2026-11-08T13:00:00.000Z".into()),
            })
        );
    }

    #[tokio::test]
    async fn compacts_on_request_and_stops_reporting_the_old_size() {
        let boundary = json!({ "type": "system", "subtype": "compact_boundary", "session_id": "s", "compact_metadata": { "trigger": "manual", "pre_tokens": 250_000, "post_tokens": 12_000 } });
        let (calls, run_query) = scripted(vec![
            vec![
                json!({ "type": "system", "subtype": "init", "session_id": "s", "mcp_servers": [] }),
            ],
            vec![boundary],
        ]);
        let claude = session(run_query, ApprovalMode::Ask, None);
        let sink = collecting_sink();
        claude
            .compact(sink.clone(), CancellationToken::new())
            .await
            .unwrap();
        assert!(calls.lock().unwrap().is_empty());

        send(&claude, "one", &sink).await;
        claude
            .compact(sink.clone(), CancellationToken::new())
            .await
            .unwrap();
        let prompts: Vec<(String, Option<String>)> = calls
            .lock()
            .unwrap()
            .iter()
            .map(|(prompt, options)| (prompt.clone(), options.resume.clone()))
            .collect();
        assert_eq!(
            prompts,
            [
                ("one".to_string(), None),
                ("/compact".to_string(), Some("s".to_string()))
            ]
        );
        assert_eq!(
            sink.notice_texts(),
            ["Compacted the chat to save context. It was 250,000 tokens."]
        );
        assert_eq!(sink.contexts(), [None]);
    }

    #[tokio::test]
    async fn passes_the_auto_compact_window_and_shortens_long_resolve_results_before_claude_reads_them()
     {
        let (calls, run_query) = scripted(vec![vec![]]);
        let config = AppConfig {
            auto_compact_window: Some(300_000),
            ..test_config()
        };
        send(
            &session_with(config, run_query, ApprovalMode::Ask, None),
            "go",
            &collecting_sink(),
        )
        .await;
        let options = calls.lock().unwrap()[0].1.clone();
        assert_eq!(
            options.settings,
            Some(json!({ "autoCompactWindow": 300_000 }))
        );
        let (event, matchers) = options.hooks[1].clone();
        assert_eq!(event, "PostToolUse");
        assert_eq!(matchers[0].matcher.as_deref(), Some("mcp__resolve__.*"));
        let hook = matchers[0].hooks[0].clone();
        let run = |tool_response: Value| {
            hook(
                json!({ "hook_event_name": "PostToolUse", "tool_name": "mcp__resolve__run_script", "tool_input": {}, "tool_response": tool_response, "tool_use_id": "t", "session_id": "s", "transcript_path": "", "cwd": "" }),
                Some("t".into()),
            )
        };
        assert_eq!(
            run(json!([{ "type": "text", "text": "{}" }])).await,
            json!({})
        );
        let items: Vec<Value> = (0..2000)
            .map(|i| json!({ "name": format!("clip {i}") }))
            .collect();
        let long = json!({ "items": items }).to_string();
        let output = run(json!([{ "type": "text", "text": long }])).await;
        assert!(
            output["hookSpecificOutput"]["updatedToolOutput"][0]["text"]
                .as_str()
                .unwrap()
                .contains("1950 more items left out")
        );
    }

    #[tokio::test]
    async fn leaves_resolves_own_mcp_server_out_when_this_computer_has_none() {
        let (calls, run_query) = scripted(vec![vec![]]);
        let mut config = test_config();
        config.resolve_mcp.command = None;
        send(
            &session_with(config, run_query, ApprovalMode::Ask, None),
            "go",
            &collecting_sink(),
        )
        .await;
        let keys: Vec<String> = calls.lock().unwrap()[0]
            .1
            .mcp_servers
            .keys()
            .cloned()
            .collect();
        assert_eq!(keys, ["linker"]);
    }

    #[tokio::test]
    async fn reports_a_resolve_mcp_that_failed_to_start_and_a_turn_that_ended_badly() {
        let (_, run_query) = scripted(vec![vec![
            json!({ "type": "system", "subtype": "init", "session_id": "s", "mcp_servers": [{ "name": "resolve", "status": "failed" }, { "name": "other", "status": "failed" }] }),
            json!({ "type": "result", "subtype": "error_max_turns", "session_id": "s" }),
            json!({ "type": "result", "subtype": "success", "is_error": true, "result": "Prompt is too long", "session_id": "s" }),
        ]]);
        let sink = collecting_sink();
        send(&session(run_query, ApprovalMode::Ask, None), "check", &sink).await;
        assert_eq!(
            sink.notice_texts(),
            [
                "MCP server \"resolve\" is failed.",
                "Claude stopped (error_max_turns).",
                CONTEXT_OVERFLOW_NOTICE,
            ]
        );
    }

    #[tokio::test]
    async fn passes_a_failed_run_on_as_the_turns_error() {
        let run_query = |_: String, _: QueryOptions| -> Messages {
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            tx.try_send(Err(anyhow::anyhow!(
                "Claude Code process exited with code 1"
            )))
            .unwrap();
            rx
        };
        let error = session(Arc::new(run_query), ApprovalMode::Ask, None)
            .send("go", collecting_sink(), CancellationToken::new())
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "Claude Code process exited with code 1");
    }
}
