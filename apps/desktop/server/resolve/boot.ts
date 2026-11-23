// Keeps Linker-Edit from connecting to Resolve while Resolve boots. Resolve's script server starts late in the
// boot and quits when another client connects before Resolve itself did; Resolve retries once, and after that
// never answers scripts until it's restarted.
import { ResolveError } from "./connection.ts";
import { resolveProcesses, type ResolveProcesses } from "./program.ts";

const SETTLE_MS = 2000;
// The script server's name is unverified on macOS and Linux.
const CONNECT_ANYWAY_MS = 90_000;
const NO_ANSWER = /DaVinci Resolve is not running|Script timed out/;

export function createBootGuard(processes: () => Promise<ResolveProcesses> = resolveProcesses) {
  // While Resolve answers, nothing looks at processes: on Windows a look takes half a second.
  let answered = false;
  let withoutScriptServerSince: number | null = null;

  async function look(): Promise<ResolveProcesses | null> {
    const seen = await processes().catch(() => null);
    if (seen) withoutScriptServerSince = seen.resolve && !seen.scriptServer ? (withoutScriptServerSince ?? Date.now()) : null;
    return seen;
  }

  async function mayConnect(): Promise<boolean> {
    if (answered) return true;
    const seen = await look();
    if (seen?.resolve && seen.scriptServer) {
      await new Promise((resolve) => setTimeout(resolve, SETTLE_MS));
      await look();
    }
    return withoutScriptServerSince === null || Date.now() - withoutScriptServerSince >= CONNECT_ANYWAY_MS;
  }

  return {
    /** Runs a call to one of ResolveMCP's tools. While Resolve boots, it answers as if Resolve isn't running instead. */
    async call(tool: string, connect: () => Promise<string>): Promise<string> {
      if (!(await mayConnect())) {
        if (tool === "get_resolve_status") return JSON.stringify({ running: false });
        throw new ResolveError("error", "DaVinci Resolve isn't running.");
      }
      answered = false;
      const text = await connect();
      answered = tool === "get_resolve_status" ? /"running":\s*true/.test(text) : tool === "run_script" && !NO_ANSWER.test(text);
      return text;
    },
    /** The MCP server that got the answers is gone. */
    forget() {
      answered = false;
    },
  };
}
