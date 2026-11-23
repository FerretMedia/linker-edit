// Starts DaVinci Resolve and waits until it answers scripts. Resolve's script server quits when a client
// connects before Resolve itself did; Resolve retries once, and after that never answers scripts until it's
// restarted. So nothing connects until the script server has run for a moment. That's also why this doesn't
// use ResolveMCP's launch_resolve, which keeps connecting while Resolve boots.
import { ResolveError, type ResolveConnection } from "./connection.ts";
import { MISSING_RESOLVE_MCP } from "./locate.ts";
import { launchResolve, resolveProcesses, type ResolveProcesses } from "./program.ts";

const POLL_MS = 1000;
/** How long the script server runs before something may connect to it. */
const SETTLE_MS = 2000;
/** Then something connects anyway: the script server's name is unverified on macOS and Linux. */
const SCRIPT_SERVER_TIMEOUT_MS = 90_000;
/** A slow computer or a big database needs long. */
const START_TIMEOUT_MS = 3 * 60_000;
/** A Resolve that was up already should answer soon, unless a dialog is up. */
const UP_TIMEOUT_MS = 30_000;

export interface StartOptions {
  /** Aborts when the user stops the turn. Resolve goes on starting; only the waiting stops. */
  signal?: AbortSignal;
  /** This computer's processes and how to start Resolve; tests pass their own. */
  processes?: () => Promise<ResolveProcesses>;
  launch?: (resolveMcp: string) => Promise<void>;
}

const answers = (connection: ResolveConnection) =>
  connection.runScript<{ state?: string } | null>("status", {}, 10).then(
    (probe) => probe?.state === "connected",
    () => false,
  );

/** Starts Resolve when it isn't running, and waits until it answers. True when this started it. */
export async function startResolve(connection: ResolveConnection, options: StartOptions = {}): Promise<boolean> {
  const { signal, processes = resolveProcesses, launch = launchResolve } = options;
  const { command } = connection;
  if (!command) throw new ResolveError("unavailable", MISSING_RESOLVE_MCP);
  const stopped = new Promise<never>((_resolve, reject) => {
    const stop = () => reject(new ResolveError("unavailable", "Stopped while waiting for Resolve."));
    if (signal?.aborted) stop();
    signal?.addEventListener("abort", stop, { once: true });
  });
  stopped.catch(() => {});
  const unlessStopped = <T>(work: Promise<T>) => Promise.race([stopped, work]);
  const pause = () => unlessStopped(new Promise((resolve) => setTimeout(resolve, POLL_MS)));

  // When this look fails, it ends here: Resolve may be running, and a second one must never start.
  const first = await unlessStopped(processes());
  if (!first.resolve) await unlessStopped(launch(command));

  // With both there at the first look, the script server may have appeared just now, but nearly always it's a
  // Resolve that has been up for long, and Open in Resolve shouldn't wait on that chance every time.
  const up = first.resolve && first.scriptServer;
  const begun = Date.now();
  if (!up) {
    let seenAt: number | null = null;
    while (Date.now() - begun < SCRIPT_SERVER_TIMEOUT_MS) {
      const scriptServer = await unlessStopped(processes().then((seen) => seen.scriptServer, () => false));
      seenAt = scriptServer ? (seenAt ?? Date.now()) : null;
      if (seenAt !== null && Date.now() - seenAt >= SETTLE_MS) break;
      await pause();
    }
  }

  const deadline = begun + (up ? UP_TIMEOUT_MS : START_TIMEOUT_MS);
  while (!(await unlessStopped(answers(connection)))) {
    if (Date.now() >= deadline) {
      throw new ResolveError(
        "unavailable",
        `${first.resolve ? "Resolve is running but doesn't answer scripts" : "Resolve was started but didn't get ready in time"}. Close any dialog in Resolve and try again; if that doesn't help, save your work and restart Resolve.`,
      );
    }
    await pause();
  }
  return !first.resolve;
}
