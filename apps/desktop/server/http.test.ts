import { once } from "node:events";
import * as fs from "node:fs";
import * as http from "node:http";
import type { AddressInfo } from "node:net";
import * as os from "node:os";
import * as path from "node:path";
import * as zlib from "node:zlib";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { WebSocket } from "ws";
import { createApp } from "./app.ts";
import { memoryLibraryStore } from "./library/folders.ts";
import { createLibrary } from "./library/library.ts";
import { memoryProjectStore, workspaceProject } from "./projects/testing.ts";
import { createSessions } from "./connect/sessions.ts";
import { byteRange, createServer, isAllowedOrigin, staticFile, subprotocolSession, type ConnectRoutes } from "./http.ts";
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

describe("subprotocolSession", () => {
  it.each([
    ["linker-edit, bearer.abc", "abc"],
    ["bearer.abc,linker-edit", "abc"],
    [undefined, null],
    ["linker-edit", null],
    ["bearer.abc", null],
    ["linker-edit, bearer.", null],
    ["linker-edit, bearer.abc, bearer.def", null],
    ["linker-edit, bearer.abc, chat", null],
    ["chat, bearer.abc", null],
  ])("%s → %s", (header, expected) => {
    expect(subprotocolSession(header)).toBe(expected);
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

  it("compresses the UI and the socket for the owner's other devices, not this computer's own window", async () => {
    fs.mkdirSync(path.join(dist, "_app", "immutable"), { recursive: true });
    const script = "export const cut = (clip) => clip.trim();\n".repeat(200);
    fs.writeFileSync(path.join(dist, "_app", "immutable", "entry.js"), script);
    const fetchRaw = (urlPath: string, headers: http.OutgoingHttpHeaders) =>
      new Promise<{ headers: http.IncomingHttpHeaders; body: Buffer }>((resolve, reject) => {
        http
          .get({ host: "127.0.0.1", port, path: urlPath, headers }, (res) => {
            const chunks: Buffer[] = [];
            res.on("data", (chunk: Buffer) => chunks.push(chunk));
            res.on("end", () => resolve({ headers: res.headers, body: Buffer.concat(chunks) }));
          })
          .on("error", reject);
      });
    const extensions = async (headers: Record<string, string>, origin: string) => {
      const socket = new WebSocket(`ws://127.0.0.1:${port}/ws`, { origin, headers });
      await once(socket, "open");
      socket.terminate();
      return socket.extensions;
    };
    const gzipped = { "accept-encoding": "gzip, deflate, br" };

    const local = await fetchRaw("/_app/immutable/entry.js", gzipped);
    expect(local.headers["content-encoding"]).toBeUndefined();
    expect(local.headers["cache-control"]).toBe("public, max-age=31536000, immutable");
    expect(String(local.body)).toBe(script);
    expect((await fetchRaw("/", gzipped)).headers["cache-control"]).toBe("no-cache");
    expect(await extensions({}, `http://127.0.0.1:${port}`)).toBe("");

    gate = { host: "studio.tail1234.ts.net:8443", owner: "jane@example.com" };
    try {
      const owner = { host: "studio.tail1234.ts.net:8443", "x-forwarded-for": "100.64.0.2", "tailscale-user-login": "jane@example.com" };
      const remote = await fetchRaw("/_app/immutable/entry.js", { ...owner, ...gzipped });
      expect(remote.headers["content-encoding"]).toBe("gzip");
      expect(remote.body.length).toBeLessThan(script.length / 10);
      expect(String(zlib.gunzipSync(remote.body))).toBe(script);
      expect((await fetchRaw("/_app/immutable/entry.js", { ...owner, "accept-encoding": "gzip;q=0" })).headers["content-encoding"]).toBeUndefined();
      expect(await extensions(owner, "https://studio.tail1234.ts.net:8443")).toBe("permessage-deflate");
    } finally {
      gate = null;
    }
  });

  it("sends the app's own pages a snapshot", async () => {
    const socket = new WebSocket(`ws://127.0.0.1:${port}/ws`, { origin: `http://127.0.0.1:${port}` });
    const [data] = (await once(socket, "message")) as [Buffer];
    expect(JSON.parse(String(data))).toMatchObject({ type: "snapshot", state: { busy: false, items: [] } });
    socket.close();
  });
});

describe("Linker Connect", () => {
  const TUNNEL = "edit-0123456789abcdef.linker.example";
  const viaTunnel = { host: TUNNEL, "cf-connecting-ip": "203.0.113.9", "x-forwarded-for": "203.0.113.9" };
  const TAILNET = "studio.tail1234.ts.net:8443";
  const viaTailscale = { host: TAILNET, "x-forwarded-for": "100.64.0.2", "tailscale-user-login": "jane@example.com" };
  /** Jane's own web address, allowed at Linker. */
  const WEB = "https://linker-edit-web.jane.workers.dev";
  let server: http.Server;
  let port: number;
  let on = true;
  let gate: RemoteGate | null = null;
  /** Moves the sessions' clock, for tickets that ran out. */
  let later = 0;
  const connectDir = fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-http-connect-"));
  const sessions = createSessions({ file: path.join(connectDir, "sessions.json"), now: () => Date.now() + later });
  const opened: string[] = [];
  fs.writeFileSync(path.join(connectDir, "Song.mp3"), "0123456789");

  beforeAll(async () => {
    const store = memoryLibraryStore([{ id: "lib", kind: "music", label: "Music", path: connectDir, note: "" }]);
    const app = createApp({
      version: "0.0.0",
      store: memoryProjectStore([{ project: { ...workspaceProject, dir: root } }]),
      createSession: () => ({ send: async () => {}, resumeId: () => null }),
      checkResolve: async () => ({ state: "not_running" }),
      library: createLibrary({ store, cacheDir: path.join(connectDir, ".cache"), probe: async () => ({ duration: null, channels: null, loudness: null }) }),
    });
    const connect: ConnectRoutes = {
      host: () => (on ? TUNNEL : null),
      admit: async (token, device) =>
        token === "jane" || token === "lew_jane" ? { code: sessions.issueCode(device) } : { status: 403, error: "Not yours." },
      redeem: (code) => {
        const entered = sessions.enter(code, "native");
        return entered ? { session: entered.token, expiresAt: new Date(entered.expiresAt).toISOString() } : { status: 401, error: "Ran out." };
      },
      open: async (url) => {
        opened.push(url);
        return `${url}/api/connect/enter?code=x`;
      },
      others: async () => [
        { id: "office", name: "Office", url: "https://edit-1111111111111111.linker.example", tailnetUrl: null, online: true, linkedAt: "", lastSeenAt: null },
        { id: "away", name: "Away", url: "https://edit-2222222222222222.linker.example", tailnetUrl: null, online: false, linkedAt: "", lastSeenAt: null },
        // Linked for Tailscale only: no address at Linker, so not in the computer menu.
        { id: "den", name: "Den", url: null, tailnetUrl: "https://den.tail1234.ts.net:8443", online: null, linkedAt: "", lastSeenAt: null },
      ],
      sessionOn: async () => null,
      sessions,
      webOrigins: { allows: async (origin) => origin === WEB },
      probe: async (url) => (url.includes("1111") ? { name: "Office PC", version: "2026.1114.1" } : null),
    };
    server = createServer(app, dist, "secret", { gate: () => gate, computers: async () => [] }, undefined, connect);
    server.listen(0, "127.0.0.1");
    await once(server, "listening");
    port = (server.address() as AddressInfo).port;
  });

  afterAll(() => {
    server.closeAllConnections();
    server.close();
  });

  function request(
    method: string,
    urlPath: string,
    headers: http.OutgoingHttpHeaders = {},
    body?: unknown,
  ): Promise<{ status: number; body: string; headers: http.IncomingHttpHeaders }> {
    return new Promise((resolve, reject) => {
      http
        .request({ host: "127.0.0.1", port, path: urlPath, method, headers }, (res) => {
          let text = "";
          res.on("data", (chunk) => (text += chunk));
          res.on("end", () => resolve({ status: res.statusCode ?? 0, body: text, headers: res.headers }));
        })
        .on("error", reject)
        .end(body === undefined ? undefined : JSON.stringify(body));
    });
  }

  async function signIn(): Promise<string> {
    const admitted = await request("POST", "/api/connect/session", { ...viaTunnel, authorization: "Bearer jane" }, { name: "Laptop" });
    expect(admitted.status).toBe(200);
    const { code } = JSON.parse(admitted.body) as { code: string };
    const entered = await request("GET", `/api/connect/enter?code=${code}`, viaTunnel);
    expect(entered.status).toBe(302);
    const cookie = String(entered.headers["set-cookie"]);
    expect(cookie).toMatch(/HttpOnly; Secure; SameSite=Lax/);
    return cookie.split(";")[0]!;
  }

  it("lets nothing through the tunnel without a session but the way in", async () => {
    expect((await request("GET", "/", viaTunnel)).status).toBe(401);
    expect((await request("GET", "/media/x", viaTunnel)).status).toBe(401);
    const hello = await request("GET", "/api/hello", viaTunnel);
    expect(JSON.parse(hello.body)).toEqual({ app: "linker-edit", name: expect.any(String), version: "0.0.0", protocol: 1 });
    expect((await request("POST", "/api/connect/session", viaTunnel, {})).status).toBe(401);
    expect((await request("POST", "/api/connect/session", { ...viaTunnel, authorization: "Bearer bob" }, {})).status).toBe(403);
    expect((await request("GET", "/api/connect/enter?code=guess", viaTunnel)).status).toBe(401);
    // Opening other computers and signing in with Linker are for this computer itself.
    expect((await request("POST", "/api/connect/open", viaTunnel, { url: "https://x" })).status).toBe(401);
    expect((await request("GET", "/api/linker/sign-in", viaTunnel)).status).toBe(401);
  });

  it("serves the app to a session, and cuts its connections off when it's signed out", async () => {
    const cookie = await signIn();
    expect(await request("GET", "/", { ...viaTunnel, cookie })).toMatchObject({ status: 200, body: "<html>app</html>" });
    expect((await request("POST", "/api/connect/open", { ...viaTunnel, cookie }, { url: "https://x" })).status).toBe(403);

    const refused = new WebSocket(`ws://127.0.0.1:${port}/ws`, { origin: "https://evil.example", headers: { ...viaTunnel, cookie } });
    const [refusedRequest, refusedResponse] = (await once(refused, "unexpected-response")) as [http.ClientRequest, http.IncomingMessage];
    expect(refusedResponse.statusCode).toBe(401);
    refusedRequest.destroy();

    const socket = new WebSocket(`ws://127.0.0.1:${port}/ws`, { origin: `https://${TUNNEL}`, headers: { ...viaTunnel, cookie } });
    await once(socket, "open");
    const closed = once(socket, "close");
    sessions.revokeAll();
    await closed;
    expect((await request("GET", "/", { ...viaTunnel, cookie })).status).toBe(401);
  });

  /** Signs in as the iPad app does; answers its Bearer session. */
  async function signInNative(): Promise<string> {
    const admitted = await request("POST", "/api/connect/session", { ...viaTunnel, authorization: "Bearer jane" }, { name: "iPad" });
    const { code } = JSON.parse(admitted.body) as { code: string };
    const redeemed = await request("POST", "/api/connect/native", viaTunnel, { code });
    expect(redeemed.status).toBe(200);
    expect(redeemed.headers["cache-control"]).toBe("no-store");
    expect(redeemed.headers["set-cookie"]).toBeUndefined();
    const body = JSON.parse(redeemed.body) as { session: string; expiresAt: string };
    expect(Object.keys(body).sort()).toEqual(["expiresAt", "session"]);
    expect(new Date(body.expiresAt).getTime()).toBeGreaterThan(Date.now());
    // The code is spent.
    expect((await request("POST", "/api/connect/native", viaTunnel, { code })).status).toBe(401);
    return body.session;
  }

  /** Opens /ws through the tunnel; answers whether it was let in, and the socket when it was. */
  async function openSocket(headers: Record<string, string>, origin?: string): Promise<WebSocket | null> {
    const socket = new WebSocket(`ws://127.0.0.1:${port}/ws`, { ...(origin ? { origin } : {}), headers: { ...viaTunnel, ...headers } });
    const [event] = await Promise.race([once(socket, "open").then(() => ["open"]), once(socket, "unexpected-response").then(() => ["refused"])]);
    if (event === "open") return socket;
    socket.terminate();
    return null;
  }

  it("gives the iPad app a Bearer session that opens everything a cookie does", async () => {
    const session = await signInNative();
    const bearer = { authorization: `Bearer ${session}` };
    expect(await request("GET", "/", { ...viaTunnel, ...bearer })).toMatchObject({ status: 200, body: "<html>app</html>" });
    // AVPlayer sends the header with its Range requests.
    expect(await request("GET", "/library/lib/Song.mp3", { ...viaTunnel, ...bearer, range: "bytes=2-4" })).toMatchObject({ status: 206, body: "234" });
    expect(sessions.list()).toContainEqual(expect.objectContaining({ name: "iPad", kind: "native" }));

    // A native app sends no Origin; a page can't add the header to a WebSocket, but if one names another site, no.
    const socket = await openSocket(bearer);
    expect(socket).not.toBeNull();
    const own = await openSocket(bearer, `https://${TUNNEL}`);
    expect(own).not.toBeNull();
    own?.terminate();
    expect(await openSocket(bearer, "https://evil.example")).toBeNull();
    expect(await openSocket({ authorization: "Bearer guess" })).toBeNull();

    // Signing the iPad out in Settings cuts it off at once.
    const closed = once(socket!, "close");
    sessions.revoke(sessions.list().find((device) => device.kind === "native")!.id);
    await closed;
    const after = await request("GET", "/", { ...viaTunnel, ...bearer });
    expect(after.status).toBe(401);
    expect(after.headers["www-authenticate"]).toContain("Bearer");
    expect(JSON.parse(after.body)).toEqual({ error: expect.any(String) });
    expect(await openSocket(bearer)).toBeNull();
  });

  it("takes a session only the way it was given out", async () => {
    const session = await signInNative();
    const cookie = await signIn();
    expect((await request("GET", "/", { ...viaTunnel, cookie: `linker_edit_session=${session}` })).status).toBe(401);
    expect((await request("GET", "/", { ...viaTunnel, authorization: `Bearer ${cookie.split("=")[1]}` })).status).toBe(401);
    // A Bearer header that isn't a session doesn't fall back to the cookie next to it.
    expect((await request("GET", "/", { ...viaTunnel, cookie, authorization: "Bearer guess" })).status).toBe(401);
    // Without a session the native route answers nothing else.
    expect((await request("GET", "/api/connect/native", viaTunnel)).status).toBe(405);
    expect((await request("POST", "/api/connect/native", viaTunnel, { code: "guess" })).status).toBe(401);
    expect((await request("POST", "/api/connect/native", viaTunnel, {})).status).toBe(401);
  });

  it("takes a Bearer session only through the tunnel or Tailscale", async () => {
    const bearer = { authorization: `Bearer ${await signInNative()}` };
    // Through Tailscale while it's off, or with the tunnel's name but not through Cloudflare, it counts for nothing.
    const tailscale = { host: "studio.tail1234.ts.net:8443", "x-forwarded-for": "100.64.0.2", "tailscale-user-login": "jane@example.com" };
    expect((await request("GET", "/", { ...tailscale, ...bearer })).status).toBe(403);
    expect((await request("GET", "/", { host: TUNNEL, "x-forwarded-for": "203.0.113.9", ...bearer })).status).toBe(403);
    // This computer itself is let in as before, whatever the header says.
    expect((await request("GET", "/", { authorization: "Bearer guess" })).status).toBe(200);
    // And the native route is only for the tunnel.
    expect((await request("POST", "/api/connect/native", { host: TUNNEL, "x-forwarded-for": "203.0.113.9" }, { code: "x" })).status).toBe(403);
  });

  it("turns the tunnel away while Linker Connect is off", async () => {
    const cookie = await signIn();
    const authorization = `Bearer ${await signInNative()}`;
    on = false;
    try {
      expect((await request("GET", "/", { ...viaTunnel, cookie })).status).toBe(403);
      expect((await request("GET", "/", { ...viaTunnel, authorization })).status).toBe(403);
    } finally {
      on = true;
    }
  });

  /** Signs in as a page on Jane's web address does, through the tunnel or over Tailscale; answers its session. */
  async function signInWeb(via: Record<string, string> = viaTunnel): Promise<string> {
    const page = { ...via, origin: WEB, "content-type": "application/json" };
    const admitted = await request("POST", "/api/connect/session", { ...page, authorization: "Bearer lew_jane" }, { name: "Browser on Windows" });
    expect(admitted.status).toBe(200);
    expect(admitted.headers["access-control-allow-origin"]).toBe(WEB);
    const redeemed = await request("POST", "/api/connect/native", page, { code: (JSON.parse(admitted.body) as { code: string }).code });
    expect(redeemed.status).toBe(200);
    return (JSON.parse(redeemed.body) as { session: string }).session;
  }

  /** Opens /ws as a page on a web address does: the session in Sec-WebSocket-Protocol. */
  async function openPageSocket(via: Record<string, string>, origin: string, protocols: string[]): Promise<WebSocket | null> {
    const socket = new WebSocket(`ws://127.0.0.1:${port}/ws`, protocols, { origin, headers: via });
    const [event] = await Promise.race([
      once(socket, "open").then(() => ["open"]),
      once(socket, "unexpected-response").then(() => ["refused"]),
      once(socket, "error").then(() => ["refused"]),
    ]);
    if (event === "open") return socket;
    socket.terminate();
    return null;
  }

  it("answers only your own web addresses' preflights, never with credentials", async () => {
    const preflight = { ...viaTunnel, origin: WEB, "access-control-request-method": "POST", "access-control-request-headers": "authorization" };
    const allowed = await request("OPTIONS", "/api/connect/session", preflight);
    expect(allowed.status).toBe(204);
    expect(allowed.headers).toMatchObject({
      "access-control-allow-origin": WEB,
      vary: "origin",
      "access-control-allow-headers": "authorization, content-type, range",
      "access-control-allow-methods": "GET, POST, PUT, PATCH, DELETE, OPTIONS",
      "access-control-expose-headers": "content-range, accept-ranges, content-length, www-authenticate",
      "access-control-max-age": "600",
    });
    expect(allowed.headers["access-control-allow-credentials"]).toBeUndefined();
    expect(allowed.headers["access-control-allow-private-network"]).toBeUndefined();
    // Chrome's local network check, for Tailscale's 100.x addresses.
    gate = { host: TAILNET, owner: "jane@example.com" };
    try {
      const local = await request("OPTIONS", "/api/hello", { ...viaTailscale, origin: WEB, "access-control-request-private-network": "true" });
      expect(local.status).toBe(204);
      expect(local.headers["access-control-allow-private-network"]).toBe("true");
    } finally {
      gate = null;
    }
    // Any other site: no CORS at all, and nothing else either.
    for (const origin of ["https://evil.example", "https://linker-edit-web.jane.workers.dev.evil.example", "null", "http://localhost:5173"]) {
      const refused = await request("OPTIONS", "/api/connect/session", { ...preflight, origin });
      expect(refused.status).toBe(403);
      expect(refused.headers["access-control-allow-origin"]).toBeUndefined();
      expect((await request("GET", "/api/hello", { ...viaTunnel, origin })).status).toBe(403);
      expect((await request("POST", "/api/connect/session", { ...viaTunnel, origin, authorization: "Bearer lew_jane" }, {})).status).toBe(403);
    }
  });

  it("signs a page on your web address in through the tunnel with a Bearer session, never the cookie", async () => {
    const session = await signInWeb();
    const page = { ...viaTunnel, origin: WEB };
    const app = await request("GET", "/", { ...page, authorization: `Bearer ${session}` });
    expect(app).toMatchObject({ status: 200, body: "<html>app</html>" });
    expect(app.headers["access-control-allow-origin"]).toBe(WEB);
    // A cookie never counts cross-origin, even from an allowed address.
    const cookie = await signIn();
    const withCookie = await request("GET", "/", { ...page, cookie });
    expect(withCookie.status).toBe(401);
    expect(withCookie.headers["www-authenticate"]).toContain("Bearer");
    expect(withCookie.headers["access-control-allow-origin"]).toBe(WEB);
    expect((await request("GET", "/api/connect/enter?code=x", page)).status).toBe(401);
    // Its sign-in works with the Linker web token of the account that linked this computer only.
    expect((await request("POST", "/api/connect/session", { ...page, authorization: "Bearer lew_bob" }, {})).status).toBe(403);
  });

  it("signs a page in over Tailscale too, for the owner's devices only", async () => {
    gate = { host: TAILNET, owner: "jane@example.com" };
    try {
      const session = await signInWeb(viaTailscale);
      const page = { ...viaTailscale, origin: WEB };
      expect((await request("GET", "/", { ...page, authorization: `Bearer ${session}` })).status).toBe(200);
      // Someone else's device on the tailnet doesn't get in, nor gets a code.
      const bob = { ...page, "tailscale-user-login": "bob@example.com" };
      expect((await request("POST", "/api/connect/session", { ...bob, authorization: "Bearer lew_jane" }, { name: "x" })).status).toBe(403);
      expect((await request("GET", "/", { ...bob, authorization: `Bearer ${session}` })).status).toBe(403);
      expect((await request("OPTIONS", "/", { ...bob, "access-control-request-method": "GET" })).status).toBe(403);
      // A page needs its session, and a Bearer header must be one.
      const signedOut = await request("GET", "/", page);
      expect(signedOut.status).toBe(401);
      expect(signedOut.headers["www-authenticate"]).toContain("Bearer");
      expect((await request("GET", "/", { ...page, authorization: "Bearer guess" })).status).toBe(401);
      expect((await request("GET", "/", { ...viaTailscale, authorization: "Bearer guess" })).status).toBe(401);
      expect((await request("GET", "/", { ...page, origin: "https://evil.example", authorization: `Bearer ${session}` })).status).toBe(403);
      // The owner's own Tailscale pages come in as before.
      expect((await request("GET", "/", { ...viaTailscale, origin: `https://${TAILNET}` })).status).toBe(200);
    } finally {
      gate = null;
    }
  });

  it("opens /ws for a page only with its session as a subprotocol, from your web address", async () => {
    const session = await signInWeb();
    const protocols = ["linker-edit", `bearer.${session}`];
    const socket = await openPageSocket(viaTunnel, WEB, protocols);
    expect(socket?.protocol).toBe("linker-edit");
    for (const [origin, offered] of [
      ["https://evil.example", protocols],
      [`https://${TUNNEL}`, protocols],
      [WEB, [`bearer.${session}`]],
      [WEB, ["linker-edit"]],
      [WEB, ["chat", `bearer.${session}`]],
      [WEB, [...protocols, "chat"]],
      [WEB, ["linker-edit", "bearer.guess"]],
    ] as const) {
      expect(await openPageSocket(viaTunnel, origin, [...offered])).toBeNull();
    }
    gate = { host: TAILNET, owner: "jane@example.com" };
    try {
      const overTailscale = await openPageSocket(viaTailscale, WEB, protocols);
      expect(overTailscale?.protocol).toBe("linker-edit");
      overTailscale?.terminate();
      expect(await openPageSocket({ ...viaTailscale, "tailscale-user-login": "bob@example.com" }, WEB, protocols)).toBeNull();
      expect(await openPageSocket(viaTailscale, "https://evil.example", protocols)).toBeNull();
    } finally {
      gate = null;
    }
    // Signing the page out in Settings cuts it off at once.
    const closed = once(socket!, "close");
    sessions.revoke(sessions.check(session, "native")!);
    await closed;
    expect(await openPageSocket(viaTunnel, WEB, protocols)).toBeNull();
  });

  it("gives a Bearer session media tickets that play files in its place, Range included", async () => {
    const session = await signInWeb();
    const page = { ...viaTunnel, origin: WEB };
    const issued = await request("POST", "/api/connect/media-ticket", { ...page, authorization: `Bearer ${session}` });
    expect(issued.status).toBe(200);
    expect(issued.headers["cache-control"]).toBe("no-store");
    const { ticket, expiresAt } = JSON.parse(issued.body) as { ticket: string; expiresAt: string };
    expect(new Date(expiresAt).getTime() - Date.now()).toBeGreaterThan(59 * 60_000);
    expect(ticket).not.toContain(session);
    const t = encodeURIComponent(ticket);
    // <video> sends no Origin and no header; with crossorigin it sends the Origin.
    expect(await request("GET", `/library/lib/Song.mp3?t=${t}`, { ...viaTunnel, range: "bytes=2-4" })).toMatchObject({ status: 206, body: "234" });
    const cors = await request("GET", `/library/lib/Song.mp3?t=${t}`, { ...page, range: "bytes=0-1" });
    expect(cors).toMatchObject({ status: 206, body: "01" });
    expect(cors.headers["access-control-allow-origin"]).toBe(WEB);
    // Only on the media routes, only to GET, and never as a session.
    expect((await request("GET", `/?t=${t}`, viaTunnel)).status).toBe(401);
    expect((await request("POST", `/library/lib/Song.mp3?t=${t}`, viaTunnel)).status).toBe(401);
    expect((await request("GET", "/", { ...viaTunnel, authorization: `Bearer ${ticket}` })).status).toBe(401);
    expect((await request("POST", "/api/connect/media-ticket", { ...viaTunnel, authorization: `Bearer ${ticket}` })).status).toBe(401);
    // Forged, from another start of Linker-Edit, or out of time: no.
    const [id, at] = ticket.split(".");
    expect((await request("GET", `/library/lib/Song.mp3?t=${id}.${at}.forged`, viaTunnel)).status).toBe(401);
    const otherProcess = createSessions({ file: path.join(connectDir, "sessions.json") }).ticket(id!).ticket;
    expect((await request("GET", `/library/lib/Song.mp3?t=${encodeURIComponent(otherProcess)}`, viaTunnel)).status).toBe(401);
    later = 60 * 60_000;
    try {
      expect((await request("GET", `/library/lib/Song.mp3?t=${t}`, viaTunnel)).status).toBe(401);
    } finally {
      later = 0;
    }
    // Only a Bearer session gets one: not a cookie, nor nobody.
    const cookie = await signIn();
    expect((await request("POST", "/api/connect/media-ticket", { ...viaTunnel, cookie })).status).toBe(401);
    expect((await request("POST", "/api/connect/media-ticket", viaTunnel)).status).toBe(401);
    expect((await request("GET", "/api/connect/media-ticket", { ...viaTunnel, authorization: `Bearer ${session}` })).status).toBe(405);
    // And it dies with its session.
    sessions.revoke(sessions.check(session, "native")!);
    expect((await request("GET", `/library/lib/Song.mp3?t=${t}`, viaTunnel)).status).toBe(401);
  });

  it("lists linked computers and opens them only for this computer", async () => {
    const list = await request("GET", "/api/computers");
    expect(JSON.parse(list.body).others).toEqual([
      { name: "Office PC", url: "https://edit-1111111111111111.linker.example", version: "2026.1114.1", via: "linker", online: true },
      { name: "Away", url: "https://edit-2222222222222222.linker.example", version: "", via: "linker", online: false },
    ]);
    const open = await request("POST", "/api/connect/open", { "content-type": "application/json" }, { url: "https://edit-1111111111111111.linker.example" });
    expect(JSON.parse(open.body)).toEqual({ url: "https://edit-1111111111111111.linker.example/api/connect/enter?code=x" });
    const fromWebsite = await request("POST", "/api/connect/open", { origin: "https://evil.example" }, { url: "https://x" });
    expect(fromWebsite.status).toBe(403);
    expect(opened).toEqual(["https://edit-1111111111111111.linker.example"]);
  });

  it("lists your computers and passes requests on to them only for this computer's own window", async () => {
    const own = { origin: `http://127.0.0.1:${port}`, "sec-fetch-site": "same-origin" };
    const list = await request("GET", "/api/machines", own);
    expect(JSON.parse(list.body).machines).toEqual([
      { id: "this", name: os.hostname(), version: "0.0.0", via: "this", online: true, base: "" },
      { id: "office", name: "Office PC", version: "2026.1114.1", via: "linker", online: true, base: "/machines/office" },
      { id: "away", name: "Away", version: "", via: "linker", online: false, base: "/machines/away" },
    ]);
    // Reaching Office needs this computer's Linker sign-in, which the fake doesn't have.
    expect(JSON.parse((await request("GET", "/machines/office/api/hello", own)).body)).toEqual({
      error: "Sign in with Linker on this computer to reach Office PC.",
    });
    for (const headers of [{ origin: "https://evil.example" }, { "sec-fetch-site": "cross-site" }, viaTunnel, viaTailscale]) {
      expect((await request("GET", "/api/machines", headers)).status).toBe(403);
      expect((await request("GET", "/api/machines/storage?id=office", headers)).status).toBe(403);
      expect((await request("GET", "/machines/office/api/hello", headers)).status).toBe(403);
    }
    const socket = new WebSocket(`ws://127.0.0.1:${port}/machines/office/ws`, { origin: "https://evil.example" });
    const [refused, response] = (await once(socket, "unexpected-response")) as [http.ClientRequest, http.IncomingMessage];
    expect(response.statusCode).toBe(403);
    refused.destroy();
  });
});

describe("library files", () => {
  const library = fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-http-library-"));
  const outside = fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-http-outside-"));
  fs.writeFileSync(path.join(library, "Song.mp3"), "0123456789");
  fs.writeFileSync(path.join(library, "page.html"), "<script>alert(1)</script>");
  fs.writeFileSync(path.join(outside, "Secret.mp3"), "secret");
  fs.symlinkSync(outside, path.join(library, "Out"), "junction");
  let server: http.Server;
  let port: number;
  let gate: RemoteGate | null = null;

  beforeAll(async () => {
    const store = memoryLibraryStore([{ id: "lib", kind: "music", label: "Music", path: library, note: "" }]);
    const app = createApp({
      version: "0.0.0",
      store: memoryProjectStore([{ project: workspaceProject }]),
      createSession: () => ({ send: async () => {}, resumeId: () => null }),
      checkResolve: async () => ({ state: "not_running" }),
      library: createLibrary({ store, cacheDir: path.join(library, ".cache"), probe: async () => ({ duration: null, channels: null, loudness: null }) }),
    });
    server = createServer(app, dist, "secret", { gate: () => gate, computers: async () => [] });
    server.listen(0, "127.0.0.1");
    await once(server, "listening");
    port = (server.address() as AddressInfo).port;
  });

  afterAll(() => {
    server.closeAllConnections();
    server.close();
  });

  function get(urlPath: string, headers: http.OutgoingHttpHeaders = {}): Promise<{ status: number; body: string; headers: http.IncomingHttpHeaders }> {
    return new Promise((resolve, reject) => {
      // A raw path, so ".." reaches the server as it was typed.
      http
        .get({ host: "127.0.0.1", port, path: urlPath, headers }, (res) => {
          let body = "";
          res.on("data", (chunk) => (body += chunk));
          res.on("end", () => resolve({ status: res.statusCode ?? 0, body, headers: res.headers }));
        })
        .on("error", reject);
    });
  }

  it("plays a file inside a library folder, in parts when asked, never as a page", async () => {
    const whole = await get("/library/lib/Song.mp3");
    expect(whole).toMatchObject({ status: 200, body: "0123456789" });
    expect(whole.headers["content-type"]).toBe("audio/mpeg");
    expect(whole.headers["x-content-type-options"]).toBe("nosniff");
    expect(whole.headers["content-security-policy"]).toContain("sandbox");
    expect(await get("/library/lib/Song.mp3", { range: "bytes=2-4" })).toMatchObject({ status: 206, body: "234" });
  });

  it.each([
    `/library/lib/../${path.basename(outside)}/Secret.mp3`,
    `/library/lib/%2e%2e/${path.basename(outside)}/Secret.mp3`,
    `/library/lib/..%2F${path.basename(outside)}%2FSecret.mp3`,
    `/library/lib/..%5C${path.basename(outside)}%5CSecret.mp3`,
    "/library/lib/Out/Secret.mp3",
    `/library/lib/${encodeURIComponent(path.join(outside, "Secret.mp3"))}`,
    "/library/lib/C:/Windows/Media/tada.wav",
    "/library/lib/C%3A%5CWindows%5CMedia%5Ctada.wav",
    "/library/lib/%5C%5Cserver%5Cshare%5CSong.mp3",
    "/library/lib/page.html",
    "/library/other/Song.mp3",
    "/library/lib/",
  ])("refuses %s", async (urlPath) => {
    const response = await get(urlPath);
    expect(response.status).toBe(404);
    expect(response.body).toBe("");
  });

  it("plays only to this computer and the owner's own devices", async () => {
    const owner = { host: "studio.tail1234.ts.net:8443", "x-forwarded-for": "100.64.0.2", "tailscale-user-login": "jane@example.com" };
    expect((await get("/library/lib/Song.mp3", owner)).status).toBe(403);
    expect((await get("/library/lib/Song.mp3", { host: "evil.example" })).status).toBe(403);
    gate = { host: "studio.tail1234.ts.net:8443", owner: "jane@example.com" };
    try {
      expect(await get("/library/lib/Song.mp3", owner)).toMatchObject({ status: 200, body: "0123456789" });
      expect((await get("/library/lib/Song.mp3", { ...owner, "tailscale-user-login": "bob@example.com" })).status).toBe(403);
    } finally {
      gate = null;
    }
  });

  it("takes the agents' library calls only with their token", async () => {
    const call = (token?: string) =>
      new Promise<number>((resolve, reject) => {
        http
          .request(
            { host: "127.0.0.1", port, path: "/api/library-tool", method: "POST", headers: token ? { authorization: `Bearer ${token}` } : {} },
            (res) => {
              res.resume();
              resolve(res.statusCode ?? 0);
            },
          )
          .on("error", reject)
          .end(JSON.stringify({ tool: "library_search", input: {} }));
      });
    expect(await call()).toBe(403);
    expect(await call("guess")).toBe(403);
    expect(await call("secret")).toBe(200);
  });
});
