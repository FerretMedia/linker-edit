//! linker-edit: the GPUI window with its server, `serve` for the browser only, `mcp` for the agents' tools.

fn main() -> anyhow::Result<()> {
    // `linker-edit mcp`: the agents' "linker" MCP server on stdio (agents::commands::linker_mcp_command).
    if std::env::args().nth(1).as_deref() == Some("mcp") {
        use linker_edit_server::agents::linker_mcp;
        return linker_mcp::run_linker_mcp(linker_mcp::catalog());
    }
    // Until crates/server runs here, the window gets a link nobody answers; it shows the layout
    // and waits. The server's end stays alive so the window doesn't see the link close.
    let (link, _server_end) = linker_edit_ui::Link::pair();
    linker_edit_ui::run(link)
}
