//! Stands in for `claude` and `codex` in the driver tests (cargo test builds it): plays the steps in
//! fake-cli.json and logs what it was given to fake-cli.log, both in the folder it runs in (codex: --cd).
//!
//! Steps, in order:
//!   {"out": value}                     writes value as one JSON line to stdout
//!   {"line": "text"}                   writes a raw line
//!   {"stderr": "text"}                 writes to stderr
//!   {"read": n}                        reads n stdin lines
//!   {"answer": value}                  reads a control_request and answers it with value
//!   {"request": value, "id": "r1"}     sends a control_request and reads until its control_response
//!   {"read_to_end": true}              reads stdin until it closes
//!   {"hang": true}                     waits until it is killed
//!   {"exit": code}                     exits with that code
//!
//! The log is JSON lines: {"argv", "env"} first, then {"stdin": line} per line read and {"eof": true}.

use std::io::{BufRead, Write};
use std::path::PathBuf;

use serde_json::{Value, json};

struct Fake {
    log: std::fs::File,
    stdin: std::io::StdinLock<'static>,
    eof: bool,
}

impl Fake {
    fn record(&mut self, entry: Value) {
        writeln!(self.log, "{entry}").unwrap();
        self.log.flush().unwrap();
    }

    fn read_line(&mut self) -> Option<String> {
        if self.eof {
            return None;
        }
        let mut line = String::new();
        if self.stdin.read_line(&mut line).unwrap_or(0) == 0 {
            self.eof = true;
            self.record(json!({ "eof": true }));
            return None;
        }
        let line = line.trim_end_matches(['\r', '\n']).to_string();
        self.record(json!({ "stdin": line }));
        Some(line)
    }
}

fn out(value: &Value) {
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{value}").unwrap();
    stdout.flush().unwrap();
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let dir = argv
        .iter()
        .position(|arg| arg == "--cd")
        .and_then(|index| argv.get(index + 1))
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap());
    let Ok(script) = std::fs::read_to_string(dir.join("fake-cli.json")) else {
        // Run by cargo as an example, or anywhere without a script: nothing to play.
        return;
    };
    let script: Value = serde_json::from_str(&script).unwrap();
    // Only what the drivers set; the rest of the environment stays out of the log.
    let env: serde_json::Map<String, Value> = std::env::vars()
        .filter(|(key, _)| {
            [
                "CLAUDE_CODE_ENTRYPOINT",
                "CLAUDE_AGENT_SDK_VERSION",
                "CLAUDE_CODE_SDK_READS_SESSION_STATE",
                "CODEX_INTERNAL_ORIGINATOR_OVERRIDE",
                "NODE_OPTIONS",
                "DEBUG",
            ]
            .contains(&key.as_str())
                || key.eq_ignore_ascii_case("path")
        })
        .map(|(key, value)| (key, json!(value)))
        .collect();
    let mut fake = Fake {
        log: std::fs::File::create(dir.join("fake-cli.log")).unwrap(),
        stdin: std::io::stdin().lock(),
        eof: false,
    };
    fake.record(json!({ "argv": argv, "env": env }));

    for step in script["steps"].as_array().unwrap() {
        if let Some(value) = step.get("out") {
            out(value);
        } else if let Some(line) = step.get("line") {
            let mut stdout = std::io::stdout().lock();
            writeln!(stdout, "{}", line.as_str().unwrap()).unwrap();
            stdout.flush().unwrap();
        } else if let Some(text) = step.get("stderr") {
            eprint!("{}", text.as_str().unwrap());
        } else if let Some(count) = step.get("read") {
            for _ in 0..count.as_u64().unwrap() {
                fake.read_line();
            }
        } else if let Some(response) = step.get("answer") {
            let request: Value = serde_json::from_str(&fake.read_line().unwrap()).unwrap();
            assert_eq!(request["type"], "control_request");
            out(&json!({
                "type": "control_response",
                "response": { "subtype": "success", "request_id": request["request_id"], "response": response },
            }));
        } else if let Some(request) = step.get("request") {
            let id = step["id"].clone();
            out(&json!({ "type": "control_request", "request_id": id, "request": request }));
            while let Some(line) = fake.read_line() {
                let message: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
                if message["type"] == "control_response" && message["response"]["request_id"] == id
                {
                    break;
                }
            }
        } else if step.get("read_to_end").is_some() {
            while fake.read_line().is_some() {}
        } else if step.get("hang").is_some() {
            // A turn that goes on until the driver stops it.
            loop {
                std::thread::park();
            }
        } else if let Some(code) = step.get("exit") {
            std::process::exit(code.as_i64().unwrap() as i32);
        }
    }
}
