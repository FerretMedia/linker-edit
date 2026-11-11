import * as path from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { createResolveConnection, parseScriptResult, ResolveError, scriptSource, type ResolveConnection } from "./connection.ts";

const FAKE = path.join(import.meta.dirname, "fixtures", "fakeResolveMcp.mjs");
const open: ResolveConnection[] = [];

function connect(env: Record<string, string> = {}) {
  Object.assign(process.env, env);
  const connection = createResolveConnection({ command: process.execPath, args: [FAKE] });
  open.push(connection);
  return connection;
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
    ["Error: Resolve is not running", "error", "Error: Resolve is not running"],
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
    const missing = createResolveConnection({ command: null });
    expect(await failure(missing.runScript("status"))).toMatchObject({ kind: "unavailable", message: expect.stringContaining("21.1.2") });
    const old = connect({ FAKE_NO_RUN_SCRIPT: "1" });
    expect(await failure(old.runScript("status"))).toMatchObject({ kind: "unavailable", message: expect.stringContaining("no run_script") });
  });
});
