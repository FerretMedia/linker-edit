// The brief's tools on the agents' "linker" MCP server: how an agent changes a workspace's or series' rules, adds
// a lesson learned from feedback, and keeps its notes on example videos. The rules and lessons steer every later
// edit, so the user accepts or declines each change, in full access too.

use serde_json::{Value, json};

use super::collections::{SERIES_LABEL, WORKSPACE_LABEL};

pub const SAVE_BRIEF: &str = "save_brief";
pub const ADD_LESSON: &str = "add_lesson";
pub const SAVE_EXAMPLE: &str = "save_example";

/// The tools as MCP lists them: name, description, inputSchema and annotations.
pub fn brief_tools() -> Vec<Value> {
    let (workspace, series) = (WORKSPACE_LABEL.lower, SERIES_LABEL.lower);
    let with_where = |properties: Value| {
        let mut all = json!({
            "for": {
                "type": "string",
                "enum": ["workspace", "series"],
                "description": format!("Whose: the {workspace} (the brand) or a {series} (one format). Defaults to the most specific one this thread's project is in."),
            },
            "series": {
                "type": "string",
                "description": format!("With for \"series\": the name of a {series} in this {workspace}, when it isn't this project's own."),
            },
        });
        all.as_object_mut()
            .expect("an object")
            .extend(properties.as_object().expect("an object").clone());
        all
    };
    let annotations = json!({ "readOnlyHint": false, "destructiveHint": false });
    vec![
        json!({
            "name": SAVE_BRIEF,
            "description": format!("Propose new rules for the {workspace} or a {series}: the brief every edit in it follows. Pass the complete new text, not a patch: it replaces the rules. The user sees what changes and accepts or declines. Naming a {series} that doesn't exist makes it."),
            "inputSchema": {
                "type": "object",
                "properties": with_where(json!({
                    "rules": { "type": "string", "description": "The complete new rules, in Markdown." },
                    "summary": { "type": "string", "description": "One or two sentences on what changes and why, for the user." },
                })),
                "required": ["rules", "summary"],
            },
            "annotations": annotations,
        }),
        json!({
            "name": ADD_LESSON,
            "description": "Propose one lesson from the user's feedback: a short rule that holds for the next videos too. The user sees it next to their own words and accepts or declines. Only after the user has seen your fix and it is what they meant, or when they ask you to remember something.",
            "inputSchema": {
                "type": "object",
                "properties": with_where(json!({
                    "rule": { "type": "string", "description": "The rule, concrete enough to follow without this chat: \"Intro at most 3 seconds.\"" },
                    "feedback": { "type": "string", "description": "The user's own words this comes from, quoted, or what they changed in Resolve themselves." },
                })),
                "required": ["rule", "feedback"],
            },
            "annotations": annotations,
        }),
        json!({
            "name": SAVE_EXAMPLE,
            "description": "Keep your notes on one example video of this brand, as a Markdown file next to its rules: what it is, its numbers, its structure with times, what stands out. Saving again under the same name replaces them.",
            "inputSchema": {
                "type": "object",
                "properties": with_where(json!({
                    "name": { "type": "string", "description": "The video's file name or title." },
                    "notes": { "type": "string", "description": "Your notes, in Markdown." },
                })),
                "required": ["name", "notes"],
            },
            "annotations": annotations,
        }),
    ]
}

pub fn is_brief_tool(name: &str) -> bool {
    name == SAVE_BRIEF || name == ADD_LESSON || name == SAVE_EXAMPLE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_the_three_brief_tools_as_the_typescript_did() {
        let tools = brief_tools();
        let names: Vec<&str> = tools
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, [SAVE_BRIEF, ADD_LESSON, SAVE_EXAMPLE]);
        assert!(names.iter().all(|name| is_brief_tool(name)));
        assert!(!is_brief_tool("create_timeline"));
        assert_eq!(
            tools[0]["inputSchema"]["properties"]["series"]["description"],
            "With for \"series\": the name of a series in this workspace, when it isn't this project's own."
        );
        assert_eq!(
            tools[1]["inputSchema"]["required"],
            json!(["rule", "feedback"])
        );
    }
}
