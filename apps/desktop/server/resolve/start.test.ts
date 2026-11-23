import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createBootGuard } from "./boot.ts";
import type { ResolveProcesses } from "./program.ts";
import { startResolve } from "./start.ts";
import { fakeConnection } from "./testing.ts";

const MINUTE = 60_000;

beforeEach(() => void vi.useFakeTimers({ now: 0 }));
afterEach(() => void vi.useRealTimers());

interface Computer {
  running?: boolean;
  scriptServerAt?: number;
  answersAt?: number;
  failedLooks?: number[];
  launchError?: string;
  hangs?: boolean;
}

function computer({ running = false, scriptServerAt = 0, answersAt = 0, failedLooks = [], launchError, hangs = false }: Computer = {}) {
  const launches: string[] = [];
  const connections: number[] = [];
  let looks = 0;
  const processes = async (): Promise<ResolveProcesses> => {
    if (failedLooks.includes(looks++)) throw new Error("can't see whether Resolve is running");
    const resolve = running || launches.length > 0;
    return { resolve, scriptServer: resolve && Date.now() >= scriptServerAt };
  };
  const boot = createBootGuard(processes);
  const { connection } = fakeConnection({
    script: async () => {
      const text = await boot.call("run_script", async () => {
        connections.push(Date.now());
        if (Date.now() < answersAt) throw new Error("DaVinci Resolve isn't running.");
        if (hangs) return new Promise(() => {});
        return JSON.stringify({ state: "connected", page: null });
      });
      return JSON.parse(text) as unknown;
    },
  });
  const start = async (signal?: AbortSignal) => {
    const started = startResolve(connection, {
      signal,
      processes,
      launch: async (resolveMcp) => {
        launches.push(resolveMcp);
        if (launchError) throw new Error(launchError);
      },
    });
    const result = started.then(
      (value) => ({ value, error: undefined, after: Date.now() }),
      (error: Error) => ({ value: undefined, error: error.message, after: Date.now() }),
    );
    await vi.advanceTimersByTimeAsync(5 * MINUTE);
    return result;
  };
  return { start, launches, connections };
}

describe("startResolve", () => {
  it.each([
    ["it launched, once", false, ["ResolveMCP.exe"]],
    ["the user just started, without launching another", true, []],
  ])("doesn't connect until the script server has run for 2 seconds, of a Resolve %s", async (_whose, running, launched) => {
    const { start, launches, connections } = computer({ running, scriptServerAt: 20_000, answersAt: 30_000 });
    expect(await start()).toMatchObject({ value: !running });
    expect(launches).toEqual(launched);
    expect(connections[0]).toBeGreaterThanOrEqual(22_000);
    expect(connections[0]).toBeLessThan(25_000);
  });

  it("uses a Resolve that's already up, without launching another", async () => {
    const { start, launches } = computer({ running: true });
    expect(await start()).toMatchObject({ value: false, after: 2000 });
    expect(launches).toEqual([]);
  });

  it("launches nothing when it can't see whether Resolve is running", async () => {
    const { start, launches } = computer({ failedLooks: [0] });
    expect(await start()).toMatchObject({ error: expect.stringContaining("can't see") });
    expect(launches).toEqual([]);
  });

  it("goes on waiting when a later look fails", async () => {
    const { start, launches, connections } = computer({ scriptServerAt: 5000, failedLooks: [2, 3] });
    expect(await start()).toMatchObject({ value: true });
    expect(launches).toHaveLength(1);
    expect(connections[0]).toBeGreaterThanOrEqual(7000);
  });

  it("says why when Resolve can't be launched", async () => {
    const { start, connections } = computer({ launchError: "Resolve.exe is missing" });
    expect(await start()).toEqual({ error: "Resolve.exe is missing", after: 0 });
    expect(connections).toEqual([]);
  });

  it("gives up after 3 minutes on a Resolve that doesn't come up", async () => {
    const { error, after } = await computer({ answersAt: Infinity }).start();
    expect(error).toContain("didn't get ready in time");
    expect(after).toBeGreaterThanOrEqual(3 * MINUTE);
  });

  it("gives up after 30 seconds on a Resolve that's up but doesn't answer", async () => {
    const { error, after } = await computer({ running: true, answersAt: Infinity }).start();
    expect(error).toContain("doesn't answer");
    expect(after).toBeGreaterThanOrEqual(30_000);
    expect(after).toBeLessThan(MINUTE);
  });

  it.each([
    ["for the script server", { scriptServerAt: Infinity }],
    ["for an answer", { answersAt: Infinity }],
    ["during a call to Resolve", { hangs: true }],
  ])("stops waiting %s at once when the user stops the turn", async (_what, options) => {
    const controller = new AbortController();
    const { start, launches } = computer(options);
    setTimeout(() => controller.abort(), 10_500);
    expect(await start(controller.signal)).toEqual({ error: expect.stringContaining("Stopped"), after: 10_500 });
    expect(launches).toHaveLength(1);
  });

  it("launches nothing for a turn that was stopped already", async () => {
    const { start, launches } = computer();
    expect(await start(AbortSignal.abort())).toEqual({ error: expect.stringContaining("Stopped"), after: 0 });
    expect(launches).toEqual([]);
  });
});
