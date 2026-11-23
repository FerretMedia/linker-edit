import * as path from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createResolveConnection, parseScriptResult, ResolveError, scriptSource, type ResolveConnection } from "./connection.ts";
import type { ResolveProcesses } from "./program.ts";

const FAKE = path.join(import.meta.dirname, "fixtures", "fakeResolveMcp.mjs");
const CLOSED: ResolveProcesses = { resolve: false, scriptServer: false };
const BOOTING: ResolveProcesses = { resolve: true, scriptServer: false };
const UP: ResolveProcesses = { resolve: true, scriptServer: true };
const open: ResolveConnection[] = [];

/** A connection on a computer with these processes; "unseen" is a look that fails. */
function on(processes: ResolveProcesses | "unseen", server: { command: string | null; args?: string[] } = { command: process.execPath, args: [FAKE] }) {
  const computer = { processes, looks: 0 };
  const connection = createResolveConnection({
    ...server,
    processes: async () => {
      computer.looks += 1;
      if (computer.processes === "unseen") throw new Error("can't see whether Resolve is running");
      return computer.processes;
    },
  });
  open.push(connection);
  return { computer, connection };
}

function connect(env: Record<string, string> = {}) {
  Object.assign(process.env, env);
  return on(CLOSED).connection;
}

afterEach(() => {
  for (const connection of open.splice(0)) connection.close();
  delete process.env.FAKE_NO_RUN_SCRIPT;
});

const status = async (connection: ResolveConnection) => JSON.parse(await connection.callTool("get_resolve_status")) as { calls: number; pid: number };
const failure = (promise: Promise<unknown>) =>
  promise.then(
    () => null,
    (error: unknown) => (error instanceof ResolveError ? { kind: error.kind, message: error.message } : String(error)),
  );

describe("scriptSource", () => {
  it("passes the input as JSON Python reads, ahead of the helpers and the script", () => {
    const source = scriptSource("status", { name: 'AI - "Cut"\n', n: 2 });
    expect(source.startsWith('import json\nARGS = json.loads("{\\"name\\":\\"AI - \\\\\\"Cut\\\\\\"\\\\n\\",\\"n\\":2}")')).toBe(true);
    expect(source).toContain("class Refused(Exception)");
    expect(source.trimEnd().endsWith("result = out")).toBe(true);
    expect(() => scriptSource("../secrets")).toThrow("Bad script name");
  });
});

describe("parseScriptResult", () => {
  it.each([
    ['{"result": {"a": 1}}', { a: 1 }],
    ['{"output": "hi\\n", "result": 2}', 2],
    ['{"output": "hi\\n"}', null],
  ])("reads %s", (text, expected) => {
    expect(parseScriptResult(text)).toEqual(expected);
  });

  it.each([
    ['{"error": "Traceback (most recent call last):\\n  File \\"<script>\\", line 3\\nRefused: \\"Edit\\" isn\'t an AI timeline."}', "refused", '"Edit" isn\'t an AI timeline.'],
    ['{"error": "Traceback (most recent call last):\\r\\n  ...\\r\\nAttributeError: \'NoneType\' object has no attribute \'GetName\'\\r\\n"}', "error", "AttributeError: 'NoneType' object has no attribute 'GetName'"],
    ['{"error": "Script timed out after 60s. Avoid long iterations; fetch only what you need."}', "timeout", "Script timed out after 60s. Avoid long iterations; fetch only what you need."],
    ["Error: DaVinci Resolve is not running, start it with the `launch_resolve` tool.", "error", "DaVinci Resolve isn't running."],
  ])("turns %s into a %s error", async (text, kind, message) => {
    expect(await failure(Promise.resolve().then(() => parseScriptResult(text)))).toEqual({ kind, message });
  });
});

describe("createResolveConnection", () => {
  it("starts Resolve's MCP server once and runs scripts through run_script, capped at 60 seconds", async () => {
    const connection = connect();
    expect(await connection.runScript("status", {}, 600)).toEqual({ length: scriptSource("status").length, timeout: 60 });
    const first = await status(connection);
    const second = await status(connection);
    expect([first.calls, second.calls, second.pid]).toEqual([1, 2, first.pid]);
  });

  it("runs calls one at a time, in order", async () => {
    const connection = connect();
    const results = await Promise.all([1, 2, 3].map(() => status(connection)));
    expect(results.map((result) => result.calls)).toEqual([1, 2, 3]);
  });

  it("gives up on a call that never answers, and starts afresh for the next", async () => {
    const connection = connect();
    const before = await status(connection);
    expect(await failure(connection.callTool("run_script", { script: "HANG" }, 300))).toMatchObject({ kind: "unavailable", message: expect.stringContaining("didn't answer in time") });
    expect((await status(connection)).pid).not.toBe(before.pid);
  });

  it("starts afresh after the server stops", async () => {
    const connection = connect();
    const before = await status(connection);
    expect(await failure(connection.callTool("run_script", { script: "DIE" }))).toMatchObject({ kind: "unavailable", message: expect.stringContaining("stopped (exit 3)") });
    expect((await status(connection)).pid).not.toBe(before.pid);
  });

  it("says when Resolve has no MCP server, or one without run_script", async () => {
    const missing = on(BOOTING, { command: null }).connection;
    expect(await failure(missing.runScript("status"))).toMatchObject({ kind: "unavailable", message: expect.stringContaining("21.1.2") });
    const old = connect({ FAKE_NO_RUN_SCRIPT: "1" });
    expect(await failure(old.runScript("status"))).toMatchObject({ kind: "unavailable", message: expect.stringContaining("no run_script") });
  });
});

