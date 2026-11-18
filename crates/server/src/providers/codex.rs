// A Codex conversation: one `codex exec` run per turn (codex_cli.rs), continued by thread id.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use linker_edit_protocol::{ApprovalMode, Tone, ToolStatus};
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

use super::claude::ApprovalModeFn;
use super::codex_cli::{CodexCli, CodexExec, CodexOptions, CodexRun, ThreadOptions};
use super::types::{
    CODEX_DISABLED_RESOLVE_TOOLS, ItemKind, ItemPatch, LINKER_MCP_NAME, ProviderSession,
    RESOLVE_MCP_NAME, TRUNCATE_MAX, TurnSink, truncate,
};
use crate::agents::commands::linker_mcp_command;
use crate::config::AppConfig;

fn mcp_result_text(item: &Value) -> Option<String> {
    let error = &item["error"];
    if !error.is_null() {
        return Some(error["message"].as_str().unwrap_or("").to_string());
    }
    let result = &item["result"];
    if result.is_null() {
        return None;
    }
    Some(
        result["content"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|part| match part["type"].as_str() {
                Some("text") => part["text"].as_str().unwrap_or("").to_string(),
                kind => format!("[{}]", kind.unwrap_or("undefined")),
            })
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

fn status(item: &Value) -> ToolStatus {
    match item["status"].as_str() {
        Some("in_progress") => ToolStatus::Running,
        Some("failed") => ToolStatus::Error,
        _ => ToolStatus::Done,
    }
}

fn tool(id: String, name: &str, input: Value, status: ToolStatus) -> ItemPatch {
    ItemPatch {
        id,
        kind: Some(ItemKind::Tool),
        name: Some(name.to_string()),
        input: Some(input),
        status: Some(status),
        ..ItemPatch::default()
    }
}

fn handle_item(item: &Value, sink: &dyn TurnSink, turn: &str) {
    // Codex item ids restart at item_0 every turn, and a session's own count restarts with the server, so a
    // turn gets an id that no earlier turn in the chat has.
    let id = format!("{turn}:{}", item["id"].as_str().unwrap_or("undefined"));
    match item["type"].as_str() {
        Some("agent_message") => {
            let text = item["text"].as_str().unwrap_or("");
            if !text.trim().is_empty() {
                sink.upsert(ItemPatch {
                    id,
                    kind: Some(ItemKind::Assistant),
                    text: Some(text.to_string()),
                    ..ItemPatch::default()
                });
            }
        }
        Some("command_execution") => sink.upsert(ItemPatch {
            output: Some(truncate(
                item["aggregated_output"].as_str().unwrap_or(""),
                TRUNCATE_MAX,
            )),
            ..tool(
                id,
                "shell",
                json!({ "command": item["command"] }),
                status(item),
            )
        }),
        Some("mcp_tool_call") => {
            let name = format!(
                "mcp__{}__{}",
                item["server"].as_str().unwrap_or(""),
                item["tool"].as_str().unwrap_or("")
            );
            sink.upsert(ItemPatch {
                output: mcp_result_text(item).map(|output| truncate(&output, TRUNCATE_MAX)),
                ..tool(id, &name, item["arguments"].clone(), status(item))
            });
        }
        Some("file_change") => {
            let failed = item["status"] == "failed";
            sink.upsert(tool(
                id,
                "edit files",
                item["changes"].clone(),
                if failed {
                    ToolStatus::Error
                } else {
                    ToolStatus::Done
                },
            ));
        }
        Some("web_search") => sink.upsert(tool(
            id,
            "web search",
            json!({ "query": item["query"] }),
            ToolStatus::Done,
        )),
        Some("error") => sink.notice(
            item["message"].as_str().unwrap_or("").to_string(),
            Tone::Error,
        ),
        _ => {}
    }
}

// Codex's --config parsing hangs on backslash paths; Windows accepts forward slashes.
fn slashes(value: &str) -> String {
    value.replace('\\', "/")
}

pub fn codex_options(config: &AppConfig) -> CodexOptions {
    let linker = linker_mcp_command(config);
    let mut servers = Map::new();
    // Resolve's own server, for its scripting docs and launch_resolve only: Codex has no per-call hook to
    // check a script or back the project up first, so run_script and every tool that writes are off.
    if let Some(resolve) = &config.resolve_mcp.command {
        servers.insert(
            RESOLVE_MCP_NAME.into(),
            json!({
                "command": slashes(&resolve.display().to_string()),
                "args": [],
                "default_tools_approval_mode": "approve",
                "disabled_tools": CODEX_DISABLED_RESOLVE_TOOLS,
            }),
        );
    }
    // Linker-Edit's tools: the Linker-Edit server asks the user before changes, for Codex too.
    servers.insert(
        LINKER_MCP_NAME.into(),
        json!({
            "command": slashes(&linker.command),
            "args": linker.args.iter().map(|arg| slashes(arg)).collect::<Vec<_>>(),
            "env": linker.env,
            "default_tools_approval_mode": "approve",
            // A first transcription downloads the engine, and an approval waits for the user; Codex gives
            // up on a tool after 60 seconds by default.
            "tool_timeout_sec": 3600,
        }),
    );
    CodexOptions {
        config: json!({ "mcp_servers": servers }),
    }
}

/// Codex never asks here; Resolve changes wait for the user in the Linker-Edit server in supervised mode.
/// Like T3 Code, full access runs without a sandbox; supervised keeps writes in the project's folder.
pub fn codex_thread_options(config: &AppConfig, mode: ApprovalMode) -> ThreadOptions {
    ThreadOptions {
        working_directory: Some(config.workspace_dir.clone()),
        skip_git_repo_check: true,
        sandbox_mode: Some(
            if mode == ApprovalMode::Auto {
                "danger-full-access"
            } else {
                "workspace-write"
            }
            .into(),
        ),
        approval_policy: Some("never".into()),
        ..ThreadOptions::default()
    }
}

/// The SDK's Thread: its id once Codex has said it, and the options it runs with.
struct Thread {
    id: Option<String>,
    options: ThreadOptions,
}

struct State {
    thread: Option<Thread>,
    // The sandbox is set per thread, so a thread is picked up again with the other one when the mode changes.
    thread_mode: ApprovalMode,
}

// No context meter or compaction: Codex reports tokens summed over a turn, not the context size, and `exec`
// runs no slash commands.
pub struct CodexSession {
    config: AppConfig,
    approval_mode: ApprovalModeFn,
    exec: Arc<dyn CodexExec>,
    state: Mutex<State>,
}

/// A session on the bundled codex, or the one on PATH.
pub fn create_codex_session(
    config: AppConfig,
    approval_mode: ApprovalModeFn,
    resume_id: Option<String>,
) -> CodexSession {
    let exec = Arc::new(CodexCli::new(codex_options(&config)));
    create_codex_session_with(config, approval_mode, resume_id, exec)
}

/// A session on another CodexExec, as tests use.
pub fn create_codex_session_with(
    config: AppConfig,
    approval_mode: ApprovalModeFn,
    resume_id: Option<String>,
    exec: Arc<dyn CodexExec>,
) -> CodexSession {
    let thread_mode = approval_mode();
    let thread = resume_id.map(|id| Thread {
        id: Some(id),
        options: codex_thread_options(&config, thread_mode),
    });
    CodexSession {
        config,
        approval_mode,
        exec,
        state: Mutex::new(State {
            thread,
            thread_mode,
        }),
    }
}

#[async_trait]
impl ProviderSession for CodexSession {
    async fn send(
        &self,
        text: &str,
        sink: Arc<dyn TurnSink>,
        signal: CancellationToken,
    ) -> anyhow::Result<()> {
        let (thread_id, options) = {
            let mut state = self.state.lock().unwrap();
            let mode = (self.approval_mode)();
            let id = state.thread.as_ref().and_then(|thread| thread.id.clone());
            if id.is_some() && mode != state.thread_mode {
                state.thread = Some(Thread {
                    id,
                    options: codex_thread_options(&self.config, mode),
                });
            }
            state.thread_mode = mode;
            let thread = state.thread.get_or_insert_with(|| Thread {
                id: None,
                options: codex_thread_options(&self.config, mode),
            });
            (thread.id.clone(), thread.options.clone())
        };
        let turn = uuid::Uuid::new_v4().to_string();
        let mut events = self.exec.run(CodexRun {
            input: text.to_string(),
            thread_id,
            options,
            signal,
        });
        while let Some(event) = events.recv().await {
            let event = event?;
            match event["type"].as_str() {
                // The SDK's Thread took its id from here.
                Some("thread.started") => {
                    if let (Some(thread), Some(id)) = (
                        self.state.lock().unwrap().thread.as_mut(),
                        event["thread_id"].as_str(),
                    ) {
                        thread.id = Some(id.to_string());
                    }
                }
                Some("item.started" | "item.updated" | "item.completed") => {
                    handle_item(&event["item"], &*sink, &turn)
                }
                Some("turn.failed") => sink.notice(
                    event["error"]["message"].as_str().unwrap_or("").to_string(),
                    Tone::Error,
                ),
                Some("error") => sink.notice(
                    event["message"].as_str().unwrap_or("").to_string(),
                    Tone::Error,
                ),
                _ => {}
            }
        }
        Ok(())
    }

    fn resume_id(&self) -> Option<String> {
        self.state
            .lock()
            .unwrap()
            .thread
            .as_ref()
            .and_then(|thread| thread.id.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::claude_cli::{Messages, messages};
    use crate::providers::replay::read_fixture;
    use crate::providers::testing::{collecting_sink, fixtures, test_config};
    use std::collections::VecDeque;

    /// Stands in for codex: each run plays the next turn's events, after the thread.started a new thread gets.
    #[derive(Default)]
    struct FakeExec {
        turns: Mutex<VecDeque<Vec<Value>>>,
        runs: Mutex<Vec<(Option<String>, String)>>,
    }

    impl FakeExec {
        fn new(turns: Vec<Vec<Value>>) -> Arc<Self> {
            Arc::new(FakeExec {
                turns: Mutex::new(turns.into()),
                runs: Mutex::default(),
            })
        }

        fn runs(&self) -> Vec<(Option<String>, String)> {
            self.runs.lock().unwrap().clone()
        }
    }

    impl CodexExec for FakeExec {
        fn run(&self, run: CodexRun) -> Messages {
            self.runs.lock().unwrap().push((
                run.thread_id.clone(),
                run.options.sandbox_mode.clone().unwrap_or_default(),
            ));
            let mut events = self.turns.lock().unwrap().pop_front().unwrap_or_default();
            if run.thread_id.is_none() {
                events.insert(
                    0,
                    json!({ "type": "thread.started", "thread_id": "thread-new" }),
                );
            }
            messages(events)
        }
    }

    fn recorded() -> Vec<Value> {
        read_fixture(&fixtures().join("codex-turn.jsonl")).unwrap()
    }

    fn auto() -> ApprovalModeFn {
        Arc::new(|| ApprovalMode::Auto)
    }

    async fn send(session: &CodexSession, text: &str, sink: &Arc<impl TurnSink + 'static>) {
        session
            .send(text, sink.clone(), CancellationToken::new())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn turns_a_recorded_turn_into_chat_items_keeping_each_turns_items_apart() {
        let exec = FakeExec::new(vec![recorded(), recorded()]);
        let sink = collecting_sink();
        let codex = create_codex_session_with(test_config(), auto(), None, exec);
        send(&codex, "one", &sink).await;
        let first_turn = sink.items();
        let shown: Vec<(ItemKind, String)> = first_turn
            .iter()
            .map(|item| {
                let tool = match item.kind {
                    Some(ItemKind::Tool) => format!(
                        "{} {}",
                        item.name.clone().unwrap(),
                        serde_json::to_value(item.status).unwrap().as_str().unwrap()
                    ),
                    _ => String::new(),
                };
                (item.kind.unwrap(), tool)
            })
            .collect();
        let assistant = (ItemKind::Assistant, String::new());
        let tool = |text: &str| (ItemKind::Tool, text.to_string());
        assert_eq!(
            shown,
            [
                assistant.clone(),
                tool("mcp__resolve__project_settings done"),
                assistant.clone(),
                tool("edit files done"),
                tool("shell done"),
                assistant,
            ]
        );
        assert_eq!(
            first_turn[1].input,
            Some(json!({ "action": "project_summary", "params": { "include_clips": false } }))
        );
        assert!(
            first_turn[1]
                .output
                .as_deref()
                .unwrap()
                .contains("Untitled Project")
        );

        send(&codex, "two", &sink).await;
        assert_eq!(sink.items().len(), first_turn.len() * 2);
    }

    #[tokio::test]
    async fn never_reuses_an_earlier_turns_item_ids_in_a_session_started_after_a_restart() {
        let exec = FakeExec::new(vec![recorded(), recorded()]);
        let sink = collecting_sink();
        send(
            &create_codex_session_with(test_config(), auto(), None, exec.clone()),
            "one",
            &sink,
        )
        .await;
        let before = sink.items().len();
        send(
            &create_codex_session_with(test_config(), auto(), Some("thread-new".into()), exec),
            "two",
            &sink,
        )
        .await;
        assert_eq!(sink.items().len(), before * 2);
    }

    #[tokio::test]
    async fn continues_a_thread_from_before_a_restart() {
        let exec = FakeExec::new(vec![vec![]]);
        let codex =
            create_codex_session_with(test_config(), auto(), Some("thread-1".into()), exec.clone());
        assert_eq!(codex.resume_id().as_deref(), Some("thread-1"));
        send(&codex, "go on", &collecting_sink()).await;
        assert_eq!(
            exec.runs(),
            [(Some("thread-1".into()), "danger-full-access".into())]
        );
    }

    #[tokio::test]
    async fn runs_without_a_sandbox_with_full_access_and_sandboxed_when_supervised_like_t3_code() {
        let exec = FakeExec::new(vec![vec![], vec![], vec![]]);
        let mode = Arc::new(Mutex::new(ApprovalMode::Auto));
        let current = mode.clone();
        let codex = create_codex_session_with(
            test_config(),
            Arc::new(move || *current.lock().unwrap()),
            None,
            exec.clone(),
        );
        send(&codex, "one", &collecting_sink()).await;
        *mode.lock().unwrap() = ApprovalMode::Ask;
        send(&codex, "two", &collecting_sink()).await;
        send(&codex, "three", &collecting_sink()).await;
        let new = Some("thread-new".to_string());
        assert_eq!(
            exec.runs(),
            [
                (None, "danger-full-access".into()),
                (new.clone(), "workspace-write".into()),
                (new, "workspace-write".into()),
            ]
        );
    }

    #[tokio::test]
    async fn shows_failed_turns_and_stream_errors() {
        let exec = FakeExec::new(vec![vec![
            json!({ "type": "turn.failed", "error": { "message": "usage limit reached" } }),
            json!({ "type": "error", "message": "stream closed" }),
        ]]);
        let sink = collecting_sink();
        send(
            &create_codex_session_with(test_config(), auto(), None, exec),
            "go",
            &sink,
        )
        .await;
        assert_eq!(
            sink.notice_texts(),
            ["usage limit reached", "stream closed"]
        );
    }

    #[test]
    fn gives_codex_forward_slash_paths_which_its_config_parsing_needs_and_only_resolves_read_tools()
    {
        let options = codex_options(&test_config());
        let text = options.config.to_string();
        assert!(!text.contains("\\\\"));
        assert!(!text.contains("launch_resolve"));
        let resolve = &options.config["mcp_servers"]["resolve"];
        assert_eq!(
            resolve["command"],
            "C:/Program Files/Blackmagic Design/DaVinci Resolve/ResolveMCP.exe"
        );
        let disabled = resolve["disabled_tools"].as_array().unwrap();
        for tool in ["run_script", "run_script_unsafe", "delete_lut"] {
            assert!(disabled.contains(&json!(tool)));
        }
        assert_eq!(
            options.config["mcp_servers"]["linker"]["env"]["LINKER_EDIT_TOKEN"],
            "test-token"
        );
        assert_eq!(
            options.config["mcp_servers"]["linker"]["args"],
            json!(["mcp"])
        );
    }

    #[test]
    fn leaves_resolves_own_mcp_server_out_when_this_computer_has_none() {
        let mut config = test_config();
        config.resolve_mcp.command = None;
        let options = codex_options(&config);
        let keys: Vec<&String> = options.config["mcp_servers"]
            .as_object()
            .unwrap()
            .keys()
            .collect();
        assert_eq!(keys, ["linker"]);
    }
}
