import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { describe, expect, it } from "vitest";
import { loadConfig } from "./config.ts";

function configFile(contents: string): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-config-"));
  const file = path.join(dir, "linker-edit.config.json");
  fs.writeFileSync(file, contents);
  return file;
}

describe("loadConfig", () => {
  it("finds Resolve's own MCP server, unless the config names one", () => {
    expect(loadConfig(configFile("{}"), () => "C:/Resolve/ResolveMCP.exe").resolveMcp).toEqual({ command: "C:/Resolve/ResolveMCP.exe" });
    expect(loadConfig(configFile("{}"), () => null).resolveMcp).toEqual({ command: null });
    const file = configFile(JSON.stringify({ resolveMcp: "D:/Resolve/ResolveMCP.exe" }));
    expect(loadConfig(file, () => "C:/Resolve/ResolveMCP.exe").resolveMcp).toEqual({ command: "D:/Resolve/ResolveMCP.exe" });
  });

  it("refuses a port that isn't one", () => {
    expect(() => loadConfig(configFile(JSON.stringify({ port: "4317" })))).toThrow("port in");
    expect(() => loadConfig(configFile(JSON.stringify({ port: 70000 })))).toThrow("must be a port number");
    expect(loadConfig(configFile(JSON.stringify({ port: 5000 }))).port).toBe(5000);
  });

  it("names the file when it isn't valid JSON", () => {
    const file = configFile('{ "port": 4317, }');
    expect(() => loadConfig(file)).toThrow(`Can't read ${file}`);
  });
});
