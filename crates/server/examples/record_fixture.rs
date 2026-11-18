//! Records one real agent turn as JSONL: raw stream-json messages (Claude) or thread events (Codex).
//! The fixtures drive the provider tests, and a Claude one can be replayed with LINKER_EDIT_REPLAY.
//!
//!   cargo run -p linker-edit-server --example record_fixture -- <claude|codex> <out.jsonl> "<prompt>"
//!
//! Runs on the cheapest models, in a throwaway workspace. Every tool but Bash is allowed, so record
//! against a scratch project or with Resolve closed.
//!
//! The agents start the "linker" MCP server as this same program with `mcp` (agents::commands), so that
//! argument serves it on stdio, as `linker-edit mcp` does.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use linker_edit_protocol::{ApprovalMode, ContextUsage, Tone};
use linker_edit_server::agents::linker_mcp;
use linker_edit_server::config::{AppConfig, load_config_or_exit};
use linker_edit_server::js;
use linker_edit_server::providers::claude::{ClaudeSessionOptions, create_claude_session};
use linker_edit_server::providers::claude_cli::{ClaudeCli, Messages, QueryOptions, RunQuery};
use linker_edit_server::providers::codex::{codex_options, codex_thread_options};
use linker_edit_server::providers::codex_cli::{CodexCli, CodexExec, CodexRun};
use linker_edit_server::providers::types::{ItemPatch, Policy, ProviderSession, TurnSink};
use linker_edit_server::resolve::locate::find_resolve_mcp;
use linker_edit_server::rules::{EDITING_RULES, RESOLVE_MCP_NAME, prepare_workspace};
use serde::Serialize;
use serde_json::{Map, Value};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

const USAGE: &str = "Usage: cargo run -p linker-edit-server --example record_fixture -- <claude|codex> <out.jsonl> \"<prompt>\"";

const KEEP: [&str; 5] = ["system", "assistant", "user", "tool_progress", "result"];

/// os.userInfo().username.
fn username() -> String {
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_default()
}

/// The username in a recorded line, as "you".
fn anonymize(line: String, username: &str) -> String {
    if username.is_empty() {
        line
    } else {
        line.replace(username, "you")
    }
}

// Fixtures get committed: keep only what the app reads, and drop account and machine details.
fn sanitize(message: &Value, username: &str) -> Option<String> {
    let kind = message["type"].as_str()?;
    if !KEEP.contains(&kind) {
        return None;
    }
    let mut message = message.clone();
    if kind == "system" {
        let subtype = message["subtype"].as_str();
        if subtype == Some("compact_boundary") {
            return Some(js::stringify(&message));
        }
        if subtype != Some("init") {
            return None;
        }
        let mut kept = Map::new();
        for key in ["type", "subtype", "session_id", "uuid", "model"] {
            if let Some(value) = message.get(key) {
                kept.insert(key.into(), value.clone());
            }
        }
        let resolve: Vec<Value> = message["mcp_servers"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|server| server["name"] == RESOLVE_MCP_NAME)
            .cloned()
            .collect();
        kept.insert("mcp_servers".into(), Value::Array(resolve));
        message = Value::Object(kept);
    }
    Some(anonymize(js::stringify(&message), username))
}

/// A value's name as it is sent, such as "tool" or "running".
fn label(value: &impl Serialize) -> String {
    match serde_json::to_value(value) {
        Ok(Value::String(text)) => text,
        _ => String::new(),
    }
}

struct Sink;

#[async_trait]
impl TurnSink for Sink {
    fn upsert(&self, patch: ItemPatch) {
        let kind = patch
            .kind
            .map_or_else(|| "update".to_string(), |kind| label(&kind));
        let detail = patch
            .name
            .or_else(|| patch.status.map(|status| label(&status)))
            .or_else(|| patch.text.map(|text| js::slice(&text, 80).to_string()))
            .unwrap_or_default();
        println!("{kind} {detail}");
    }

