import { spawn } from "node:child_process";
import * as fs from "node:fs";
import * as path from "node:path";
import type { ResolveMcpConfig } from "./config.ts";

/** The davinci-resolve-mcp release Linker-Edit is tested against. Update it deliberately. */
export const RESOLVE_MCP_VERSION = "v4.17.0";
const RESOLVE_MCP_REPO = "https://github.com/samuelgursky/davinci-resolve-mcp";

const INSTALL_HINTS: Record<string, string> = {
  git: "Install Git from https://git-scm.com, then run Linker-Edit again.",
  uv: "Install uv from https://docs.astral.sh/uv/, then run Linker-Edit again.",
};

export function isResolveMcpInstalled(mcp: ResolveMcpConfig): boolean {
  return fs.existsSync(mcp.python) && fs.existsSync(mcp.serverScript);
}

function run(command: string, args: string[], cwd?: string): Promise<void> {
  return new Promise((resolve, reject) => {
    spawn(command, args, { cwd, stdio: "inherit", windowsHide: true })
      .on("error", (error: NodeJS.ErrnoException) =>
        reject(new Error(error.code === "ENOENT" ? `${command} not found. ${INSTALL_HINTS[command] ?? ""}` : error.message)),
      )
      .on("exit", (code) => (code === 0 ? resolve() : reject(new Error(`${command} ${args.join(" ")} failed (exit ${code}).`))));
  });
}

/** Clones the pinned davinci-resolve-mcp and builds its venv with uv. True when it's ready to run. */
export async function setupResolveMcp(mcp: ResolveMcpConfig): Promise<boolean> {
  if (isResolveMcpInstalled(mcp)) return true;
  // serverScript is <dir>/src/server.py.
  const dir = path.dirname(path.dirname(mcp.serverScript));
  console.log(`Setting up davinci-resolve-mcp ${RESOLVE_MCP_VERSION} in ${dir}`);
  try {
    if (!fs.existsSync(mcp.serverScript)) {
      await run("git", ["clone", "--depth", "1", "--branch", RESOLVE_MCP_VERSION, RESOLVE_MCP_REPO, dir]);
    }
    if (!fs.existsSync(mcp.python)) {
      await run("uv", ["venv", "--python", "3.12", ".venv"], dir);
      await run("uv", ["pip", "install", "-r", "requirements.txt"], dir);
    }
  } catch (error) {
    console.error(`davinci-resolve-mcp setup failed: ${error instanceof Error ? error.message : String(error)}`);
    return false;
  }
  return isResolveMcpInstalled(mcp);
}
