// Keeps Linker-Edit from connecting to Resolve while Resolve boots. Resolve's script server starts late in the
// boot and quits when another client connects before Resolve itself did; Resolve retries once, and after that
// never answers scripts until it's restarted.
import { ResolveError } from "./connection.ts";
import { isAlive, resolveProcesses, type ResolveProcesses } from "./program.ts";

const SETTLE_MS = 2000;
// The script server's name is unverified on macOS and Linux.
const CONNECT_ANYWAY_MS = 90_000;
const NO_ANSWER = /DaVinci Resolve is not running|Script timed out/;

/** Ways to see this computer's processes; tests give their own. */
export interface BootOptions {
  processes?: () => Promise<ResolveProcesses>;
  alive?: (pid: number) => boolean;
}

export function createBootGuard({ processes = resolveProcesses, alive = isAlive }: BootOptions = {}) {
  let resolve: number | null = null;
  // The Resolve that gave the last answer. While it lives, nothing looks at processes: on Windows that takes half
  // a second.
  let answeredBy: number | null = null;
  let booting: { resolve: number; since: number } | null = null;

  async function look(): Promise<ResolveProcesses | null> {
    const seen = await processes().catch(() => null);
    resolve = seen?.resolve ?? null;
    if (seen && (seen.scriptServer || !seen.resolve)) booting = null;
    if (seen?.resolve && !seen.scriptServer && booting?.resolve !== seen.resolve) booting = { resolve: seen.resolve, since: Date.now() };
    return seen;
  }

  async function mayConnect(): Promise<boolean> {
    if (answeredBy !== null && alive(answeredBy)) return true;
    const seen = await look();
    if (seen?.resolve && seen.scriptServer) {
      await new Promise((done) => setTimeout(done, SETTLE_MS));
      await look();
    }
    return booting === null || Date.now() - booting.since >= CONNECT_ANYWAY_MS;
  }

  return {
    /** Runs a call to one of ResolveMCP's tools. While Resolve boots, it answers as if Resolve isn't running instead. */
    async call(tool: string, connect: () => Promise<string>): Promise<string> {
      if (!(await mayConnect())) {
        if (tool === "get_resolve_status") return JSON.stringify({ running: false });
        throw new ResolveError("error", "DaVinci Resolve isn't running.");
      }
      answeredBy = null;
      const text = await connect();
      const answered = tool === "get_resolve_status" ? /"running":\s*true/.test(text) : tool === "run_script" && !NO_ANSWER.test(text);
      if (answered) answeredBy = resolve;
      return text;
    },
    /** The MCP server that got the answers is gone. */
    forget() {
      answeredBy = null;
    },
  };
}
