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
  it("gives a venv Python its base interpreter's home on Windows, and lets env override defaults", () => {
    const venv = fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-venv-"));
    fs.mkdirSync(path.join(venv, "Scripts"));
    fs.writeFileSync(path.join(venv, "pyvenv.cfg"), "home = C:\\Python312 \nversion = 3.12.4\n");
    const python = path.join(venv, "Scripts", "python.exe");
    const file = configFile(JSON.stringify({ python, env: { RESOLVE_SCRIPT_API: "D:\\Scripting" } }));

    expect(loadConfig(file, "win32").resolveMcp.env).toMatchObject({
      PYTHONHOME: "C:\\Python312",
      RESOLVE_SCRIPT_API: "D:\\Scripting",
    });
    expect(loadConfig(file, "linux").resolveMcp.env).not.toHaveProperty("PYTHONHOME");
  });

  it("names the file when it isn't valid JSON", () => {
    const file = configFile('{ "port": 4317, }');
    expect(() => loadConfig(file)).toThrow(`Can't read ${file}`);
  });
});
