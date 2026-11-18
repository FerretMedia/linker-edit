// What the provider tests share: a config, a sink that collects a turn, and a stand-in for policy.rs.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use linker_edit_protocol::{ApprovalMode, ContextUsage, Tone};
use serde_json::Value;

use super::types::{BoxFuture, ItemKind, ItemPatch, ToolDecision, ToolGate, TurnSink};
use crate::config::{AppConfig, ConnectConfig, LinkerConfig, ResolveMcpConfig};

/// The recorded turns: apps/desktop/server/providers/fixtures.
pub fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../apps/desktop/server/providers/fixtures")
}

pub fn test_config() -> AppConfig {
    AppConfig {
        port: 0,
        projects_dir: r"C:\projects".into(),
        workspace_dir: r"C:\work".into(),
        preview_dir: r"C:\previews".into(),
        transcripts_dir: r"C:\transcripts".into(),
        tools_dir: r"C:\tools".into(),
        whisper_cli: None,
        tailscale_port: 8443,
        config_file: r"C:\linker-edit.config.json".into(),
        settings_file: r"C:\settings.json".into(),
        library_file: r"C:\library.json".into(),
        library_cache_dir: r"C:\cache\library".into(),
        linker: LinkerConfig {
            auth_server: "https://clerk.example".into(),
            app_url: "https://app.example".into(),
            client_id: None,
            file: r"C:\linker-account.json".into(),
        },
        connect: ConnectConfig {
            file: r"C:\connect.json".into(),
            sessions_file: r"C:\connect-sessions.json".into(),
            cloudflared: None,
        },
        api_token: "test-token".into(),
        resolve_mcp: ResolveMcpConfig {
            command: Some(
                r"C:\Program Files\Blackmagic Design\DaVinci Resolve\ResolveMCP.exe".into(),
            ),
        },
        auto_compact_window: None,
    }
}

type Answer = Box<dyn Fn(&str, &Value) -> bool + Send + Sync>;
type Backup = Box<dyn Fn() -> Result<(), String> + Send + Sync>;

/// Collects a turn's chat items the way the app merges them, and answers approvals with `answer`.
pub struct CollectingSink {
    answer: Answer,
    backup: Option<Backup>,
    items: Mutex<Vec<ItemPatch>>,
    approvals: Mutex<Vec<(String, Value)>>,
    contexts: Mutex<Vec<Option<ContextUsage>>>,
    notices: AtomicUsize,
}

/// A sink that allows every call.
pub fn collecting_sink() -> Arc<CollectingSink> {
    collecting_sink_answering(|_, _| true)
}

pub fn collecting_sink_answering(
    answer: impl Fn(&str, &Value) -> bool + Send + Sync + 'static,
) -> Arc<CollectingSink> {
    Arc::new(CollectingSink::new(Box::new(answer), None))
}

/// A sink with a backup: each call runs `backup`.
pub fn collecting_sink_with_backup(
    backup: impl Fn() -> Result<(), String> + Send + Sync + 'static,
) -> Arc<CollectingSink> {
    Arc::new(CollectingSink::new(
        Box::new(|_, _| true),
        Some(Box::new(backup)),
    ))
}

impl CollectingSink {
    fn new(answer: Answer, backup: Option<Backup>) -> Self {
        CollectingSink {
            answer,
            backup,
            items: Mutex::default(),
            approvals: Mutex::default(),
            contexts: Mutex::default(),
            notices: AtomicUsize::new(0),
        }
    }

    pub fn items(&self) -> Vec<ItemPatch> {
        self.items.lock().unwrap().clone()
    }

    pub fn approvals(&self) -> Vec<(String, Value)> {
        self.approvals.lock().unwrap().clone()
    }

    pub fn contexts(&self) -> Vec<Option<ContextUsage>> {
        self.contexts.lock().unwrap().clone()
    }

    /// The text of each item, and "" for those that aren't notices.
    pub fn notice_texts(&self) -> Vec<String> {
        self.items()
            .into_iter()
            .map(|item| match item.kind {
                Some(ItemKind::Notice) => item.text.unwrap_or_default(),
                _ => String::new(),
            })
            .collect()
    }
}

#[async_trait]
impl TurnSink for CollectingSink {
    fn upsert(&self, patch: ItemPatch) {
        let mut items = self.items.lock().unwrap();
        match items.iter_mut().find(|item| item.id == patch.id) {
            Some(item) => *item = patch.merged_over(item.clone()),
            None => items.push(patch),
        }
    }

    fn notice(&self, text: String, tone: Tone) {
        let id = format!("notice-{}", self.notices.fetch_add(1, Ordering::SeqCst));
        self.upsert(ItemPatch {
            id,
            kind: Some(ItemKind::Notice),
            text: Some(text),
            tone: Some(tone),
            ..ItemPatch::default()
        });
    }

    async fn request_approval(&self, tool: &str, input: &Value) -> bool {
        self.approvals
            .lock()
            .unwrap()
            .push((tool.to_string(), input.clone()));
        (self.answer)(tool, input)
    }

    fn context(&self, usage: Option<ContextUsage>) {
        self.contexts.lock().unwrap().push(usage);
    }

