// Stand-ins for Resolve in tests: Linker-Edit's real scripts run in this computer's Python against a stand-in
// scripting module, or a connection that answers from closures and records the scripts it was asked to run.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;
use tokio::io::AsyncWriteExt;

use super::connection::{ResolveConnection, parse_script_result, script_source};

/// Stands in for Resolve's scripting module (fixtures/DaVinciResolveScript.py): state from the JSON file in
/// FAKE_RESOLVE, every call that changes something logged to FAKE_RESOLVE + ".log".
pub const FAKE_RESOLVE_MODULE: &str = include_str!("fixtures/DaVinciResolveScript.py");

pub const PYTHON: &str = if cfg!(windows) { "python" } else { "python3" };

static HAS_PYTHON: LazyLock<bool> = LazyLock::new(|| {
    std::process::Command::new(PYTHON)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
});

/// Is there a Python to run the scripts in? Tests that need one pass without it, as the TypeScript skipped them.
pub fn has_python() -> bool {
    if !*HAS_PYTHON {
        eprintln!("No {PYTHON} on this computer: skipping a test of the Resolve scripts.");
    }
    *HAS_PYTHON
}

/// The ffmpeg for tests: one on PATH, or the one npm installed with ffmpeg-static. None skips the test.
pub fn test_ffmpeg() -> Option<PathBuf> {
    crate::transcribe::tools::find_ffmpeg().ok()
}

// Plays the part of Resolve's run_script: sets `resolve` and `project` from a stand-in scripting module,
// runs the script and prints {"result": …} or {"error": traceback}, as Resolve's MCP server answers.
const RUNNER: &str = r#"
import json, sys, traceback
import DaVinciResolveScript as d
resolve = d.scriptapp("Resolve")
scope = {"resolve": resolve, "project": resolve.GetProjectManager().GetCurrentProject()}
try:
    exec(sys.stdin.read(), scope)
    print(json.dumps({"result": scope.get("result")}))
except Exception:
    print(json.dumps({"error": traceback.format_exc()}))
"#;

/// A connection that runs Linker-Edit's real scripts in this computer's Python against `module`, a stand-in
/// DaVinciResolveScript module. env reaches the module, for state files.
pub struct PythonConnection {
    dir: tempfile::TempDir,
    env: Vec<(String, String)>,
}

pub fn python_connection(module: &str, env: &[(&str, &str)]) -> Arc<PythonConnection> {
    let dir = tempfile::Builder::new()
        .prefix("linker-edit-fake-resolve-")
        .tempdir()
        .unwrap();
    std::fs::write(dir.path().join("DaVinciResolveScript.py"), module).unwrap();
    Arc::new(PythonConnection {
        dir,
        env: env
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect(),
    })
}

#[async_trait]
impl ResolveConnection for PythonConnection {
    fn command(&self) -> Option<&str> {
        Some(PYTHON)
    }

    async fn call_tool(
        &self,
        _name: &str,
        _args: Value,
        _timeout: Duration,
    ) -> anyhow::Result<String> {
        anyhow::bail!("python_connection only runs scripts.")
    }

    async fn run_script_value(
        &self,
        name: &str,
        args: Value,
        _timeout_seconds: u64,
    ) -> anyhow::Result<Value> {
        let source = script_source(name, &args)?;
        let mut child = tokio::process::Command::new(PYTHON)
            .args(["-c", RUNNER])
            .env("PYTHONPATH", self.dir.path())
            .env("PYTHONIOENCODING", "utf-8")
            .envs(self.env.iter().map(|(key, value)| (key, value)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(source.as_bytes()).await?;
        drop(stdin);
        let out = child.wait_with_output().await?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        let last = stdout
            .trim()
            .split('\n')
            .next_back()
            .unwrap_or("")
            .trim()
            .to_string();
        let text = if last.is_empty() {
            String::from_utf8_lossy(&out.stderr).into_owned()
        } else {
            last
        };
        Ok(parse_script_result(&text)?)
    }

    fn close(&self) {}
}

type ToolFake = dyn Fn(&str, &Value) -> anyhow::Result<String> + Send + Sync;
type ScriptFake = dyn Fn(&str, &Value) -> anyhow::Result<Value> + Send + Sync;

/// A script FakeConnection was asked to run.
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptCall {
    pub name: String,
    pub args: Value,
    pub timeout: u64,
}

/// A stand-in for the connection to Resolve's MCP server that records the scripts it was asked to run.
pub struct FakeConnection {
    command: Option<String>,
    /// A ResolveMCP tool other than run_script; returns its text.
    tool: Option<Box<ToolFake>>,
    /// A Linker-Edit script by name; returns its result or fails with a ResolveError.
    script: Option<Box<ScriptFake>>,
    scripts: Mutex<Vec<ScriptCall>>,
}

impl FakeConnection {
    pub fn new(command: Option<&str>) -> Self {
        Self {
            command: command.map(str::to_string),
            tool: None,
            script: None,
            scripts: Mutex::new(Vec::new()),
        }
    }

    pub fn tool(
        mut self,
        tool: impl Fn(&str, &Value) -> anyhow::Result<String> + Send + Sync + 'static,
    ) -> Self {
        self.tool = Some(Box::new(tool));
        self
    }

    pub fn script(
        mut self,
        script: impl Fn(&str, &Value) -> anyhow::Result<Value> + Send + Sync + 'static,
    ) -> Self {
        self.script = Some(Box::new(script));
        self
    }

    pub fn scripts(&self) -> Vec<ScriptCall> {
        self.scripts.lock().unwrap().clone()
    }

    pub fn script_names(&self) -> Vec<String> {
        self.scripts().into_iter().map(|call| call.name).collect()
    }
}

/// fakeConnection(): a FakeConnection with the usual ResolveMCP program; build it up with .tool() and .script().
pub fn fake_connection() -> FakeConnection {
    FakeConnection::new(Some("ResolveMCP.exe"))
}

#[async_trait]
impl ResolveConnection for FakeConnection {
    fn command(&self) -> Option<&str> {
        self.command.as_deref()
    }

    async fn call_tool(
        &self,
        name: &str,
        args: Value,
        _timeout: Duration,
    ) -> anyhow::Result<String> {
        match &self.tool {
            Some(tool) => tool(name, &args),
            None => anyhow::bail!("No fake for {name}."),
        }
    }

    async fn run_script_value(
        &self,
        name: &str,
        args: Value,
        timeout_seconds: u64,
    ) -> anyhow::Result<Value> {
        self.scripts.lock().unwrap().push(ScriptCall {
            name: name.to_string(),
            args: args.clone(),
            timeout: timeout_seconds,
        });
        match &self.script {
            Some(script) => script(name, &args),
            None => anyhow::bail!("No fake for {name}."),
        }
    }

    fn close(&self) {}
}

/// The calls the stand-in Resolve logged to <state file>.log, one JSON array each, with numbers as JavaScript
/// reads them (Python's 10.0 is 10).
pub fn logged_calls(state_file: &Path) -> Vec<Value> {
    let log = PathBuf::from(format!("{}.log", state_file.display()));
    match std::fs::read_to_string(log) {
        Ok(text) => text
            .trim()
            .lines()
            .map(|line| crate::js::whole_numbers(serde_json::from_str(line).unwrap()))
            .collect(),
        Err(_) => Vec::new(),
    }
}
