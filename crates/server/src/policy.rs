// The guardrails: what agents may never do, in either approval mode, and what waits for the user's OK in "ask"
// mode. The README's "How approvals work" is the contract; keep the two in step.
//
// Regexes here use ASCII word boundaries and classes, as JavaScript's do, so a refusal matches exactly where the
// TypeScript's did.

use std::sync::LazyLock;

use linker_edit_protocol::ApprovalMode;
use regex::Regex;
use serde_json::Value;

use crate::resolve::tools::{ResolveToolKind, find_resolve_tool};
use crate::rules::{LINKER_MCP_NAME, RESOLVE_MCP_NAME};

/// library::tools' LIBRARY_IMPORT: the library tool that brings files into the media pool, so a write.
const LIBRARY_IMPORT: &str = "library_import";

const READ_ONLY_TOOLS: [&str; 10] = [
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

static LINKER_PREFIX: LazyLock<String> = LazyLock::new(|| format!("mcp__{LINKER_MCP_NAME}__"));
static RESOLVE_PREFIX: LazyLock<String> = LazyLock::new(|| format!("mcp__{RESOLVE_MCP_NAME}__"));

/// Resolve's own MCP tools that only read: its scripting API and docs, and the LUTs and DCTLs it has.
const RESOLVE_READS: [&str; 7] = [
    "get_resolve_status",
    "get_whats_new",
    "get_scripting_api",
    "search_scripting_api",
    "get_scripting_docs",
    "list_luts",
    "list_dctls",
];

/// Resolve's own MCP tools that write files only into Resolve's LUT folder, after the user's OK.
const RESOLVE_LOOK_WRITES: [&str; 2] = ["update_dctl", "generate_lut"];

/// The escape hatch: any Python against Resolve. Scripts that break the rules are refused in every mode;
/// the rest run like any other change, so they ask, showing the script, only in "ask" mode.
pub const RUN_SCRIPT: &str = "run_script";

/// Starts Resolve when it isn't running. It changes no project, so it's fine for agents.
pub const LAUNCH_RESOLVE: &str = "launch_resolve";

/// Resolve's own MCP tools agents never get: scripts with full system access and deleting LUTs. Unknown
/// tools from a newer Resolve land here too, until they're reviewed.
pub fn is_denied_resolve_tool(tool: &str) -> bool {
    !RESOLVE_READS.contains(&tool)
        && !RESOLVE_LOOK_WRITES.contains(&tool)
        && tool != RUN_SCRIPT
        && tool != LAUNCH_RESOLVE
}

/// Codex has no per-call hook to check a script or back the project up first, so it gets only Resolve's
/// read tools and launch_resolve; these are switched off for it.
pub const CODEX_DISABLED_RESOLVE_TOOLS: [&str; 6] = [
    "run_script",
    "run_script_unsafe",
    "update_dctl",
    "generate_lut",
    "delete_dctl",
    "delete_lut",
];

/// For Claude's tool list: the tools of Resolve's own MCP server it never gets.
pub static CLAUDE_DISALLOWED_RESOLVE_TOOLS: LazyLock<Vec<String>> = LazyLock::new(|| {
    ["run_script_unsafe", "delete_dctl", "delete_lut"]
        .iter()
        .map(|tool| format!("{}{tool}", *RESOLVE_PREFIX))
        .collect()
});

/// Resolve's own run_script, as agents see it. It changes the project, so the project is backed up first.
pub static RESOLVE_RUN_SCRIPT: LazyLock<String> =
    LazyLock::new(|| format!("{}{RUN_SCRIPT}", *RESOLVE_PREFIX));

/// JavaScript's \s, for regex classes: Rust's also has U+0085 and lacks the BOM.
const JS_SPACE: &str = r"\t\n\x0B\x0C\r \x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}\x{feff}";

// Reading the edit history: plain or quoted arguments only, so no chaining, substitution or redirects.
static GIT_READ: LazyLock<Regex> = LazyLock::new(|| {
    let pattern =
        r#"^git[ \t]+(log|show|diff|status)([ \t]+("[^"$`\\]*"|'[^']*'|[^SPACE;&|<>`$()"'\\])+)*$"#;
    Regex::new(&pattern.replace("SPACE", JS_SPACE)).unwrap()
});

fn is_git_read(command: &str) -> bool {
    // --output writes the result to a file.
    GIT_READ.is_match(js_trim(command)) && !command.contains("--output")
}

/// String.prototype.trim: JavaScript's whitespace and line terminators.
fn js_trim(text: &str) -> &str {
    text.trim_matches(|c: char| {
        matches!(
            c,
            '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
                ..='\u{200a}'
        ) || matches!(
            c,
            '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'
        )
    })
}

/// Does this tool call change the open Resolve project? True for Linker-Edit's write tools and Resolve's
/// run_script, which can do anything. These are the calls the edit history lists after a turn.
pub fn is_resolve_write(tool_name: &str) -> bool {
    if let Some(tool) = tool_name.strip_prefix(LINKER_PREFIX.as_str()) {
        return tool == LIBRARY_IMPORT
            || find_resolve_tool(tool).is_some_and(|tool| tool.kind == ResolveToolKind::Write);
    }
    tool_name == RESOLVE_RUN_SCRIPT.as_str()
}

/// In "ask" mode: does this tool call need the user's OK before it runs?
pub fn needs_approval(tool_name: &str, input: &Value) -> bool {
    if READ_ONLY_TOOLS.contains(&tool_name) {
        return false;
    }
    // The Linker-Edit server asks for its own tools' writes itself, for every agent alike.
    if tool_name.starts_with(LINKER_PREFIX.as_str()) {
        return false;
    }
    if let Some(tool) = tool_name.strip_prefix(RESOLVE_PREFIX.as_str()) {
        return !RESOLVE_READS.contains(&tool);
    }
    // Claude Code on Windows runs shell commands through PowerShell instead of Bash.
    let shell = tool_name == "Bash" || tool_name == "PowerShell";
    if shell
        && let Some(command) = input.get("command").and_then(Value::as_str)
        && is_git_read(command)
    {
        return false;
    }
    true
}

// What scripts may not do through run_script, in any mode: each thread belongs to one Resolve project,
// which only the user switches, in Linker-Edit; media, bins, timelines, projects, the user's render queue
// and render presets are never deleted; and a clip's media is never swapped for another file. With full
// access no one reads a script before it runs, so this check is what stops these mistakes.
// Matched as names, called or not, so `f = pm.LoadProject; f("x")` is refused too. Python can still build
// a name at run time, so getattr and code from strings are refused as well. The backup is the real safety
// net: this check stops mistakes, it can't stop a script written to get around it.
// (?-u:\b) is JavaScript's ASCII word boundary.
static FORBIDDEN_CALLS: LazyLock<Regex> = LazyLock::new(|| {
    let pattern = concat!(
        r"(?-u:\b)(LoadProject|LoadCloudProject|CloseProject|CreateProject|CreateCloudProject|DeleteProject|ImportProject|ImportCloudProject|RestoreProject|RestoreCloudProject|ArchiveProject|SetCurrentDatabase|DeleteTimelines|DeleteFolders|DeleteClips|DeleteClipMattes|DeleteRenderJob|DeleteAllRenderJobs|DeleteRenderPreset|ReplaceClip|Quit)(?-u:\b)",
        r"|(?-u:\b)(getattr|eval|exec|compile|__import__)[SPACE]*\(",
        r"|__getattribute__"
    );
    Regex::new(&pattern.replace("SPACE", JS_SPACE)).unwrap()
});

pub const FORBIDDEN_SCRIPT: &str = "Linker-Edit doesn't let scripts open, close, create or switch Resolve projects or databases, delete media, bins, timelines, render jobs or render presets, or replace a clip's media, and scripts may not name those calls or use getattr, eval or exec. Use the linker tools (remove_clips takes clips off an AI timeline), or ask the user to switch projects in Linker-Edit's sidebar.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolDecision {
    Allow,
    Deny { message: String },
}

