//! Words and small derivations the views share: src/lib/labels.ts.
//!
//! Tool names and inputs come from the agents as JSON; these turn them into what a row or an
//! approval shows. The history panel's clip colors (labels.ts's CHANGE_STYLE) live with that panel.

use chrono::{Datelike, NaiveDate};
use linker_edit_protocol::{CollectionKind, Provider, ResolveLinkKind};
use serde_json::Value;

// Resolve's own MCP server, and Linker-Edit's, whose tools work in Resolve except the ones named here.
const RESOLVE_PREFIX: &str = "mcp__resolve__";
const LINKER_PREFIX: &str = "mcp__linker__";
const NOT_RESOLVE: [&str; 6] = [
    "transcribe",
    "study_media",
    "compare_cut",
    "save_brief",
    "add_lesson",
    "save_example",
];

pub fn provider_label(provider: Provider) -> &'static str {
    match provider {
        Provider::Claude => "Claude",
        Provider::Codex => "Codex",
    }
}

pub fn is_resolve_tool(name: &str) -> bool {
    name.starts_with(RESOLVE_PREFIX)
        || name
            .strip_prefix(LINKER_PREFIX)
            .is_some_and(|tool| !NOT_RESOLVE.contains(&tool))
}

/// A change to a workspace's or series' brief that an agent proposes; the user accepts or declines it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BriefRequest {
    Rules {
        for_: String,
        summary: String,
        was: String,
        rules: String,
    },
    Lesson {
        for_: String,
        feedback: String,
        rule: String,
    },
}

/// What an approval of save_brief or add_lesson shows; None for every other tool.
pub fn brief_request(name: &str, input: &Value) -> Option<BriefRequest> {
    let text = |key: &str| {
        input
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    match name.strip_prefix(LINKER_PREFIX) {
        Some("save_brief") => Some(BriefRequest::Rules {
            for_: text("for"),
            summary: text("summary"),
            was: text("was"),
            rules: text("rules"),
        }),
        Some("add_lesson") => Some(BriefRequest::Lesson {
            for_: text("for"),
            feedback: text("feedback"),
            rule: text("rule"),
        }),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Same,
    Removed,
    Added,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    pub text: String,
}

/// The lines of a text change, in order: kept, taken out or added. For showing new rules against the old.
pub fn line_diff(before: &str, after: &str) -> Vec<DiffLine> {
    let lines = |text: &str| -> Vec<String> {
        let text = text.trim();
        if text.is_empty() {
            Vec::new()
        } else {
            text.split('\n').map(str::to_string).collect()
        }
    };
    let (a, b) = (lines(before), lines(after));
    // The longest run of lines both have, from the end, so walking forward gives the changes in order.
    let mut common = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            common[i][j] = if a[i] == b[j] {
                common[i + 1][j + 1] + 1
            } else {
                common[i + 1][j].max(common[i][j + 1])
            };
        }
    }
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            out.push(DiffLine {
                kind: LineKind::Same,
                text: a[i].clone(),
            });
            i += 1;
            j += 1;
        } else if i < a.len() && (j == b.len() || common[i + 1][j] >= common[i][j + 1]) {
            out.push(DiffLine {
                kind: LineKind::Removed,
                text: a[i].clone(),
            });
            i += 1;
        } else {
            out.push(DiffLine {
                kind: LineKind::Added,
                text: b[j].clone(),
            });
            j += 1;
        }
    }
    out
}

fn script<'a>(name: &str, input: &'a Value) -> Option<&'a str> {
    if name == "mcp__resolve__run_script" {
        input.get("script").and_then(Value::as_str)
    } else {
        None
    }
}

/// "mcp__linker__create_timeline" → "create_timeline"; other tools keep their name.
pub fn tool_label(name: &str) -> &str {
    if name.starts_with(RESOLVE_PREFIX) || name.starts_with(LINKER_PREFIX) {
        // The second "__", after "mcp__".
        name[5..]
            .find("__")
            .map_or(name, |index| &name[5 + index + 2..])
    } else {
        name
    }
}

/// The one-line detail shown after a tool's label.
pub fn tool_summary(name: &str, input: &Value) -> String {
    let Value::Object(record) = input else {
        return String::new();
    };
    if let Some(script) = script(name, input) {
        return script
            .split('\n')
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with('#'))
            .unwrap_or_default()
            .to_string();
    }
    if is_resolve_tool(name) {
        let text = input.to_string();
        return if text == "{}" { String::new() } else { text };
    }
    [
        "command",
        "file_path",
        "path",
        "pattern",
        "query",
        "url",
        "description",
    ]
    .iter()
    .find_map(|key| record.get(*key).and_then(Value::as_str))
    .unwrap_or_default()
    .to_string()
}

/// What an approval shows: a script as code, anything else as JSON.
pub fn approval_detail(name: &str, input: &Value) -> String {
    script(name, input).map_or_else(|| format_json(input), str::to_string)
}

/// A string as it is, anything else as indented JSON (`JSON.stringify(value, null, 2)`).
pub fn format_json(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    }
}

/// `"AI - Cut": 2 clips added` → the timeline and what happened to it, as written by describeChanges.
pub fn split_change(change: &str) -> (Option<&str>, &str) {
    if let Some(rest) = change.strip_prefix('"') {
        // The last `": ` wins, like the greedy `(.*)` of the TS regex.
        if let Some(index) = rest.rfind("\": ") {
            return (Some(&rest[..index]), &rest[index + 3..]);
        }
    }
    (None, change)
}

