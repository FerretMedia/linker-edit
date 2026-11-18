// Resolve tools can return thousands of timeline items or transcript lines in one call. All of it
// would stay in the agent's context for the rest of the chat, so long results are cut first.

use serde_json::{Map, Value};

use super::types::{js_len, js_slice};

/// About 5,000 tokens.
pub const MAX_TOOL_OUTPUT_CHARS: usize = 20_000;

const ITEM_LIMITS: [usize; 5] = [50, 20, 10, 5, 2];

const NOTE: &str = "Linker-Edit shortened this result to save context. Ask again for a narrower part (one track, a range, a filter) to see what was left out.";

/// A JSON result with its long lists capped, or a plain cut when it isn't JSON; None when it fits.
pub fn shorten_text(text: &str, max: usize) -> Option<String> {
    if js_len(text) <= max {
        return None;
    }
    let Ok(parsed) = serde_json::from_str::<Value>(text) else {
        return Some(cut(text, max));
    };
    for limit in ITEM_LIMITS {
        // Compact JSON: indentation would cost tokens too.
        let shortened = cap_lists(&parsed, limit).to_string();
        if js_len(&shortened) <= max {
            return Some(format!("{shortened}\n{NOTE}"));
        }
    }
    Some(cut(
        &cap_lists(&parsed, ITEM_LIMITS[ITEM_LIMITS.len() - 1]).to_string(),
        max,
    ))
}

fn cut(text: &str, max: usize) -> String {
    format!(
        "{}\n… {} more characters left out. {NOTE}",
        js_slice(text, max),
        js_len(text) - max
    )
}

fn cap_lists(value: &Value, limit: usize) -> Value {
    match value {
        Value::Array(items) => {
            let mut kept: Vec<Value> = items
                .iter()
                .take(limit)
                .map(|item| cap_lists(item, limit))
                .collect();
            if items.len() > limit {
                kept.push(Value::String(format!(
                    "… {} more items left out",
                    items.len() - limit
                )));
            }
            Value::Array(kept)
        }
        Value::Object(entries) => Value::Object(
            entries
                .iter()
                .map(|(key, item)| (key.clone(), cap_lists(item, limit)))
                .collect(),
        ),
        _ => value.clone(),
    }
}

/// A PostToolUse hook's replacement for an MCP result, or None to leave it alone. Claude passes
/// MCP results as an array of content blocks; the object form is MCP's own CallToolResult.
pub fn shorten_tool_output(output: &Value, max: usize) -> Option<Value> {
    let blocks = match output {
        Value::Array(blocks) => blocks,
        Value::Object(result) => result.get("content")?.as_array()?,
        _ => return None,
    };
    let mut changed = false;
    let shortened: Vec<Value> = blocks
        .iter()
        .map(|block| {
            let text = (block.get("type").and_then(Value::as_str) == Some("text"))
                .then(|| block.get("text").and_then(Value::as_str))
                .flatten()
                .and_then(|text| shorten_text(text, max));
            match text {
                Some(text) => {
                    changed = true;
                    let mut block = block.clone();
                    block["text"] = Value::String(text);
                    block
                }
                None => block.clone(),
            }
        })
        .collect();
    if !changed {
        return None;
    }
    match output {
        Value::Array(_) => Some(Value::Array(shortened)),
        Value::Object(result) => {
            // structuredContent repeats the full result; the shortened text stands in for it.
            let mut rest: Map<String, Value> = result
                .iter()
                .filter(|(key, _)| *key != "structuredContent")
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
            rest.insert("content".into(), Value::Array(shortened));
            Some(Value::Object(rest))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn clips(count: usize) -> String {
        let items: Vec<Value> = (0..count)
            .map(|i| json!({ "name": format!("clip {i}") }))
            .collect();
        json!({ "project": "Doc", "items": items }).to_string()
    }

    #[test]
    fn leaves_a_result_that_fits_alone() {
        assert_eq!(shorten_text(&clips(3), 1000), None);
    }

    #[test]
    fn caps_long_lists_in_json_keeping_the_rest_and_saying_how_much_was_left_out() {
        let text = shorten_text(&clips(500), 2000).unwrap();
        assert!(text.len() <= 2000 + 200);
        assert!(text.contains(r#""project":"Doc""#));
        assert!(text.contains(r#""name":"clip 0""#));
        assert!(text.contains(" more items left out"));
        assert!(text.contains("Ask again for a narrower part"));
    }

    #[test]
    fn cuts_text_that_isnt_json_or_json_whose_size_is_one_long_string_at_the_limit() {
        for input in [
            "x".repeat(5000),
            json!({ "transcript": "word ".repeat(1000) }).to_string(),
        ] {
            let text = shorten_text(&input, 1000).unwrap();
            assert!(text.starts_with(&input[..1000]));
            assert!(text.contains(&format!("{} more characters left out", input.len() - 1000)));
        }
    }

    #[test]
    fn shortens_the_text_blocks_of_claudes_mcp_result_array_and_keeps_the_others() {
        let image = json!({ "type": "image", "data": "abc" });
        let output = shorten_tool_output(
            &json!([{ "type": "text", "text": clips(500) }, image]),
            2000,
        )
        .unwrap();
        assert!(
            output[0]["text"]
                .as_str()
                .unwrap()
                .contains("more items left out")
        );
        assert_eq!(output[1], image);
    }

    #[test]
    fn drops_structured_content_from_an_mcp_call_tool_result_since_it_repeats_the_full_result() {
        let output = shorten_tool_output(
            &json!({ "content": [{ "type": "text", "text": clips(500) }], "structuredContent": { "big": true }, "isError": false }),
            2000,
        )
        .unwrap();
        assert_eq!(output["isError"], json!(false));
        assert_eq!(output["content"][0]["type"], json!("text"));
        assert!(output.get("structuredContent").is_none());
    }

    #[test]
    fn returns_none_for_a_result_that_fits_or_an_unknown_shape_leaving_it_alone() {
        assert_eq!(
            shorten_tool_output(&json!([{ "type": "text", "text": "{}" }]), 2000),
            None
        );
        assert_eq!(shorten_tool_output(&json!("plain string"), 2000), None);
    }
}
