// Our own stand-in for @openai/codex-sdk 0.166.0: one `codex exec --experimental-json` run per turn, the prompt
// on stdin, one JSON thread event per stdout line. The arguments, the --config flattening and the environment
// are the SDK's (CodexExec in its dist/index.js).

use std::path::PathBuf;

use anyhow::{Context as _, anyhow, bail};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::claude_cli::Messages;
use super::cli::{self, CliProgram, prepend_path_dirs};

/// The SDK's CodexOptions, as far as Linker-Edit uses them: `config`, passed as --config overrides.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CodexOptions {
    pub config: Value,
}

/// The SDK's ThreadOptions, as far as Linker-Edit uses them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ThreadOptions {
    pub model: Option<String>,
    pub sandbox_mode: Option<String>,
    pub working_directory: Option<PathBuf>,
    pub skip_git_repo_check: bool,
    pub model_reasoning_effort: Option<String>,
    pub approval_policy: Option<String>,
}

/// One turn: on a new thread (thread_id None) or a resumed one.
#[derive(Debug, Clone)]
pub struct CodexRun {
    pub input: String,
    pub thread_id: Option<String>,
    pub options: ThreadOptions,
    pub signal: CancellationToken,
}

/// Runs a Codex turn and streams its thread events; tests stand in for it.
pub trait CodexExec: Send + Sync {
    fn run(&self, run: CodexRun) -> Messages;
}

fn toml_bare_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn toml_value(value: &Value, path: &str) -> anyhow::Result<String> {
    Ok(match value {
        Value::String(text) => serde_json::to_string(text)?,
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Array(items) => {
            let rendered = items
                .iter()
                .enumerate()
                .map(|(index, item)| toml_value(item, &format!("{path}[{index}]")))
                .collect::<anyhow::Result<Vec<_>>>()?;
            format!("[{}]", rendered.join(", "))
        }
        Value::Object(entries) => {
            let parts = entries
                .iter()
                .map(|(key, child)| {
                    if key.is_empty() {
                        bail!("Codex config override keys must be non-empty strings");
                    }
                    let child = toml_value(child, &format!("{path}.{key}"))?;
                    let key = if toml_bare_key(key) {
                        key.clone()
                    } else {
                        serde_json::to_string(key)?
                    };
                    Ok(format!("{key} = {child}"))
                })
                .collect::<anyhow::Result<Vec<_>>>()?;
            format!("{{{}}}", parts.join(", "))
        }
        Value::Null => bail!("Codex config override at {path} cannot be null"),
    })
}

fn flatten(value: &Value, prefix: &str, overrides: &mut Vec<String>) -> anyhow::Result<()> {
    let Value::Object(entries) = value else {
        if prefix.is_empty() {
            bail!("Codex config overrides must be a plain object");
        }
        overrides.push(format!("{prefix}={}", toml_value(value, prefix)?));
        return Ok(());
    };
    if entries.is_empty() {
        if !prefix.is_empty() {
            overrides.push(format!("{prefix}={{}}"));
        }
        return Ok(());
    }
    for (key, child) in entries {
        if key.is_empty() {
            bail!("Codex config override keys must be non-empty strings");
        }
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        if child.is_object() {
            flatten(child, &path, overrides)?;
        } else {
            overrides.push(format!("{path}={}", toml_value(child, &path)?));
        }
    }
    Ok(())
}

/// The SDK's serializeConfigOverrides: `config` as dotted `key=TOML value` overrides.
pub fn serialize_config_overrides(config: &Value) -> anyhow::Result<Vec<String>> {
    let mut overrides = Vec::new();
    if !config.is_null() {
        flatten(config, "", &mut overrides)?;
    }
    Ok(overrides)
}