/// "Today", "Yesterday", or "Tue, Nov 18".
pub fn day_label(date: NaiveDate, today: NaiveDate) -> String {
    match (today - date).num_days() {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        _ => format!(
            "{}, {} {}",
            date.format("%a"),
            date.format("%b"),
            date.day()
        ),
    }
}

pub fn library_label(kind: ResolveLinkKind) -> &'static str {
    match kind {
        ResolveLinkKind::Local => "Local database",
        ResolveLinkKind::Network => "Network database",
        ResolveLinkKind::Cloud => "Blackmagic Cloud",
    }
}

/// What workspaces and series are called, in the app and in the agent's rules. In code they are collections.
pub struct CollectionLabel {
    pub one: &'static str,
    pub many: &'static str,
    pub lower: &'static str,
    pub lower_many: &'static str,
}

pub fn collection_label(kind: CollectionKind) -> CollectionLabel {
    match kind {
        CollectionKind::Workspace => CollectionLabel {
            one: "Workspace",
            many: "Workspaces",
            lower: "workspace",
            lower_many: "workspaces",
        },
        CollectionKind::Series => CollectionLabel {
            one: "Series",
            many: "Series",
            lower: "series",
            lower_many: "series",
        },
    }
}

/// The settings section and the project settings field for them.
pub const COLLECTIONS_TITLE: &str = "Workspaces & series";
pub const COLLECTION_PICKER_LABEL: &str = "Workspace / series";
pub const NO_COLLECTION: &str = "No workspace";

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // labels.test.ts
    #[test]
    fn shows_new_rules_against_the_old_line_by_line_and_in_order() {
        let diff = line_diff(
            "Captions in white.\nEnd on the logo.\nNo swearing.\n",
            "Captions in white.\nIntro at most 3 seconds.\nNo swearing.",
        );
        let shown: Vec<String> = diff
            .iter()
            .map(|line| format!("{:?}: {}", line.kind, line.text))
            .collect();
        assert_eq!(
            shown,
            [
                "Same: Captions in white.",
                "Removed: End on the logo.",
                "Added: Intro at most 3 seconds.",
                "Same: No swearing.",
            ]
        );
        // A first brief is all new.
        assert_eq!(
            line_diff("", "One.\nTwo."),
            [
                DiffLine {
                    kind: LineKind::Added,
                    text: "One.".into()
                },
                DiffLine {
                    kind: LineKind::Added,
                    text: "Two.".into()
                },
            ]
        );
    }

    #[test]
    fn reads_what_save_brief_and_add_lesson_ask_to_change_and_nothing_else_as_a_brief() {
        assert_eq!(
            brief_request(
                "mcp__linker__add_lesson",
                &json!({ "for": "Series \"Reels\"", "feedback": "the start drags", "rule": "Intro at most 3 seconds." })
            ),
            Some(BriefRequest::Lesson {
                for_: "Series \"Reels\"".into(),
                feedback: "the start drags".into(),
                rule: "Intro at most 3 seconds.".into(),
            })
        );
        assert!(matches!(
            brief_request("mcp__linker__save_brief", &json!({ "for": "W", "rules": "x" })),
            Some(BriefRequest::Rules { was, rules, .. }) if was.is_empty() && rules == "x"
        ));
        assert_eq!(brief_request("mcp__linker__add_clips", &json!({})), None);
        // A brief tool isn't a Resolve tool, and keeps its short name.
        assert_eq!(
            [
                is_resolve_tool("mcp__linker__add_clips"),
                is_resolve_tool("mcp__linker__save_brief")
            ],
            [true, false]
        );
        assert_eq!(tool_label("mcp__linker__study_media"), "study_media");
    }

    #[test]
    fn summarizes_a_tool_in_one_line() {
        // A script shows its first line of code, not its comments.
        let script =
            json!({ "script": "# Check\n\nresult = resolve.GetVersionString()\nprint(1)" });
        assert_eq!(
            tool_summary("mcp__resolve__run_script", &script),
            "result = resolve.GetVersionString()"
        );
        assert_eq!(
            approval_detail("mcp__resolve__run_script", &script),
            script["script"]
        );
        // Resolve tools show their input as compact JSON, in the agent's order; others one field.
        assert_eq!(
            tool_summary(
                "mcp__linker__add_clips",
                &json!({ "timeline": "AI - Cut", "at": 2 })
            ),
            r#"{"timeline":"AI - Cut","at":2}"#
        );
        assert_eq!(tool_summary("mcp__linker__list_timelines", &json!({})), "");
        assert_eq!(
            tool_summary("Bash", &json!({ "description": "d", "command": "ls" })),
            "ls"
        );
        assert_eq!(tool_summary("Bash", &json!("ls")), "");
        assert_eq!(tool_label("Bash"), "Bash");
        assert_eq!(
            approval_detail("Write", &json!({ "file_path": "a.txt" })),
            "{\n  \"file_path\": \"a.txt\"\n}"
        );
    }

    #[test]
    fn splits_a_change_into_its_timeline_and_what_happened() {
        assert_eq!(
            split_change("\"AI - Cut\": 2 clips added"),
            (Some("AI - Cut"), "2 clips added")
        );
        assert_eq!(split_change("Media added"), (None, "Media added"));
    }

    #[test]
    fn names_the_day() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 18).unwrap();
        assert_eq!(day_label(today, today), "Today");
        assert_eq!(day_label(today.pred_opt().unwrap(), today), "Yesterday");
        assert_eq!(
            day_label(NaiveDate::from_ymd_opt(2026, 11, 10).unwrap(), today),
            "Tue, Nov 10"
        );
    }
}
