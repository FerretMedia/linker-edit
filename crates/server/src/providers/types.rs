// What every provider session has: the sink a turn writes into, the session itself, and what they need from
// the policy (policy.ts) and the rules (rules.ts), which the Resolve port owns.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use linker_edit_protocol::{ApprovalMode, ContextUsage, Tone, ToolStatus};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

/// A boxed future that can move between threads, as callbacks into a running turn return.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// ChatItem["kind"].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    User,
    Assistant,
    Tool,
    Notice,
}

/// Partial update for a chat item; the server merges it into the stored item by id.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemPatch {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<ItemKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tone: Option<Tone>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<ToolStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}

impl ItemPatch {
    /// The patch merged over an earlier one, as `{ ...earlier, ...patch }`.
    pub fn merged_over(self, earlier: ItemPatch) -> ItemPatch {
        ItemPatch {
            id: self.id,
            kind: self.kind.or(earlier.kind),
            text: self.text.or(earlier.text),
            tone: self.tone.or(earlier.tone),
            name: self.name.or(earlier.name),
            input: self.input.or(earlier.input),
            status: self.status.or(earlier.status),
            output: self.output.or(earlier.output),
        }
    }
}

#[async_trait]
pub trait TurnSink: Send + Sync {
    fn upsert(&self, patch: ItemPatch);
    fn notice(&self, text: String, tone: Tone);
    /// True when the user allows the call.
    async fn request_approval(&self, tool: &str, input: &Value) -> bool;
    /// None when the size is no longer known, as after a compaction that didn't report it.
    fn context(&self, usage: Option<ContextUsage>);
    /// Backs the open Resolve project up, once per turn, before the first change to it. An error (its
    /// message) when that fails, and the change must not run then. Where there is no project to back up,
    /// the default does nothing, as the TypeScript's absent `backup` did.
    async fn backup(&self) -> Result<(), String> {
        Ok(())
    }
}

#[async_trait]
pub trait ProviderSession: Send + Sync {
    /// Runs one turn. An error after `signal` was cancelled is the stop, not a failure.
    async fn send(
        &self,
        text: &str,
        sink: Arc<dyn TurnSink>,
        signal: CancellationToken,
    ) -> anyhow::Result<()>;
    /// Whether `compact` does anything: false where the provider can't (the TypeScript's absent compact).
    fn can_compact(&self) -> bool {
        false
    }
    /// Summarizes the conversation so far to free context.
    async fn compact(
        &self,
        _sink: Arc<dyn TurnSink>,
        _signal: CancellationToken,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    /// The provider's id for this conversation, to continue it after a restart; None before the first turn.
    fn resume_id(&self) -> Option<String>;
}

/// policy.ts's ToolDecision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolDecision {
    Allow,
    Deny { message: String },
}

/// What a session needs from policy.ts, which the Resolve port owns: `refusal` and `decideToolCall`, the same
/// functions behind a trait so the hub can hand the real policy in.
#[async_trait]
pub trait ToolGate: Send + Sync {
    /// What Linker-Edit refuses whatever the mode; None when the call may go on.
    fn refusal(&self, tool_name: &str, input: &Value) -> Option<String>;
    /// The decision for one call; `ask` asks the user, and is awaited only when the mode and tool need it.
    async fn decide_tool_call(
        &self,
        tool_name: &str,
        input: &Value,
        mode: ApprovalMode,
        ask: BoxFuture<'_, bool>,
    ) -> ToolDecision;
}

// From rules.ts and policy.ts, which the Resolve port owns; the hub swaps these for `pub use` of its consts.

/// rules.ts's RESOLVE_MCP_NAME: Resolve's own MCP server, as agents see it.
pub const RESOLVE_MCP_NAME: &str = "resolve";
/// rules.ts's LINKER_MCP_NAME: Linker-Edit's own MCP server.
pub const LINKER_MCP_NAME: &str = "linker";
/// policy.ts's RESOLVE_RUN_SCRIPT.
pub const RESOLVE_RUN_SCRIPT: &str = "mcp__resolve__run_script";
/// policy.ts's CLAUDE_DISALLOWED_RESOLVE_TOOLS.
pub const CLAUDE_DISALLOWED_RESOLVE_TOOLS: &[&str] = &[
    "mcp__resolve__run_script_unsafe",
    "mcp__resolve__delete_dctl",
    "mcp__resolve__delete_lut",
];
/// policy.ts's CODEX_DISABLED_RESOLVE_TOOLS.
pub const CODEX_DISABLED_RESOLVE_TOOLS: &[&str] = &[
    "run_script",
    "run_script_unsafe",
    "update_dctl",
    "generate_lut",
    "delete_dctl",
    "delete_lut",
];

/// A string's length as JavaScript counts it, in UTF-16 code units.
pub fn js_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// The first `end` UTF-16 code units of a string, as JavaScript's `text.slice(0, end)`.
pub fn js_slice(text: &str, end: usize) -> String {
    let units: Vec<u16> = text.encode_utf16().take(end).collect();
    String::from_utf16_lossy(&units)
}

/// truncate's limit when the caller names none.
pub const TRUNCATE_MAX: usize = 4000;

pub fn truncate(text: &str, max: usize) -> String {
    let len = js_len(text);
    if len > max {
        format!("{}\n… ({} more characters)", js_slice(text, max), len - max)
    } else {
        text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_long_text_and_says_how_much_was_left_out() {
        assert_eq!(truncate("abcdef", 4), "abcd\n… (2 more characters)");
        assert_eq!(truncate("abc", 4), "abc");
    }
}
