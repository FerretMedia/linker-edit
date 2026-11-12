import * as fs from "node:fs";
import * as path from "node:path";
import type { AppConfig } from "../config.ts";
import { VERSION } from "../updates.ts";
import { apiEnv } from "./api.ts";

const SERVER_DIR = path.join(import.meta.dirname, "..");

// In the npm package this module is bundled into dist/, next to the helper processes' own bundles.
function script(bundled: string, source: string): string {
  const file = path.join(import.meta.dirname, bundled);
  return fs.existsSync(file) ? file : path.join(SERVER_DIR, source);
}

// In the desktop app, process.execPath is the app itself, which runs scripts only with this set. Codex
// passes MCP servers just the environment it's given, so every command carries it.
const runAsNode: Record<string, string> = process.versions.electron ? { ELECTRON_RUN_AS_NODE: "1" } : {};

/** How agents start the "linker" MCP server. */
export function linkerMcpCommand(config: AppConfig) {
  return {
    command: process.execPath,
    args: [script("linker-mcp.js", path.join("agents", "linkerMcp.ts"))],
    env: { ...runAsNode, ...apiEnv(config.port, config.apiToken), LINKER_EDIT_VERSION: VERSION },
  };
}
