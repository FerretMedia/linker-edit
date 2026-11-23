// Starts DaVinci Resolve with its own launch_resolve tool and waits until it answers scripts.
import { createResolveConnection, ResolveError, type ResolveConnection } from "./connection.ts";
import { MISSING_RESOLVE_MCP } from "./locate.ts";

/** launch_resolve itself gives up after 60 seconds; a slow computer or a big database needs longer. */
const LAUNCH_TIMEOUT_MS = 3 * 60_000;
/** A Resolve that was running already should answer soon, unless it's still starting or a dialog is up. */
const RUNNING_TIMEOUT_MS = 30_000;

export interface StartOptions {
  /** Aborts when the user stops the turn. Resolve goes on starting; only the waiting stops. */
  signal?: AbortSignal;
  pollMs?: number;
  launchTimeoutMs?: number;
  runningTimeoutMs?: number;
  /** The connection that launches Resolve; tests pass their own. */
  launcher?: () => ResolveConnection;
}

export async function resolveRunning(connection: ResolveConnection): Promise<boolean> {
  return (JSON.parse(await connection.callTool("get_resolve_status", {}, 15_000)) as { running?: boolean }).running === true;
}

const answers = (connection: ResolveConnection) =>
  connection.runScript<{ state?: string } | null>("status", {}, 10).then(
    (probe) => probe?.state === "connected",
    () => false,
  );

/** Resolves when the signal aborts, or never. */
const aborted = (signal?: AbortSignal) =>
  new Promise<void>((resolve) => {
    if (signal?.aborted) resolve();
    signal?.addEventListener("abort", () => resolve(), { once: true });
  });

/** Starts Resolve when it isn't running, and waits until it answers. True when this started it. */
export async function startResolve(connection: ResolveConnection, options: StartOptions = {}): Promise<boolean> {
  const { signal, pollMs = 2000, launchTimeoutMs = LAUNCH_TIMEOUT_MS, runningTimeoutMs = RUNNING_TIMEOUT_MS } = options;
  const { command } = connection;
  if (!command) throw new ResolveError("unavailable", MISSING_RESOLVE_MCP);
  const stopped = aborted(signal);
  const waitUntilAnswering = async (deadline: number): Promise<boolean> => {
    while (!signal?.aborted) {
      if (await answers(connection)) return true;
      if (Date.now() >= deadline) return false;
      await Promise.race([stopped, new Promise((resolve) => setTimeout(resolve, pollMs))]);
    }
    throw new ResolveError("unavailable", "Stopped while waiting for Resolve.");
  };

  if (await resolveRunning(connection)) {
    if (await waitUntilAnswering(Date.now() + runningTimeoutMs)) return false;
    throw new ResolveError(
      "unavailable",
      "Resolve is running but doesn't answer. Close any dialog in Resolve and try again; if that doesn't help, restart Resolve.",
    );
  }

  const deadline = Date.now() + launchTimeoutMs;
  // Through a server of its own that ends once Resolve is up: quitting Linker-Edit stops every process it
  // started, down the tree, and must never take Resolve and the user's unsaved work with it.
  const launcher = options.launcher?.() ?? createResolveConnection({ command, ownGroup: true });
  let problem = "";
  try {
    const launched = launcher.callTool("launch_resolve", {}, 90_000).catch((error: unknown) => {
      problem = error instanceof Error ? error.message : String(error);
    });
    await Promise.race([stopped, launched]);
    if (await waitUntilAnswering(deadline)) return true;
  } finally {
    launcher.close();
  }
  throw new ResolveError("unavailable", `Resolve didn't start in time${problem ? ` (${problem})` : ""}. Is a dialog open in Resolve?`);
}
