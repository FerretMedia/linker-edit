// What every provider session has: the sink a turn writes into, the session itself, and what they need from
// the policy (policy.rs) and the rules (rules.rs).

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use linker_edit_protocol::{ApprovalMode, ContextUsage, Tone, ToolStatus};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::js;

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

pub use crate::policy::ToolDecision;

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

pub use crate::policy::{
    CLAUDE_DISALLOWED_RESOLVE_TOOLS, CODEX_DISABLED_RESOLVE_TOOLS, RESOLVE_RUN_SCRIPT,
};
pub use crate::rules::{LINKER_MCP_NAME, RESOLVE_MCP_NAME};

/// The real policy (policy.rs) as a ToolGate: what the hub hands every session.
pub struct Policy;

#[async_trait]
impl ToolGate for Policy {
    fn refusal(&self, tool_name: &str, input: &Value) -> Option<String> {
        crate::policy::refusal(tool_name, input)
    }

    async fn decide_tool_call(
        &self,
        tool_name: &str,
        input: &Value,
        mode: ApprovalMode,
        ask: BoxFuture<'_, bool>,
    ) -> ToolDecision {
        crate::policy::decide_tool_call(tool_name, input, mode, || ask).await
    }
}

/// truncate's limit when the caller names none.
pub const TRUNCATE_MAX: usize = 4000;

pub fn truncate(text: &str, max: usize) -> String {
    let len = js::length(text);
    if len > max {
        format!(
            "{}\n… ({} more characters)",
            js::slice(text, max),
            len - max
        )
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