    async fn backup(&self) -> Result<(), String> {
        match &self.backup {
            Some(backup) => backup(),
            None => Ok(()),
        }
    }
}

/// Stands in for policy.rs's refusal and decideToolCall, which the Resolve port owns: the same rules, with the
/// forbidden-script check cut down to plain names. The hub can hand the real policy to these tests instead.
pub struct TestGate;

const READ_ONLY_TOOLS: &[&str] = &[
    "Read",
    "Glob",
    "Grep",
    "LS",
    "WebSearch",
    "WebFetch",
    "TodoWrite",
    "ToolSearch",
    "ListMcpResourcesTool",
    "ReadMcpResourceTool",
];
const RESOLVE_READS: &[&str] = &[
    "get_resolve_status",
    "get_whats_new",
    "get_scripting_api",
    "search_scripting_api",
    "get_scripting_docs",
    "list_luts",
    "list_dctls",
];
const FORBIDDEN_NAMES: &[&str] = &[
    "LoadProject",
    "CloseProject",
    "CreateProject",
    "DeleteProject",
    "DeleteTimelines",
    "DeleteClips",
    "Quit",
    "getattr(",
    "eval(",
    "exec(",
];
const FORBIDDEN_SCRIPT: &str = "Linker-Edit doesn't let scripts open, close, create or switch Resolve projects or databases, delete media, bins, timelines, render jobs or render presets, or replace a clip's media, and scripts may not name those calls or use getattr, eval or exec. Use the linker tools (remove_clips takes clips off an AI timeline), or ask the user to switch projects in Linker-Edit's sidebar.";

fn needs_approval(tool_name: &str) -> bool {
    if READ_ONLY_TOOLS.contains(&tool_name) || tool_name.starts_with("mcp__linker__") {
        return false;
    }
    match tool_name.strip_prefix("mcp__resolve__") {
        Some(tool) => !RESOLVE_READS.contains(&tool),
        None => true,
    }
}

#[async_trait]
impl ToolGate for TestGate {
    fn refusal(&self, tool_name: &str, input: &Value) -> Option<String> {
        let tool = tool_name.strip_prefix("mcp__resolve__")?;
        let allowed = RESOLVE_READS.contains(&tool)
            || [
                "update_dctl",
                "generate_lut",
                "run_script",
                "launch_resolve",
            ]
            .contains(&tool);
        if !allowed {
            return Some(format!("Linker-Edit doesn't let agents use {tool}."));
        }
        if tool == "run_script" {
            let Some(script) = input.get("script").and_then(Value::as_str) else {
                return Some("run_script needs the script as text.".into());
            };
            if FORBIDDEN_NAMES.iter().any(|name| script.contains(name)) {
                return Some(FORBIDDEN_SCRIPT.into());
            }
        }
        None
    }

    async fn decide_tool_call(
        &self,
        tool_name: &str,
        input: &Value,
        mode: ApprovalMode,
        ask: BoxFuture<'_, bool>,
    ) -> ToolDecision {
        if let Some(message) = self.refusal(tool_name, input) {
            return ToolDecision::Deny { message };
        }
        if mode == ApprovalMode::Ask && needs_approval(tool_name) && !ask.await {
            return ToolDecision::Deny {
                message: "The user declined this action.".into(),
            };
        }
        ToolDecision::Allow
    }
}

pub fn test_gate() -> Arc<dyn ToolGate> {
    Arc::new(TestGate)
}

/// examples/fake_cli, which `cargo test` builds next to the test binaries' profile folder.
pub fn fake_cli() -> PathBuf {
    let file = format!("fake_cli{}", std::env::consts::EXE_SUFFIX);
    let exe = std::env::current_exe().expect("the test binary has a path");
    exe.ancestors()
        .map(|dir| dir.join("examples").join(&file))
        .find(|candidate| candidate.is_file())
        .expect("examples/fake_cli is built: run the tests with cargo test, which builds it")
}

/// A folder where fake_cli finds its steps and leaves its log.
pub struct FakeCliRun {
    pub dir: tempfile::TempDir,
}

impl FakeCliRun {
    pub fn new(steps: Value) -> Self {
        let dir = tempfile::Builder::new()
            .prefix("linker-edit-fake-cli-")
            .tempdir()
            .expect("a temp folder");
        std::fs::write(
            dir.path().join("fake-cli.json"),
            serde_json::json!({ "steps": steps }).to_string(),
        )
        .expect("the script is written");
        FakeCliRun { dir }
    }

    /// What fake_cli logged: {"argv", "env"} first, then {"stdin"} and {"eof"} entries.
    pub fn log(&self) -> Vec<Value> {
        std::fs::read_to_string(self.dir.path().join("fake-cli.log"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).expect("a log line is JSON"))
            .collect()
    }

    /// The JSON lines it read on stdin.
    pub fn stdin(&self) -> Vec<Value> {
        self.log()
            .iter()
            .filter_map(|entry| entry["stdin"].as_str())
            .map(|line| serde_json::from_str(line).unwrap_or(Value::String(line.into())))
            .collect()
    }
}