/// codex's arguments, in the order the SDK builds them.
pub fn codex_args(options: &CodexOptions, run: &CodexRun) -> anyhow::Result<Vec<String>> {
    let mut args = vec!["exec".to_string(), "--experimental-json".to_string()];
    for config in serialize_config_overrides(&options.config)? {
        args.extend(["--config".to_string(), config]);
    }
    let thread = &run.options;
    if let Some(model) = &thread.model {
        args.extend(["--model".to_string(), model.clone()]);
    }
    if let Some(sandbox) = &thread.sandbox_mode {
        args.extend(["--sandbox".to_string(), sandbox.clone()]);
    }
    if let Some(dir) = &thread.working_directory {
        args.extend(["--cd".to_string(), dir.display().to_string()]);
    }
    if thread.skip_git_repo_check {
        args.push("--skip-git-repo-check".into());
    }
    if let Some(effort) = &thread.model_reasoning_effort {
        args.extend([
            "--config".to_string(),
            format!("model_reasoning_effort=\"{effort}\""),
        ]);
    }
    if let Some(policy) = &thread.approval_policy {
        args.extend([
            "--config".to_string(),
            format!("approval_policy=\"{policy}\""),
        ]);
    }
    if let Some(id) = &run.thread_id {
        args.extend(["resume".to_string(), id.clone()]);
    }
    Ok(args)
}

const ORIGINATOR: &str = "CODEX_INTERNAL_ORIGINATOR_OVERRIDE";

/// codex's environment: ours, marked as the TypeScript SDK's, with its own folders first on PATH.
pub fn codex_env(mut env: Vec<(String, String)>, path_dirs: &[PathBuf]) -> Vec<(String, String)> {
    if !env
        .iter()
        .any(|(key, value)| key == ORIGINATOR && !value.is_empty())
    {
        env.retain(|(key, _)| key != ORIGINATOR);
        env.push((ORIGINATOR.into(), "codex_sdk_ts".into()));
    }
    prepend_path_dirs(&mut env, path_dirs);
    env
}

/// Runs the codex program: the real CodexExec.
#[derive(Debug, Clone)]
pub struct CodexCli {
    program: Option<CliProgram>,
    options: CodexOptions,
}

impl CodexCli {
    /// The bundled codex, or the one on PATH (cli::find_codex).
    pub fn new(options: CodexOptions) -> Self {
        CodexCli {
            program: cli::find_codex(),
            options,
        }
    }

    pub fn at(program: CliProgram, options: CodexOptions) -> Self {
        CodexCli {
            program: Some(program),
            options,
        }
    }
}

impl CodexExec for CodexCli {
    fn run(&self, run: CodexRun) -> Messages {
        let (tx, rx) = mpsc::channel(64);
        let (program, options) = (self.program.clone(), self.options.clone());
        tokio::spawn(async move {
            if let Err(error) = drive(program, options, run, tx.clone()).await {
                let _ = tx.send(Err(error)).await;
            }
        });
        rx
    }
}

const ABORTED: &str = "The operation was aborted";

