import { spawnSync } from "node:child_process";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { parseScriptResult, scriptSource, type ResolveConnection } from "./connection.ts";

export const PYTHON = process.platform === "win32" ? "python" : "python3";
export const hasPython = spawnSync(PYTHON, ["--version"]).status === 0;

// Plays the part of Resolve's run_script: sets `resolve` and `project` from a stand-in scripting module,
// runs the script and prints {"result": …} or {"error": traceback}, as Resolve's MCP server answers.
const RUNNER = String.raw`
import json, sys, traceback
import DaVinciResolveScript as d
resolve = d.scriptapp("Resolve")
scope = {"resolve": resolve, "project": resolve.GetProjectManager().GetCurrentProject()}
try:
    exec(sys.stdin.read(), scope)
    print(json.dumps({"result": scope.get("result")}))
except Exception:
    print(json.dumps({"error": traceback.format_exc()}))
`;

/**
 * A connection that runs Linker-Edit's real scripts in this computer's Python against `module`, a stand-in
 * DaVinciResolveScript module. env reaches the module, for state files.
 */
export function pythonConnection(module: string, env: Record<string, string> = {}): ResolveConnection {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-fake-resolve-"));
  fs.writeFileSync(path.join(dir, "DaVinciResolveScript.py"), module);
  return {
    command: PYTHON,
    callTool: async () => {
      throw new Error("pythonConnection only runs scripts.");
    },
    runScript: async <T>(name: string, args?: unknown) => {
      const run = spawnSync(PYTHON, ["-c", RUNNER], {
        input: scriptSource(name, args),
        encoding: "utf8",
        env: { ...process.env, PYTHONPATH: dir, PYTHONIOENCODING: "utf-8", ...env },
      });
      return parseScriptResult(run.stdout.trim().split("\n").pop() ?? run.stderr) as T;
    },
    close: () => {},
  };
}

export interface FakeResolve {
  /** A ResolveMCP tool other than run_script; returns its text. */
  tool?: (name: string, args: Record<string, unknown>) => string | Promise<string>;
  /** A Linker-Edit script by name; returns its result or throws a ResolveError. */
  script?: (name: string, args: unknown) => unknown;
}

/** A stand-in for the connection to Resolve's MCP server that records the scripts it was asked to run. */
export function fakeConnection(fake: FakeResolve = {}, command: string | null = "ResolveMCP.exe") {
  const scripts: { name: string; args: unknown; timeout?: number }[] = [];
  const connection: ResolveConnection = {
    command,
    callTool: async (name, args = {}) => {
      if (!fake.tool) throw new Error(`No fake for ${name}.`);
      return fake.tool(name, args);
    },
    runScript: async <T>(name: string, args?: unknown, timeout?: number) => {
      scripts.push({ name, args, timeout });
      if (!fake.script) throw new Error(`No fake for ${name}.`);
      return (await fake.script(name, args)) as T;
    },
    close: () => {},
  };
  return { connection, scripts };
}
