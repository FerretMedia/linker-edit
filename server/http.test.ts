import { once } from "node:events";
import * as fs from "node:fs";
import * as http from "node:http";
import type { AddressInfo } from "node:net";
import * as os from "node:os";
import * as path from "node:path";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { WebSocket } from "ws";
import { createApp } from "./app.ts";
import { memoryProjectStore, workspaceProject } from "./projects/testing.ts";
import { byteRange, createServer, isAllowedOrigin, staticFile } from "./http.ts";

describe("isAllowedOrigin", () => {
  it.each([
    [undefined, true],
    ["http://localhost:5317", true],
    ["http://127.0.0.1:4317", true],
    ["http://[::1]:4317", true],
    ["https://localhost", true],
    ["https://evil.example", false],
    ["http://localhost.evil.example", false],
    ["http://127.0.0.1.nip.io:4317", false],
    ["null", false],
    ["file://", false],
  ])("%s → %s", (origin, expected) => {
    expect(isAllowedOrigin(origin)).toBe(expected);
  });
});

describe("byteRange", () => {
  it.each([
    [undefined, null],
    ["bytes=0-", { start: 0, end: 999 }],
    ["bytes=100-199", { start: 100, end: 199 }],
    ["bytes=900-5000", { start: 900, end: 999 }],
    ["bytes=-100", { start: 900, end: 999 }],
    ["bytes=1000-", "invalid"],
    ["bytes=5-2", "invalid"],
    ["bytes=0-1,5-9", null],
    ["items=0-1", null],
  ])("%s → %j", (header, expected) => {
    expect(byteRange(header, 1000)).toEqual(expected);
  });
});

const root = fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-http-"));
const dist = path.join(root, "build");
fs.mkdirSync(path.join(dist, "_app"), { recursive: true });
fs.writeFileSync(path.join(dist, "index.html"), "<html>app</html>");
fs.writeFileSync(path.join(dist, "_app", "app.js"), "app()");
fs.mkdirSync(path.join(root, "build-secret"));
fs.writeFileSync(path.join(root, "build-secret", "token.txt"), "secret");
fs.writeFileSync(path.join(root, "outside.txt"), "outside");

describe("staticFile", () => {
  it.each([
    ["/", "index.html"],
    ["/_app/app.js", "_app/app.js"],
    ["/_app", "index.html"],
    ["/threads/42", "index.html"],
    ["/../outside.txt", "index.html"],
    ["/../build-secret/token.txt", "index.html"],
    ["\\..\\outside.txt", "index.html"],
  ])("%s → %s", (urlPath, expected) => {
    expect(staticFile(dist, urlPath)).toBe(path.join(dist, expected));
  });

  it("returns null when the UI isn't built", () => {
    expect(staticFile(path.join(root, "missing"), "/")).toBeNull();
  });
});

describe("server", () => {
  let server: http.Server;
  let port: number;

  beforeAll(async () => {
    const app = createApp({
      version: "0.0.0",
      store: memoryProjectStore([{ project: { ...workspaceProject, dir: root } }]),
      createSession: () => ({ send: async () => {}, resumeId: () => null }),
      checkResolve: async () => ({ state: "not_running" }),
    });
    server = createServer(app, dist);
    server.listen(0, "127.0.0.1");
    await once(server, "listening");
    port = (server.address() as AddressInfo).port;
  });

  afterAll(() => {
    server.closeAllConnections();
    server.close();
  });

  function get(urlPath: string): Promise<{ status: number; body: string }> {
    return new Promise((resolve, reject) => {
      http
        .get({ host: "127.0.0.1", port, path: urlPath }, (res) => {
          let body = "";
          res.on("data", (chunk) => (body += chunk));
          res.on("end", () => resolve({ status: res.statusCode ?? 0, body }));
        })
        .on("error", reject);
    });
  }

  it("answers a malformed URL with 400 and keeps serving", async () => {
    expect((await get("/%")).status).toBe(400);
    expect(await get("/")).toEqual({ status: 200, body: "<html>app</html>" });
  });

  it("refuses WebSocket connections from other websites", async () => {
    const socket = new WebSocket(`ws://127.0.0.1:${port}/ws`, { origin: "https://evil.example" });
    const [request, response] = (await once(socket, "unexpected-response")) as [http.ClientRequest, http.IncomingMessage];
    expect(response.statusCode).toBe(401);
    request.destroy();
  });

  it("sends the app's own pages a snapshot", async () => {
    const socket = new WebSocket(`ws://127.0.0.1:${port}/ws`, { origin: "http://localhost:5317" });
    const [data] = (await once(socket, "message")) as [Buffer];
    expect(JSON.parse(String(data))).toMatchObject({ type: "snapshot", state: { busy: false, items: [] } });
    socket.close();
  });
});
