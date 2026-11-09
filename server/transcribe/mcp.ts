import * as fs from "node:fs";
import * as path from "node:path";
import type { AppConfig } from "../config.ts";

// In the npm package this module is bundled into dist/, next to the MCP server's own bundle.
function mcpServerScript(): string {
  const bundled = path.join(import.meta.dirname, "linker-mcp.js");
  return fs.existsSync(bundled) ? bundled : path.join(import.meta.dirname, "mcpServer.ts");
}

/** How agents start the "linker" MCP server. */
export function linkerMcpCommand(config: AppConfig) {
  return {
    command: process.execPath,
    args: [mcpServerScript()],
    env: { LINKER_EDIT_API: `http://127.0.0.1:${config.port}`, LINKER_EDIT_TOKEN: config.apiToken },
  };
}
