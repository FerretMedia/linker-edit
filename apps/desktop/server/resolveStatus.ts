import type { ResolveStatus } from "../shared/protocol.ts";
import { ResolveError, type ResolveConnection } from "./resolve/connection.ts";
import { MISSING_RESOLVE_MCP } from "./resolve/locate.ts";

const BLOCKED_HINT =
  "Resolve answers but refuses edits while the Project Manager or a dialog, such as an update prompt, is open. Open a project and close the dialog in Resolve, then refresh.";

// Seen on Studio 21.1.2 after its update prompt: scripting stayed dead after the prompt closed, until
// Resolve restarted.
const STUCK_HINT =
  "Resolve is running but doesn't answer scripts. An update prompt or another dialog usually causes this, and Resolve can stay stuck after it closes. Close any dialog in Resolve and refresh. If that doesn't help, save your work and restart Resolve.";

const unreachable = (detail: string): ResolveStatus => ({
  state: "unreachable",
  hint: `Resolve is running but doesn't answer Linker-Edit (${detail}). Restart Resolve, then refresh.`,
});

/** What status.py returns. */
type Probe = { state: string; page?: string | null } & Record<string, unknown>;

export function statusFromProbe(probe: Probe | null): ResolveStatus {
  if (probe?.state !== "connected") return unreachable("Resolve's scripting isn't reachable");
  const { page, ...status } = probe;
  // No page means Resolve is on the Project Manager or behind a modal dialog.
  if (page === null || page === undefined) return { state: "blocked", hint: BLOCKED_HINT };
  return { projectId: null, database: null, ...status } as unknown as ResolveStatus;
}

export async function checkResolve(connection: ResolveConnection): Promise<ResolveStatus> {
  if (!connection.command) return { state: "setup_needed", hint: MISSING_RESOLVE_MCP };
  let running: boolean;
  try {
    running = (JSON.parse(await connection.callTool("get_resolve_status", {}, 15_000)) as { running?: boolean }).running === true;
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    // An older Resolve's server can start but lack what Linker-Edit needs.
    if (/no run_script/.test(message)) return { state: "setup_needed", hint: message };
    return { state: "unreachable", hint: `Linker-Edit can't use Resolve's MCP server: ${message}` };
  }
  if (!running) return { state: "not_running" };
  try {
    return statusFromProbe(await connection.runScript<Probe>("status", {}, 10));
  } catch (error) {
    if (error instanceof ResolveError && (error.kind === "timeout" || error.kind === "unavailable")) {
      return { state: "blocked", hint: STUCK_HINT };
    }
    return unreachable(error instanceof Error ? error.message : String(error));
  }
}
