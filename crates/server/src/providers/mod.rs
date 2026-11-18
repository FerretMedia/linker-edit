//! Claude and Codex sessions, and replaying a recorded turn (apps/desktop/server/providers). There is no Rust
//! Agent SDK or Codex SDK: claude_cli and codex_cli drive the two CLIs the way those SDKs did.

pub mod claude;
pub mod claude_cli;
pub mod cli;
pub mod codex;
pub mod codex_cli;
pub mod replay;
pub mod shorten;
#[cfg(test)]
pub mod testing;
pub mod types;
