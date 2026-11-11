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
import type { RemoteGate } from "./remote.ts";

describe("isAllowedOrigin", () => {
  it.each([
    [undefined, "127.0.0.1:4317", true],
    ["http://localhost:5317", "localhost:5317", true],
    ["http://127.0.0.1:4317", "127.0.0.1:4317", true],
    ["http://[::1]:4317", "[::1]:4317", true],
    ["https://localhost", "localhost", true],
    ["http://localhost:3000", "127.0.0.1:4317", false],
    ["http://127.0.0.1:3000", "127.0.0.1:4317", false],
    ["https://evil.example", "127.0.0.1:4317", false],
    ["http://localhost.evil.example", "localhost.evil.example", false],
    ["http://127.0.0.1.nip.io:4317", "127.0.0.1.nip.io:4317", false],
    ["null", "127.0.0.1:4317", false],
    ["file://", "127.0.0.1:4317", false],
  ])("%s on %s → %s", (origin, host, expected) => {
    expect(isAllowedOrigin(origin, host)).toBe(expected);
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
  let gate: RemoteGate | null = null;

  beforeAll(async () => {
    const app = createApp({
      version: "0.0.0",
      store: memoryProjectStore([{ project: { ...workspaceProject, dir: root } }]),
      createSession: () => ({ send: async () => {}, resumeId: () => null }),
      checkResolve: async () => ({ state: "not_running" }),
    });
    const linker = {
      start: async () => "https://clerk.example/oauth/authorize?state=abc",
      finish: async (query: URLSearchParams) => {
        if (query.get("state") !== "abc") throw new Error("Not <this> one.");
        return "Jane <Doe>";
      },
    };
    server = createServer(app, dist, "secret", { gate: () => gate, computers: async () => [] }, linker);
    server.listen(0, "127.0.0.1");
    await once(server, "listening");
    port = (server.address() as AddressInfo).port;
  });

  afterAll(() => {
    server.closeAllConnections();
    server.close();
  });

  function get(urlPath: string, headers: http.OutgoingHttpHeaders = {}): Promise<{ status: number; body: string }> {
    return new Promise((resolve, reject) => {
      http
        .get({ host: "127.0.0.1", port, path: urlPath, headers }, (res) => {
          let body = "";
          res.on("data", (chunk) => (body += chunk));
          res.on("end", () => resolve({ status: res.statusCode ?? 0, body }));
        })
        .on("error", reject);
    });
  }

  function post(urlPath: string, body: unknown, token?: string): Promise<{ status: number; body: string }> {
    return new Promise((resolve, reject) => {
      const headers = token ? { authorization: `Bearer ${token}` } : {};
      http
        .request({ host: "127.0.0.1", port, path: urlPath, method: "POST", headers }, (res) => {
          let text = "";
          res.on("data", (chunk) => (text += chunk));
          res.on("end", () => resolve({ status: res.statusCode ?? 0, body: text }));
        })
        .on("error", reject)
        .end(JSON.stringify(body));
    });
  }

  it("takes the agents' calls only with their token", async () => {
    const files = { files: ["C:\\Footage\\a.mov"] };
    expect((await post("/api/transcribe", files)).status).toBe(403);
    expect((await post("/api/transcribe", files, "guess")).status).toBe(403);
    expect(await post("/api/transcribe", files, "secret")).toEqual({ status: 400, body: '{"error":"Transcription is off."}' });
    const call = { tool: "list_timelines", input: {} };
    expect((await post("/api/resolve-tool", call)).status).toBe(403);
    expect(await post("/api/resolve-tool", call, "secret")).toEqual({
      status: 200,
      body: '{"text":"There is no Resolve tool called list_timelines.","isError":true}',
    });
  });

  it("signs in with Linker on this computer only", async () => {
    const start = await new Promise<http.IncomingMessage>((resolve, reject) =>
      http.get({ host: "127.0.0.1", port, path: "/api/linker/sign-in" }, resolve).on("error", reject),
    );
    start.resume();
    expect(start.statusCode).toBe(302);
    expect(start.headers.location).toBe("https://clerk.example/oauth/authorize?state=abc");

    const done = await get("/api/linker/callback?code=x&state=abc");
    expect(done.status).toBe(200);
    expect(done.body).toContain("Signed in as Jane &lt;Doe&gt;");
    const refused = await get("/api/linker/callback?code=x&state=forged");
    expect(refused.status).toBe(400);
    expect(refused.body).toContain("Not &lt;this&gt; one.");

    gate = { host: "studio.tail1234.ts.net:8443", owner: "jane@example.com" };
    try {
      const owner = { host: "studio.tail1234.ts.net:8443", "x-forwarded-for": "100.64.0.2", "tailscale-user-login": "jane@example.com" };
      expect((await get("/api/linker/sign-in", owner)).status).toBe(403);
    } finally {
      gate = null;
    }
  });

  it("answers a malformed URL with 400 and keeps serving", async () => {
    expect((await get("/%")).status).toBe(400);
    expect(await get("/")).toEqual({ status: 200, body: "<html>app</html>" });
  });

  it("lets only the owner's devices in through Tailscale, and only while remote access is on", async () => {
    const owner = { host: "studio.tail1234.ts.net:8443", "x-forwarded-for": "100.64.0.2", "tailscale-user-login": "jane@example.com" };
    const connects = async (headers: Record<string, string>, origin: string) => {
      const socket = new WebSocket(`ws://127.0.0.1:${port}/ws`, { origin, headers });
      const [event] = await Promise.race([once(socket, "open").then(() => ["open"]), once(socket, "unexpected-response").then(() => ["refused"])]);
      socket.terminate();
      return event === "open";
    };
    expect((await get("/", { host: "evil.example" })).status).toBe(403);
    expect((await get("/", owner)).status).toBe(403);

    gate = { host: "studio.tail1234.ts.net:8443", owner: "jane@example.com" };
    try {
      expect(await get("/", owner)).toEqual({ status: 200, body: "<html>app</html>" });
      expect((await get("/", { ...owner, "tailscale-user-login": "bob@example.com" })).status).toBe(403);
      expect(await connects(owner, "https://studio.tail1234.ts.net:8443")).toBe(true);
      expect(await connects(owner, "https://evil.example")).toBe(false);
    } finally {
      gate = null;
    }
  });

  it.each(["https://evil.example", "http://localhost:3000"])("refuses WebSocket connections from %s", async (origin) => {
    const socket = new WebSocket(`ws://127.0.0.1:${port}/ws`, { origin });
    const [request, response] = (await once(socket, "unexpected-response")) as [http.ClientRequest, http.IncomingMessage];
    expect(response.statusCode).toBe(401);
    request.destroy();
  });

  it("sends the app's own pages a snapshot", async () => {
    const socket = new WebSocket(`ws://127.0.0.1:${port}/ws`, { origin: `http://127.0.0.1:${port}` });
    const [data] = (await once(socket, "message")) as [Buffer];
    expect(JSON.parse(String(data))).toMatchObject({ type: "snapshot", state: { busy: false, items: [] } });
    socket.close();
  });
});
