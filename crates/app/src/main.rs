//! linker-edit: the GPUI window with its server, `serve` for the browser only, `mcp` for the agents' tools.

use linker_edit_ui::transport::{self, Auth};

fn main() -> anyhow::Result<()> {
    // `linker-edit mcp`: the agents' "linker" MCP server on stdio (agents::commands::linker_mcp_command).
    if std::env::args().nth(1).as_deref() == Some("mcp") {
        use linker_edit_server::agents::linker_mcp;
        return linker_mcp::run_linker_mcp(linker_mcp::catalog());
    }
    // Dev only, until crates/server runs here: LINKER_EDIT_SERVER_URL=http://127.0.0.1:4521 opens
    // the window on a server that is already running (such as the TypeScript one, which speaks
    // the same /ws). LINKER_EDIT_SERVER_TOKEN, when set, is a Linker Connect session sent as
    // `Authorization: Bearer`; without it the server must be on this computer.
    if let Ok(url) = std::env::var("LINKER_EDIT_SERVER_URL") {
        let auth = std::env::var("LINKER_EDIT_SERVER_TOKEN").map_or(Auth::Local, Auth::Bearer);
        return linker_edit_ui::run(transport::connect(&url, auth)?);
    }
    // Otherwise the window gets a link nobody answers; it shows "Connecting…" and waits. The
    // server's end stays alive so the window doesn't see the link close.
    let (link, _server_end) = linker_edit_ui::Link::pair();
    linker_edit_ui::run(link)
}
