import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { describe, expect, it } from "vitest";
import { checkResolve, parseProbeOutput } from "./resolveStatus.ts";

describe("parseProbeOutput", () => {
  const connected = {
    state: "connected",
    product: "DaVinci Resolve Studio",
    version: "21.1.2",
    project: "Doc",
    projectId: "a1b2",
    database: { type: "Disk", name: "Local Database" },
    timeline: null,
  };
  const probed = JSON.stringify({ ...connected, page: "edit" });

  it.each([
    ["a connection", probed, connected],
    ["a connection after module warnings", `DeprecationWarning: imp\r\n${probed}\r\n`, connected],
    [
      "a connection without a project id or database",
      JSON.stringify({ state: "connected", project: "Doc", timeline: null, page: "edit" }),
      { project: "Doc", projectId: null, database: null },
    ],
    ["a dialog or the Project Manager in front", JSON.stringify({ ...connected, page: null }), { state: "blocked" }],
    ["a missing scripting module", '{"state": "no_module", "detail": "DLL load failed"}', { state: "setup_needed" }],
    ["a refused connection", '{"state": "unreachable"}', { state: "unreachable" }],
    ["a crash or timeout", "", { state: "unreachable" }],
    ["a traceback", "Traceback (most recent call last):\n  ...", { state: "unreachable" }],
  ])("reads %s", (_name, stdout, expected) => {
    expect(parseProbeOutput(stdout)).toMatchObject(expected);
  });

  it("says why the scripting module failed", () => {
    expect(parseProbeOutput('{"state": "no_module", "detail": "DLL load failed"}')).toEqual({
      state: "setup_needed",
      hint: "Resolve scripting module failed to load: DLL load failed",
    });
  });
});

describe("checkResolve", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-resolve-"));
  const python = path.join(dir, "python.exe");
  const serverScript = path.join(dir, "server.py");
  fs.writeFileSync(python, "");
  fs.writeFileSync(serverScript, "");

  it("asks for setup when the MCP checkout is missing", async () => {
    const status = await checkResolve({ python: path.join(dir, "nope.exe"), serverScript, env: {} }, async () => true);
    expect(status).toMatchObject({ state: "setup_needed", hint: expect.stringContaining("davinci-resolve-mcp not found") });
  });

  it("reports Resolve not running without probing it", async () => {
    expect(await checkResolve({ python, serverScript, env: {} }, async () => false)).toEqual({ state: "not_running" });
  });
});
