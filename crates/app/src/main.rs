//! linker-edit: the GPUI window with its server, `serve` for the browser only, `mcp` for the agents' tools.

fn main() -> anyhow::Result<()> {
    // Until crates/server runs here, the window gets a link nobody answers; it shows the layout
    // and waits. The server's end stays alive so the window doesn't see the link close.
    let (link, _server_end) = linker_edit_ui::Link::pair();
    linker_edit_ui::run(link)
}