describe("the boot guard", () => {
  // Only the guard's waiting is fake; the MCP server is a real process.
  beforeEach(() => void vi.useFakeTimers({ now: 0, toFake: ["setTimeout", "clearTimeout", "Date"] }));
  afterEach(() => void vi.useRealTimers());

  const NOT_RUNNING = { kind: "error", message: "DaVinci Resolve isn't running." };
  const run = (connection: ResolveConnection, script: string) => connection.callTool("run_script", { script }, 300);
  const blocked = async (connection: ResolveConnection) => expect(await failure(run(connection, ""))).toEqual(NOT_RUNNING);
  async function after<T>(ms: number, call: Promise<T>): Promise<T> {
    call.catch(() => {});
    await vi.advanceTimersByTimeAsync(ms);
    return call;
  }

  it("starts no MCP server while Resolve boots, and answers as if Resolve isn't running", async () => {
    // Starting this one would fail with another message.
    const { connection } = on(BOOTING, { command: "/no such folder/ResolveMCP" });
    expect(JSON.parse(await connection.callTool("get_resolve_status"))).toEqual({ running: false });
    await blocked(connection);
  });

  it("makes no call to a running MCP server while Resolve boots", async () => {
    const { computer, connection } = on(CLOSED);
    const { pid } = await status(connection);
    await run(connection, "CLOSED");
    computer.processes = BOOTING;
    await blocked(connection);
    expect(JSON.parse(await connection.callTool("get_resolve_status"))).toEqual({ running: false });
    computer.processes = UP;
    expect(await after(2000, status(connection))).toMatchObject({ pid, calls: 2 });
  });

  it("connects when the script server is still there after 2 seconds", async () => {
    const { computer, connection } = on(UP);
    const call = status(connection);
    await vi.advanceTimersByTimeAsync(1999);
    expect(computer.looks).toBe(1);
    expect(await after(1, call)).toMatchObject({ calls: 1 });
  });

  it("doesn't connect when the script server is gone after 2 seconds", async () => {
    const { computer, connection } = on(UP);
    const call = blocked(connection);
    await vi.advanceTimersByTimeAsync(1999);
    computer.processes = BOOTING;
    await after(1, call);
  });

  it("connects when the script server has been missing for 90 seconds, anew each time it vanishes", async () => {
    const { computer, connection } = on(BOOTING);
    await blocked(connection);
    await vi.advanceTimersByTimeAsync(80_000);
    computer.processes = UP;
    const call = blocked(connection);
    await vi.advanceTimersByTimeAsync(1999);
    computer.processes = BOOTING;
    await after(1, call);
    await vi.advanceTimersByTimeAsync(89_999);
    await blocked(connection);
    await vi.advanceTimersByTimeAsync(1);
    await run(connection, "");
  });

  it("connects when it can't look, unless it just saw Resolve boot", async () => {
    const { computer, connection } = on("unseen");
    await run(connection, "");
    await run(connection, "CLOSED");
    computer.processes = BOOTING;
    await blocked(connection);
    computer.processes = "unseen";
    await blocked(connection);
    await vi.advanceTimersByTimeAsync(90_000);
    await run(connection, "");
  });

  it.each([
    ["a result", (connection: ResolveConnection) => run(connection, "")],
    ["a refusal", (connection: ResolveConnection) => run(connection, "ERROR")],
    ["running", (connection: ResolveConnection) => connection.callTool("get_resolve_status")],
  ])("doesn't look again after Resolve answered with %s", async (_answer, answered) => {
    const { computer, connection } = on(UP);
    await after(2000, answered(connection));
    computer.looks = 0;
    await run(connection, "");
    await run(connection, "");
    expect(computer.looks).toBe(0);
  });

  it.each([
    ["isn't running", (connection: ResolveConnection) => connection.callTool("get_resolve_status", { running: false })],
    ["isn't running for a script", (connection: ResolveConnection) => run(connection, "CLOSED")],
    ["runs out of time", (connection: ResolveConnection) => run(connection, "SLOW")],
    ["doesn't answer", (connection: ResolveConnection) => after(300, failure(run(connection, "HANG")))],
    ["loses its MCP server", (connection: ResolveConnection) => failure(run(connection, "DIE"))],
  ])("looks again when Resolve %s", async (_what, unanswered) => {
    const { computer, connection } = on(UP);
    await after(2000, status(connection));
    await unanswered(connection);
    computer.processes = BOOTING;
    await blocked(connection);
  });
});