/// What Linker-Edit refuses whatever the mode: Resolve's denied tools, and scripts that can't be checked or
/// break the rules. None when the call may go on. Claude runs this in a PreToolUse hook too, because full
/// access (bypassPermissions) never calls canUseTool.
pub fn refusal(tool_name: &str, input: &Value) -> Option<String> {
    let tool = tool_name.strip_prefix(RESOLVE_PREFIX.as_str())?;
    if is_denied_resolve_tool(tool) {
        return Some(format!("Linker-Edit doesn't let agents use {tool}."));
    }
    if tool == RUN_SCRIPT {
        // A script that can't be checked doesn't run.
        let Some(script) = input.get("script").and_then(Value::as_str) else {
            return Some("run_script needs the script as text.".to_string());
        };
        if FORBIDDEN_CALLS.is_match(script) {
            return Some(FORBIDDEN_SCRIPT.to_string());
        }
    }
    None
}

/// One policy for every agent, like T3 Code's runtime modes: "auto" is full access, "ask" is supervised.
/// Denied tools and forbidden scripts are refused in both; in "ask" mode, changes wait for the user's OK,
/// which `ask` asks for.
pub async fn decide_tool_call<F, Fut>(
    tool_name: &str,
    input: &Value,
    mode: ApprovalMode,
    ask: F,
) -> ToolDecision
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = bool>,
{
    if let Some(message) = refusal(tool_name, input) {
        return ToolDecision::Deny { message };
    }
    if mode == ApprovalMode::Ask && needs_approval(tool_name, input) && !ask().await {
        return ToolDecision::Deny {
            message: "The user declined this action.".to_string(),
        };
    }
    ToolDecision::Allow
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use serde_json::json;

    use super::*;
    use crate::resolve::tools::resolve_tools;

    #[test]
    fn needs_approval_for_changes_only() {
        for (tool, input, expected) in [
            // Agent built-ins
            ("Read", json!({}), false),
            ("Grep", json!({}), false),
            ("ToolSearch", json!({}), false),
            ("Bash", json!({ "command": "ls" }), true),
            // Reading the edit history runs unasked; anything that could write still asks.
            ("Bash", json!({ "command": "git log -- resolve" }), false),
            (
                "Bash",
                json!({ "command": "git show \"a1b2c3d:resolve/My Doc/AI_Cut/cuts.json\"" }),
                false,
            ),
            (
                "Bash",
                json!({ "command": "git diff HEAD~1 --stat " }),
                false,
            ),
            ("Bash", json!({ "command": "git commit -m x" }), true),
            (
                "Bash",
                json!({ "command": "git log; rm -rf resolve" }),
                true,
            ),
            (
                "Bash",
                json!({ "command": "git log\nrm -rf resolve" }),
                true,
            ),
            ("Bash", json!({ "command": "git log > out.txt" }), true),
            ("Bash", json!({ "command": "git show $(rm x)" }), true),
            (
                "Bash",
                json!({ "command": "git diff --output=x.patch" }),
                true,
            ),
            (
                "PowerShell",
                json!({ "command": "git show \"0154f5b:resolve/Demo/AI - Cut/cuts.json\"" }),
                false,
            ),
            (
                "PowerShell",
                json!({ "command": "$p=\"resolve\"; git log -- $p" }),
                true,
            ),
            (
                "PowerShell",
                json!({ "command": "Remove-Item resolve" }),
                true,
            ),
            ("Write", json!({ "file_path": "notes/a.md" }), true),
            ("Edit", json!({}), true),
            ("Agent", json!({}), true),
            // Other MCP servers are never trusted as reads.
            ("mcp__gmail__search_threads", json!({}), true),
            // Linker-Edit's own tools: the server asks for their changes, so the agent's hook never does.
            (
                "mcp__linker__create_timeline",
                json!({ "name": "Cut" }),
                false,
            ),
            ("mcp__linker__transcribe", json!({ "files": [] }), false),
            // Resolve's own server: its docs run unasked, anything else asks.
            (
                "mcp__resolve__search_scripting_api",
                json!({ "pattern": "Marker" }),
                false,
            ),
            ("mcp__resolve__get_scripting_docs", json!({}), false),
            ("mcp__resolve__generate_lut", json!({}), true),
            (
                "mcp__resolve__run_script",
                json!({ "script": "x = 1" }),
                true,
            ),
            ("mcp__resolve__something_new", json!({}), true),
        ] {
            assert_eq!(needs_approval(tool, &input), expected, "{tool} {input}");
        }
    }

    #[test]
    fn is_resolve_write_lists_write_tools_and_resolve_scripts_not_reads_views_or_transcription() {
        let writes: Vec<&str> = resolve_tools()
            .iter()
            .filter(|tool| is_resolve_write(&format!("mcp__linker__{}", tool.name)))
            .map(|tool| tool.name)
            .collect();
        let expected: Vec<&str> = resolve_tools()
            .iter()
            .filter(|tool| tool.kind == ResolveToolKind::Write)
            .map(|tool| tool.name)
            .collect();
        assert_eq!(writes, expected);
        assert!(writes.contains(&"create_timeline"));
        assert!(!writes.contains(&"set_current_timeline"));
        assert!(!is_resolve_write("mcp__linker__transcribe"));
        assert!(!is_resolve_write("mcp__linker__library_search"));
        assert!(is_resolve_write("mcp__linker__library_import"));
        assert!(is_resolve_write("mcp__resolve__run_script"));
        assert!(!is_resolve_write("mcp__resolve__get_scripting_api"));
    }

    /// The decision, and the tools the user was asked about.
    async fn decide(
        tool: &str,
        input: Value,
        mode: ApprovalMode,
        answer: bool,
    ) -> (ToolDecision, Vec<String>) {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let record = asked.clone();
        let decision = decide_tool_call(tool, &input, mode, || async move {
            record.lock().unwrap().push(tool.to_string());
            answer
        })
        .await;
        let asked = asked.lock().unwrap().clone();
        (decision, asked)
    }

    const BOTH: [ApprovalMode; 2] = [ApprovalMode::Auto, ApprovalMode::Ask];

    #[tokio::test]
    async fn runs_a_script_without_asking_in_auto_mode_and_asks_first_in_ask_mode() {
        let script = json!({ "script": "result = project.GetName()" });
        let run_script = "mcp__resolve__run_script";
        assert_eq!(
            decide(run_script, script.clone(), ApprovalMode::Auto, true).await,
            (ToolDecision::Allow, vec![])
        );
        assert_eq!(
            decide(run_script, script.clone(), ApprovalMode::Ask, true).await,
            (ToolDecision::Allow, vec![run_script.to_string()])
        );
        assert!(matches!(
            decide(run_script, script, ApprovalMode::Ask, false).await.0,
            ToolDecision::Deny { .. }
        ));
    }

    #[tokio::test]
    async fn refuses_a_script_that_isnt_text_in_both_modes() {
        for mode in BOTH {
            for input in [json!({ "script": ["x = 1"] }), json!({})] {
                let (decision, asked) = decide("mcp__resolve__run_script", input, mode, true).await;
                assert_eq!(
                    decision,
                    ToolDecision::Deny {
                        message: "run_script needs the script as text.".into()
                    }
                );
                assert!(asked.is_empty());
            }
        }
    }

    #[tokio::test]
    async fn refuses_a_script_that_switches_projects_deletes_or_swaps_media_in_both_modes() {
        for script in [
            r#"resolve.GetProjectManager().LoadProject("Other")"#,
            "pm.SetCurrentDatabase({'DbType': 'Disk', 'DbName': 'X'})",
            "project.GetMediaPool().DeleteClips([clip])",
            "pool.DeleteTimelines([t])",
            "pm.CloseProject (project)",
            "project.DeleteAllRenderJobs()",
            "project.DeleteRenderJob(job_id)",
            "project.DeleteRenderPreset('Mine')",
            "item.ReplaceClip('D:/other.mov')",
            "load = pm.LoadProject\nload(\"Other\")",
            r#"getattr(pm, "Load" + "Project")("Other")"#,
            r#"eval("pm.Load" + "Project(1)")"#,
            "exec(code)",
            // JavaScript's \b is ASCII: a letter like "é" before a name doesn't hide it.
            "éLoadProject()",
            "x.__getattribute__('LoadProject')",
            "getattr\u{feff}(pm, name)",
        ] {
            for mode in BOTH {
                assert_eq!(
                    decide(
                        "mcp__resolve__run_script",
                        json!({ "script": script }),
                        mode,
                        true
                    )
                    .await,
                    (
                        ToolDecision::Deny {
                            message: FORBIDDEN_SCRIPT.into()
                        },
                        vec![]
                    ),
                    "{script}"
                );
            }
        }
        // Names inside longer names are fine.
        assert_eq!(
            refusal(
                "mcp__resolve__run_script",
                &json!({ "script": "MyLoadProjects = 1; evaluate(x)" })
            ),
            None
        );
    }

    #[tokio::test]
    async fn lets_agents_start_resolve_asking_only_in_ask_mode() {
        let launch = "mcp__resolve__launch_resolve";
        assert_eq!(
            decide(launch, json!({}), ApprovalMode::Auto, true).await,
            (ToolDecision::Allow, vec![])
        );
        assert_eq!(
            decide(launch, json!({}), ApprovalMode::Ask, true).await,
            (ToolDecision::Allow, vec![launch.to_string()])
        );
    }

    #[tokio::test]
    async fn never_lets_agents_use_resolves_dangerous_or_unknown_tools_in_either_mode() {
        for tool in [
            "run_script_unsafe",
            "delete_lut",
            "delete_dctl",
            "a_tool_from_a_newer_resolve",
        ] {
            for mode in BOTH {
                let (decision, asked) =
                    decide(&format!("mcp__resolve__{tool}"), json!({}), mode, true).await;
                assert_eq!(
                    decision,
                    ToolDecision::Deny {
                        message: format!("Linker-Edit doesn't let agents use {tool}.")
                    }
                );
                assert!(asked.is_empty());
            }
        }
    }

    #[tokio::test]
    async fn asks_for_other_changes_only_in_ask_mode() {
        let write = json!({ "file_path": "a.md" });
        assert_eq!(
            decide("Write", write.clone(), ApprovalMode::Auto, true).await,
            (ToolDecision::Allow, vec![])
        );
        assert_eq!(
            decide("Write", write, ApprovalMode::Ask, true).await,
            (ToolDecision::Allow, vec!["Write".to_string()])
        );
        assert_eq!(
            decide(
                "mcp__resolve__generate_lut",
                json!({}),
                ApprovalMode::Ask,
                false
            )
            .await
            .0,
            ToolDecision::Deny {
                message: "The user declined this action.".into()
            }
        );
    }

    #[test]
    fn names_the_tools_each_agent_never_gets() {
        assert_eq!(
            *CLAUDE_DISALLOWED_RESOLVE_TOOLS,
            [
                "mcp__resolve__run_script_unsafe",
                "mcp__resolve__delete_dctl",
                "mcp__resolve__delete_lut"
            ]
        );
        assert_eq!(*RESOLVE_RUN_SCRIPT, "mcp__resolve__run_script");
        for tool in CODEX_DISABLED_RESOLVE_TOOLS {
            assert!(!RESOLVE_READS.contains(&tool));
        }
    }
}
