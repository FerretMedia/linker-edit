// As in T3 Code's portable handoffs: a short text summary of the chat for a new agent session, not the other
// agent's real context.

use linker_edit_protocol::{ChatItem, ToolStatus};

use crate::history::history::resolve_write_action;
use crate::js;
use crate::policy::is_resolve_write;

const ITEM_CHARS: usize = 240;
pub const BUDGET_CHARS: usize = 12_000;

fn one_line(text: &str) -> String {
    let line = js::trim(text)
        .split(js::is_space)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if js::length(&line) > ITEM_CHARS {
        format!("{}...", js::slice(&line, ITEM_CHARS - 3))
    } else {
        line
    }
}

/// The chat so far as short lines, newest kept when it runs over the budget; empty when nothing was said.
pub fn handoff_summary(items: &[ChatItem], budget: usize) -> String {
    let lines: Vec<String> = items
        .iter()
        .filter_map(|item| match item {
            ChatItem::User { text, .. } => Some(format!("User: {}", one_line(text))),
            ChatItem::Assistant { text, .. } => Some(format!("Assistant: {}", one_line(text))),
            ChatItem::Tool {
                name,
                input,
                status: ToolStatus::Done,
                ..
            } => resolve_write_action(Some(name), input, is_resolve_write)
                .map(|action| format!("Resolve action: {action}")),
            _ => None,
        })
        .collect();
    if !lines.iter().any(|line| line.starts_with("User: ")) {
        return String::new();
    }
    let mut kept: Vec<&str> = Vec::new();
    let mut size = 0;
    for line in lines.iter().rev() {
        let length = js::length(line);
        if size + length + 1 > budget {
            kept.push("(earlier messages left out)");
            break;
        }
        kept.push(line);
        size += length + 1;
    }
    kept.reverse();
    kept.join("\n")
}

/// Puts the summary ahead of the first message to a new agent session in an ongoing chat.
pub fn with_handoff(summary: &str, text: &str) -> String {
    if summary.is_empty() {
        return text.to_string();
    }
    format!(
        "<earlier_conversation>
This chat started in another agent session whose context you don't have. A summary, oldest first, with long messages cut:
{summary}
</earlier_conversation>

{text}"
    )
}

#[cfg(test)]
mod tests {
    use linker_edit_protocol::Tone;
    use serde_json::json;

    use super::*;

    fn chat() -> Vec<ChatItem> {
        vec![
            ChatItem::User {
                id: "1".into(),
                text: "Cut the\n\nsilences".into(),
            },
            ChatItem::Tool {
                id: "2".into(),
                name: "mcp__linker__list_timelines".into(),
                input: json!({}),
                status: ToolStatus::Done,
                output: None,
            },
            ChatItem::Tool {
                id: "3".into(),
                name: "mcp__linker__create_timeline".into(),
                input: json!({ "name": "Cut" }),
                status: ToolStatus::Done,
                output: None,
            },
            ChatItem::Tool {
                id: "4".into(),
                name: "mcp__linker__remove_clips".into(),
                input: json!({ "item_ids": ["a"] }),
                status: ToolStatus::Error,
                output: None,
            },
            ChatItem::Notice {
                id: "5".into(),
                text: "Saved to edit history".into(),
                tone: Tone::Info,
            },
            ChatItem::Assistant {
                id: "6".into(),
                text: "a".repeat(300),
            },
        ]
    }

    #[test]
    fn keeps_what_was_said_and_done_in_resolve_one_short_line_each() {
        let summary = handoff_summary(&chat(), BUDGET_CHARS);
        assert_eq!(
            summary.split('\n').collect::<Vec<_>>(),
            [
                "User: Cut the silences".to_string(),
                r#"Resolve action: create_timeline {"name":"Cut"}"#.to_string(),
                format!("Assistant: {}...", "a".repeat(237)),
            ]
        );
    }

    #[test]
    fn keeps_the_newest_lines_when_the_chat_runs_over_the_budget() {
        let summary = handoff_summary(&chat(), 280);
        assert_eq!(
            summary.split('\n').collect::<Vec<_>>(),
            [
                "(earlier messages left out)".to_string(),
                format!("Assistant: {}...", "a".repeat(237)),
            ]
        );
    }

    #[test]
    fn is_empty_before_anyone_said_anything_and_then_adds_nothing_to_the_prompt() {
        let notice = ChatItem::Notice {
            id: "n".into(),
            text: "Stopped.".into(),
            tone: Tone::Info,
        };
        assert_eq!(handoff_summary(&[notice], BUDGET_CHARS), "");
        assert_eq!(with_handoff("", "hello"), "hello");
    }
}
