import { describe, expect, it } from "vitest";
import { ResolveError } from "./resolve/connection.ts";
import { fakeConnection } from "./resolve/testing.ts";
import { checkResolve, statusFromProbe } from "./resolveStatus.ts";

const connected = {
  state: "connected",
  product: "DaVinci Resolve Studio",
  version: "21.1.2",
  project: "Doc",
  projectId: "a1b2",
  database: { type: "Disk", name: "Local Database" },
  timeline: null,
};

describe("statusFromProbe", () => {
  it.each([
    ["a connection", { ...connected, page: "edit" }, connected],
    ["a connection without a project id or database", { state: "connected", project: "Doc", timeline: null, page: "edit" }, { project: "Doc", projectId: null, database: null }],
    ["a dialog or the Project Manager in front", { ...connected, page: null }, { state: "blocked" }],
    ["no scripting", { state: "unreachable" }, { state: "unreachable" }],
    ["nothing", null, { state: "unreachable" }],
  ])("reads %s", (_name, probe, expected) => {
    expect(statusFromProbe(probe)).toMatchObject(expected);
  });
});

describe("checkResolve", () => {
  const running = (value: boolean) => async () => JSON.stringify({ running: value, version: "21.1.2" });

  it("asks for Resolve Studio 21.1.2 or newer when its MCP server is missing", async () => {
    const { connection } = fakeConnection({}, null);
    expect(await checkResolve(connection)).toMatchObject({ state: "setup_needed", hint: expect.stringContaining("21.1.2") });
  });

  it("reports Resolve not running without running a script", async () => {
    const { connection, scripts } = fakeConnection({ tool: running(false) });
    expect(await checkResolve(connection)).toEqual({ state: "not_running" });
    expect(scripts).toEqual([]);
  });

  it("reads the open project", async () => {
    const { connection, scripts } = fakeConnection({ tool: running(true), script: () => ({ ...connected, page: "edit" }) });
    expect(await checkResolve(connection)).toEqual(connected);
    expect(scripts.map((call) => call.name)).toEqual(["status"]);
  });

  it("calls a script that never finishes stuck", async () => {
    const { connection } = fakeConnection({
      tool: running(true),
      script: () => {
        throw new ResolveError("timeout", "Script timed out after 10s.");
      },
    });
    expect(await checkResolve(connection)).toMatchObject({ state: "blocked", hint: expect.stringContaining("restart Resolve") });
  });

  it("says why when the MCP server can't start or is too old", async () => {
    const broken = fakeConnection({
      tool: () => {
        throw new ResolveError("unavailable", "Couldn't start Resolve's MCP server: spawn EACCES");
      },
    });
    expect(await checkResolve(broken.connection)).toMatchObject({ state: "unreachable", hint: expect.stringContaining("EACCES") });
    const old = fakeConnection({
      tool: () => {
        throw new ResolveError("unavailable", "This Resolve's MCP server has no run_script tool. Update DaVinci Resolve Studio.");
      },
    });
    expect(await checkResolve(old.connection)).toMatchObject({ state: "setup_needed" });
  });
});
