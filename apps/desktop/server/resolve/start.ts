// Starts DaVinci Resolve and waits until it answers scripts. The connection doesn't connect while Resolve boots
// (boot.ts); ResolveMCP's launch_resolve does, so this doesn't use it.
import { ResolveError, type ResolveConnection } from "./connection.ts";
import { MISSING_RESOLVE_MCP } from "./locate.ts";
import { launchResolve, resolveProcesses, type ResolveProcesses } from "./program.ts";

const POLL_MS = 1000;
const START_TIMEOUT_MS = 3 * 60_000;
const UP_TIMEOUT_MS = 30_000;

export interface StartOptions {
  signal?: AbortSignal;
  processes?: () => Promise<ResolveProcesses>;
  launch?: (resolveMcp: string) => Promise<void>;
}

const answers = (connection: ResolveConnection) =>
  connection.runScript<{ state?: string } | null>("status", {}, 10).then(
    (probe) => probe?.state === "connected",
    () => false,
  );

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

  const deadline = Date.now() + (first.resolve && first.scriptServer ? UP_TIMEOUT_MS : START_TIMEOUT_MS);
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