async fn drive(
    program: Option<CliProgram>,
    options: CodexOptions,
    run: CodexRun,
    tx: mpsc::Sender<anyhow::Result<Value>>,
) -> anyhow::Result<()> {
    let program = program
        .ok_or_else(|| anyhow!("Couldn't find Codex (codex) next to Linker-Edit or on PATH."))?;
    if run.signal.is_cancelled() {
        bail!(ABORTED);
    }
    let env: Vec<(String, String)> = std::env::vars_os()
        .filter_map(|(key, value)| Some((key.into_string().ok()?, value.into_string().ok()?)))
        .collect();
    let mut child = cli::command(&program.program)
        .args(codex_args(&options, &run)?)
        .env_clear()
        .envs(codex_env(env, &program.path_dirs))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| anyhow!("Couldn't start Codex: {error}"))?;

    let mut stdin = child.stdin.take().context("Codex has no stdin")?;
    stdin.write_all(run.input.as_bytes()).await?;
    drop(stdin);
    let stderr = child.stderr.take().map(|mut stderr| {
        tokio::spawn(async move {
            let mut bytes = Vec::new();
            let _ = stderr.read_to_end(&mut bytes).await;
            bytes
        })
    });
    let stdout = child.stdout.take().context("Codex has no stdout")?;
    let mut lines = BufReader::new(stdout).lines();
    loop {
        let line = tokio::select! {
            _ = run.signal.cancelled() => {
                let _ = child.kill().await;
                bail!(ABORTED);
            }
            line = lines.next_line() => line.context("Couldn't read Codex's output")?,
        };
        let Some(line) = line else { break };
        let event: Value = serde_json::from_str(&line).map_err(|error| {
            anyhow::Error::new(error).context(format!("Failed to parse item: {line}"))
        })?;
        if tx.send(Ok(event)).await.is_err() {
            // Nobody reads any more; dropping the child kills it.
            return Ok(());
        }
    }
    let status = tokio::select! {
        _ = run.signal.cancelled() => None,
        status = child.wait() => Some(status?),
    };
    let Some(status) = status else {
        let _ = child.kill().await;
        bail!(ABORTED);
    };
    if !status.success() {
        let stderr = match stderr {
            Some(reader) => reader.await.unwrap_or_default(),
            None => Vec::new(),
        };
        let detail = match status.code() {
            Some(code) => format!("code {code}"),
            None => "a signal".to_string(),
        };
        bail!(
            "Codex Exec exited with {detail}: {}",
            String::from_utf8_lossy(&stderr)
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::testing::{FakeCliRun, fake_cli};
    use serde_json::json;

    fn run_in(fake: &FakeCliRun, thread_id: Option<&str>) -> CodexRun {
        CodexRun {
            input: "check".into(),
            thread_id: thread_id.map(str::to_string),
            options: ThreadOptions {
                working_directory: Some(fake.dir.path().to_path_buf()),
                skip_git_repo_check: true,
                sandbox_mode: Some("workspace-write".into()),
                approval_policy: Some("never".into()),
                ..ThreadOptions::default()
            },
            signal: CancellationToken::new(),
        }
    }

    async fn run_to_end(mut run: Messages) -> (Vec<Value>, Option<String>) {
        let mut events = Vec::new();
        while let Some(event) = run.recv().await {
            match event {
                Ok(event) => events.push(event),
                Err(error) => return (events, Some(error.to_string())),
            }
        }
        (events, None)
    }

    #[test]
    fn writes_config_overrides_as_the_sdk_does() {
        // Keys in alphabetical order: serde_json keeps insertion order where gpui turns preserve_order on.
        let config = json!({
            "inline": { "list": [{ "a b": true }] },
            "mcp_servers": {
                "empty": {},
                "linker": { "args": ["mcp"], "command": "C:/x.exe", "env": { "A": "1" }, "tool_timeout_sec": 3600 },
            },
        });
        assert_eq!(
            serialize_config_overrides(&config).unwrap(),
            [
                "inline.list=[{\"a b\" = true}]",
                "mcp_servers.empty={}",
                "mcp_servers.linker.args=[\"mcp\"]",
                "mcp_servers.linker.command=\"C:/x.exe\"",
                "mcp_servers.linker.env.A=\"1\"",
                "mcp_servers.linker.tool_timeout_sec=3600",
            ]
        );
        assert_eq!(
            serialize_config_overrides(&json!({ "a": null }))
                .unwrap_err()
                .to_string(),
            "Codex config override at a cannot be null"
        );
    }

    #[tokio::test]
    async fn runs_codex_exec_with_the_sdks_arguments_and_environment() {
        let fake = FakeCliRun::new(json!([
            { "read_to_end": true },
            { "out": { "type": "thread.started", "thread_id": "t1" } },
            { "out": { "type": "item.completed", "item": { "id": "item_0", "type": "agent_message", "text": "OK" } } },
        ]));
        let path_dir = fake.dir.path().join("codex-path");
        let codex = CodexCli::at(
            CliProgram {
                program: fake_cli(),
                path_dirs: vec![path_dir.clone()],
            },
            CodexOptions {
                config: json!({ "mcp_servers": { "linker": { "args": ["mcp"], "command": "C:/x.exe" } } }),
            },
        );
        let (events, error) = run_to_end(codex.run(run_in(&fake, Some("t1")))).await;
        assert_eq!(error, None);
        assert_eq!(events.len(), 2);

        let log = fake.log();
        let dir = fake.dir.path().display().to_string();
        assert_eq!(
            log[0]["argv"],
            json!([
                "exec",
                "--experimental-json",
                "--config",
                "mcp_servers.linker.args=[\"mcp\"]",
                "--config",
                "mcp_servers.linker.command=\"C:/x.exe\"",
                "--sandbox",
                "workspace-write",
                "--cd",
                dir,
                "--skip-git-repo-check",
                "--config",
                "approval_policy=\"never\"",
                "resume",
                "t1",
            ])
        );
        let env = log[0]["env"].as_object().unwrap();
        let originator = std::env::var(ORIGINATOR).unwrap_or_else(|_| "codex_sdk_ts".into());
        assert_eq!(env[ORIGINATOR], json!(originator));
        let path = env
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("path"))
            .unwrap()
            .1;
        assert!(
            path.as_str()
                .unwrap()
                .starts_with(&path_dir.display().to_string())
        );
        // The prompt on stdin, which then closes.
        assert_eq!(
            &log[1..],
            [json!({ "stdin": "check" }), json!({ "eof": true })]
        );
    }

    #[tokio::test]
    async fn reports_a_failed_run_and_output_that_isnt_json() {
        let program = CliProgram {
            program: fake_cli(),
            path_dirs: vec![],
        };
        let codex = CodexCli::at(program, CodexOptions::default());
        let fake = FakeCliRun::new(json!([{ "stderr": "boom" }, { "exit": 2 }]));
        let (_, error) = run_to_end(codex.run(run_in(&fake, None))).await;
        assert_eq!(
            error.as_deref(),
            Some("Codex Exec exited with code 2: boom")
        );

        let fake = FakeCliRun::new(json!([{ "line": "nope" }]));
        let (_, error) = run_to_end(codex.run(run_in(&fake, None))).await;
        assert_eq!(error.as_deref(), Some("Failed to parse item: nope"));

        let missing = CodexCli {
            program: None,
            options: CodexOptions::default(),
        };
        let (_, error) = run_to_end(missing.run(run_in(&fake, None))).await;
        assert!(error.unwrap().starts_with("Couldn't find Codex"));
    }

    #[tokio::test]
    async fn stops_codex_when_the_turn_is_aborted() {
        let fake = FakeCliRun::new(json!([
            { "out": { "type": "thread.started", "thread_id": "t1" } },
            { "hang": true },
        ]));
        let codex = CodexCli::at(
            CliProgram {
                program: fake_cli(),
                path_dirs: vec![],
            },
            CodexOptions::default(),
        );
        let run = run_in(&fake, None);
        let signal = run.signal.clone();
        let mut events = codex.run(run);
        assert_eq!(
            events.recv().await.unwrap().unwrap()["type"],
            "thread.started"
        );
        signal.cancel();
        assert_eq!(
            events.recv().await.unwrap().unwrap_err().to_string(),
            "The operation was aborted"
        );
    }

    /// One real turn with the codex on this computer: no MCP servers, read-only, in an empty folder. Costs a
    /// few tokens: `cargo test -p linker-edit-server live_codex -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore = "runs the real codex"]
    async fn live_codex_turn() {
        let dir = tempfile::tempdir().unwrap();
        let codex = CodexCli::new(CodexOptions {
            config: json!({ "mcp_servers": {} }),
        });
        let run = codex.run(CodexRun {
            input: "Reply with OK and nothing else.".into(),
            thread_id: None,
            options: ThreadOptions {
                working_directory: Some(dir.path().to_path_buf()),
                skip_git_repo_check: true,
                sandbox_mode: Some("read-only".into()),
                approval_policy: Some("never".into()),
                model_reasoning_effort: Some("low".into()),
                ..ThreadOptions::default()
            },
            signal: CancellationToken::new(),
        });
        let (events, error) = run_to_end(run).await;
        for event in &events {
            println!(
                "{}",
                event.to_string().chars().take(300).collect::<String>()
            );
        }
        assert_eq!(error, None);
        assert_eq!(events[0]["type"], "thread.started");
        let reply = events
            .iter()
            .find(|e| e["type"] == "item.completed" && e["item"]["type"] == "agent_message")
            .unwrap();
        assert!(reply["item"]["text"].as_str().unwrap().contains("OK"));
    }
}
