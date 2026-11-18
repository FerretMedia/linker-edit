// How agents start Linker-Edit's helper processes. The "linker" MCP server is this same program, run as
// `linker-edit mcp`, so it needs no Node and no script beside it.

use std::collections::BTreeMap;

use crate::config::AppConfig;
use crate::updates::VERSION;

use super::api::api_env;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpCommand {
    pub command: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
}

/// How agents start the "linker" MCP server. Codex passes MCP servers just the environment it's given, so
/// the command carries everything the server needs.
pub fn linker_mcp_command(config: &AppConfig) -> McpCommand {
    let command = std::env::current_exe()
        .map(|exe| exe.display().to_string())
        .unwrap_or_else(|_| "linker-edit".into());
    let mut env = api_env(config.port, &config.api_token);
    env.insert("LINKER_EDIT_VERSION".into(), VERSION.into());
    McpCommand {
        command,
        args: vec!["mcp".into()],
        env,
    }
}
