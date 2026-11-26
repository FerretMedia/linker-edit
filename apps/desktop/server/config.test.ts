import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { describe, expect, it } from "vitest";
import { GOOGLE_CLIENT, LINKER_AUTH_SERVER, LINKER_CLIENT_ID, loadConfig } from "./config.ts";

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

  it("signs in to Linker with Linker-Edit's own client, only on Linker's own Clerk", () => {
    const none = () => null;
    expect(loadConfig(configFile("{}"), none).linker).toMatchObject({ authServer: LINKER_AUTH_SERVER, clientId: LINKER_CLIENT_ID });
    expect(loadConfig(configFile(JSON.stringify({ linkerClientId: "mine" })), none).linker.clientId).toBe("mine");
    // A production client id means nothing to staging's Clerk: there it's the published document or registering.
    const staging = configFile(JSON.stringify({ linkerAuthServer: "https://normal-giraffe-32.clerk.accounts.dev" }));
    expect(loadConfig(staging, none).linker.clientId).toBeUndefined();
    expect(loadConfig(configFile(JSON.stringify({ linkerAuthServer: `${LINKER_AUTH_SERVER}/` })), none).linker.clientId).toBe(LINKER_CLIENT_ID);
  });

  it("signs in to Google only with both halves of a client", () => {
    const none = () => null;
    const google = (env: Record<string, string>) => loadConfig(configFile("{}"), none, env).google.client;
    expect(google({})).toBe(GOOGLE_CLIENT);
    expect(google({ LINKER_EDIT_GOOGLE_CLIENT_ID: "id", LINKER_EDIT_GOOGLE_CLIENT_SECRET: "secret" })).toEqual({ id: "id", secret: "secret" });
    expect(google({ LINKER_EDIT_GOOGLE_CLIENT_ID: "id" })).toBe(GOOGLE_CLIENT);
  });

  it("names the file when it isn't valid JSON", () => {
    const file = configFile('{ "port": 4317, }');
    expect(() => loadConfig(file)).toThrow(`Can't read ${file}`);
  });
});
