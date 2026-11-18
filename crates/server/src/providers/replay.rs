// LINKER_EDIT_REPLAY: stands in for Claude Code by replaying a recorded turn, for the e2e tests. A fixture is
// one SDK message per JSONL line: what Claude Code writes on stdout, so a turn recorded through the TypeScript
// SDK (scripts/record-fixture.ts) replays here unchanged.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use anyhow::{Context as _, anyhow};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::claude_cli::{Messages, PermissionResult, QueryOptions, RunQuery};

/// Slow enough to watch a turn stream in the UI, fast enough for the e2e test.
pub const STEP_MS: u64 = 150;

pub fn read_fixture(file: &Path) -> anyhow::Result<Vec<Value>> {
    let text = std::fs::read_to_string(file)
        .with_context(|| format!("Can't read the fixture {}", file.display()))?;
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).context("A fixture line isn't JSON"))
        .collect()
}

/// Replays a recorded Claude turn and asks canUseTool for each tool call, like the real Claude Code. A
/// denied call replays as an error.
pub struct ReplayQuery {
    file: PathBuf,
    step: Duration,
    run: AtomicUsize,
}

pub fn replay_query(fixture_file: impl Into<PathBuf>, step_ms: u64) -> ReplayQuery {
    ReplayQuery {
        file: fixture_file.into(),
        step: Duration::from_millis(step_ms),
        run: AtomicUsize::new(0),
    }
}

impl RunQuery for ReplayQuery {
    fn query(&self, _prompt: String, options: QueryOptions) -> Messages {
        let run = self.run.fetch_add(1, Ordering::SeqCst) + 1;
        let (tx, rx) = mpsc::channel(16);
        let (file, step) = (self.file.clone(), self.step);
        tokio::spawn(async move {
            if let Err(error) = replay(&file, options, step, run, &tx).await {
                let _ = tx.send(Err(error)).await;
            }
        });
        rx
    }
}

async fn abortable<T>(
    future: impl Future<Output = T>,
    signal: &CancellationToken,
) -> anyhow::Result<T> {
    tokio::select! {
        _ = signal.cancelled() => Err(anyhow!("Replay aborted")),
        value = future => Ok(value),
    }
}

async fn replay(
    file: &Path,
    options: QueryOptions,
    step: Duration,
    run: usize,
    tx: &mpsc::Sender<anyhow::Result<Value>>,
) -> anyhow::Result<()> {
    let messages = read_fixture(file)?;
    let signal = options.abort.clone();
    let mut denied: HashMap<String, String> = HashMap::new();
    // Recorded ids repeat on every replay, and the app merges chat items by id.
    let rid = |id: &Value| format!("{}~{run}", id.as_str().unwrap_or("undefined"));

    for mut message in messages {
        abortable(tokio::time::sleep(step), &signal).await?;
        let kind = message.get("type").and_then(Value::as_str).unwrap_or("");
        if kind == "assistant" {
            if let Some(content) = message["message"]["content"].as_array_mut() {
                for block in content
                    .iter_mut()
                    .filter(|block| block["type"] == "tool_use")
                {
                    block["id"] = json!(rid(&block["id"]));
                }
            }
            message["uuid"] = json!(rid(&message["uuid"]));
            let calls: Vec<(String, Value, String)> = message["message"]["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|block| block["type"] == "tool_use")
                .map(|block| {
                    (
                        block["name"].as_str().unwrap_or("").to_string(),
                        block["input"].clone(),
                        block["id"].as_str().unwrap_or("").to_string(),
                    )
                })
                .collect();
            if tx.send(Ok(message)).await.is_err() {
                return Ok(());
            }
            let Some(can_use_tool) = &options.can_use_tool else {
                continue;
            };
            for (name, input, id) in calls {
                let decision = abortable(can_use_tool(name, input), &signal).await?;
                if let PermissionResult::Deny { message } = decision {
                    denied.insert(id, message);
                }
            }
            continue;
        }
        if kind == "user"
            && let Some(content) = message["message"]["content"].as_array_mut()
        {
            for block in content
                .iter_mut()
                .filter(|block| block["type"] == "tool_result")
            {
                let id = rid(&block["tool_use_id"]);
                if let Some(denial) = denied.get(&id) {
                    block["is_error"] = json!(true);
                    block["content"] = json!(denial);
                }
                block["tool_use_id"] = json!(id);
            }
        }
        if tx.send(Ok(message)).await.is_err() {
            return Ok(());
        }
    }
    Ok(())
}