    fn notice(&self, text: String, _tone: Tone) {
        println!("notice {text}");
    }

    async fn request_approval(&self, tool: &str, _input: &Value) -> bool {
        let allow = tool != "Bash";
        println!("{} {tool}", if allow { "allow" } else { "deny" });
        allow
    }

    fn context(&self, usage: Option<ContextUsage>) {
        let used = usage.map_or_else(|| "unknown".into(), |usage| usage.used_tokens.to_string());
        println!("context {used}");
    }
}

/// Claude Code on haiku, keeping each message it sends as a line.
fn recording(lines: Arc<Mutex<Vec<String>>>, username: String) -> Arc<dyn RunQuery> {
    let claude = ClaudeCli::new();
    Arc::new(
        move |prompt: String, mut options: QueryOptions| -> Messages {
            options.model = Some("haiku".into());
            let mut messages = claude.query(prompt, options);
            let (tx, rx) = mpsc::channel(64);
            let (lines, username) = (lines.clone(), username.clone());
            tokio::spawn(async move {
                while let Some(message) = messages.recv().await {
                    if let Ok(message) = &message
                        && let Some(kept) = sanitize(message, &username)
                    {
                        lines.lock().unwrap().push(kept);
                    }
                    if tx.send(message).await.is_err() {
                        break;
                    }
                }
            });
            rx
        },
    )
}

async fn record(provider: &str, prompt: &str, config: AppConfig) -> anyhow::Result<Vec<String>> {
    let username = username();
    let lines = Arc::new(Mutex::new(Vec::new()));
    if provider == "claude" {
        let mut options = ClaudeSessionOptions::new(|| EDITING_RULES.to_string());
        options.run_query = recording(lines.clone(), username);
        let session = create_claude_session(
            config,
            Arc::new(|| ApprovalMode::Ask),
            Arc::new(Policy),
            options,
        );
        session
            .send(prompt, Arc::new(Sink), CancellationToken::new())
            .await?;
    } else {
        let mut options = codex_thread_options(&config, ApprovalMode::Auto);
        options.model = Some("gpt-5.6-luna".into());
        options.model_reasoning_effort = Some("low".into());
        let mut events = CodexCli::new(codex_options(&config)).run(CodexRun {
            input: prompt.to_string(),
            thread_id: None,
            options,
            signal: CancellationToken::new(),
        });
        while let Some(event) = events.recv().await {
            let event = event?;
            lines
                .lock()
                .unwrap()
                .push(anonymize(js::stringify(&event), &username));
            let item = event["item"]["type"].as_str().unwrap_or("");
            println!("{} {item}", event["type"].as_str().unwrap_or("undefined"));
        }
    }
    let lines = std::mem::take(&mut *lines.lock().unwrap());
    Ok(lines)
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("mcp") {
        return linker_mcp::run_linker_mcp(linker_mcp::catalog());
    }
    let (provider, out, prompt) = match args.as_slice() {
        [provider, out, prompt, ..]
            if (provider == "claude" || provider == "codex")
                && !out.is_empty()
                && !prompt.is_empty() =>
        {
            (provider.clone(), out.clone(), prompt.clone())
        }
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(1);
        }
    };

    let workspace_dir = tempfile::Builder::new()
        .prefix("linker-edit-record-")
        .tempdir()?
        .keep();
    prepare_workspace(&workspace_dir, EDITING_RULES)?;
    let mut config = AppConfig {
        workspace_dir,
        ..load_config_or_exit(find_resolve_mcp)
    };
    // An empty resolveMcp means this computer has none, as in index.rs.
    config.resolve_mcp.command = config
        .resolve_mcp
        .command
        .take()
        .filter(|command| !command.as_os_str().is_empty());

    let lines = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(record(&provider, &prompt, config))?;
    std::fs::write(&out, format!("{}\n", lines.join("\n")))?;
    println!("Wrote {} messages to {out}", lines.len());
    Ok(())
}
