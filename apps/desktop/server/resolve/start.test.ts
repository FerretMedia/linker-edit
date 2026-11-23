import { describe, expect, it } from "vitest";
import type { ResolveConnection } from "./connection.ts";
import { startResolve, type StartOptions } from "./start.ts";
import { fakeConnection } from "./testing.ts";

/** A Resolve that answers scripts after `readyAfter` status checks; launching it can fail. */
function closedResolve({ running = false, readyAfter = 1, launchError = "" } = {}) {
  let checks = 0;
  const launches: string[] = [];
  let closed = 0;
  const { connection } = fakeConnection({
    tool: (name) => {
      if (name !== "get_resolve_status") throw new Error(`${name} must not go through the lasting connection.`);
      return JSON.stringify({ running });
    },
    script: () => {
      checks += 1;
      if (checks < readyAfter) throw new Error("DaVinci Resolve isn't running.");
      return { state: "connected", page: null };
    },
  });
  const launcher = (): ResolveConnection => ({
    ...connection,
    callTool: async (name) => {
      launches.push(name);
      if (launchError) throw new Error(launchError);
      return "DaVinci Resolve is running";
    },
    close: () => void (closed += 1),
  });
  const start = (options: StartOptions = {}) => startResolve(connection, { pollMs: 1, launcher, ...options });
  return { start, launches, closed: () => closed };
}

describe("startResolve", () => {
  it("launches Resolve through a connection of its own, and ends that one when Resolve answers", async () => {
    const { start, launches, closed } = closedResolve({ readyAfter: 3 });
    expect(await start()).toBe(true);
    expect(launches).toEqual(["launch_resolve"]);
    expect(closed()).toBe(1);
  });

  it("launches nothing when Resolve is running", async () => {
    const { start, launches } = closedResolve({ running: true });
    expect(await start()).toBe(false);
    expect(launches).toEqual([]);
  });

  it("goes on waiting when launch_resolve gives up before Resolve is ready", async () => {
    const { start } = closedResolve({ readyAfter: 3, launchError: "Timed out after 60s" });
    expect(await start()).toBe(true);
  });

  it("says why when Resolve doesn't come up, and when a running one doesn't answer", async () => {
    const never = closedResolve({ readyAfter: Infinity, launchError: "Timed out after 60s" });
    await expect(never.start({ launchTimeoutMs: 0 })).rejects.toThrow("Resolve didn't start in time (Timed out after 60s).");
    expect(never.closed()).toBe(1);
    const stuck = closedResolve({ running: true, readyAfter: Infinity });
    await expect(stuck.start({ runningTimeoutMs: 0 })).rejects.toThrow("doesn't answer");
  });

  it("stops waiting when the user stops the turn", async () => {
    const controller = new AbortController();
    const { start, closed } = closedResolve({ readyAfter: Infinity });
    const waiting = start({ signal: controller.signal, pollMs: 60_000 });
    controller.abort();
    await expect(waiting).rejects.toThrow("Stopped");
    expect(closed()).toBe(1);
  });
});
