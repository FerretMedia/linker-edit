import { once } from "node:events";
import * as http from "node:http";
import type { AddressInfo } from "node:net";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { WebSocket, WebSocketServer } from "ws";
import type { Computer, LinkedComputer } from "../shared/protocol.ts";
import { createMachines, machineLocation } from "./machines.ts";

const linked = (id: string, name: string, url: string | null, tailnetUrl: string | null = null, online: boolean | null = true): LinkedComputer => ({
  id,
  name,
  url,
  tailnetUrl,
  online,
  linkedAt: "",
  lastSeenAt: null,
});

const listen = async (server: http.Server) => {
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  return (server.address() as AddressInfo).port;
};

describe("machineLocation", () => {
  it.each([
    ["/there?x=1", "/machines/office/there?x=1"],
    ["https://edit-1.linker.example/there#a", "/machines/office/there#a"],
    ["https://elsewhere.example/there", "https://elsewhere.example/there"],
    ["//elsewhere.example/there", "//elsewhere.example/there"],
  ])("%s → %s", (location, expected) => {
    expect(machineLocation(location, "https://edit-1.linker.example", "/machines/office")).toBe(expected);
  });
});

describe("the machine list", () => {
  it("lists this computer first, then each other computer once, the direct way first", async () => {
    let clock = 0;
    const asked: boolean[] = [];
    const machines = createMachines({
      name: "Studio",
      version: "2026.11.23",
      linked: async (fresh) => {
        asked.push(fresh);
        return [
          linked("both", "Office", "https://edit-1.linker.example", "https://office.tail1.ts.net:8443"),
          linked("tunnel", "Laptop", "https://edit-2.linker.example"),
          linked("away", "Away", "https://edit-3.linker.example", null, false),
          // Linked for Tailscale only, and Tailscale doesn't find it: no way there.
          linked("gone", "Gone", null, "https://gone.tail1.ts.net:8443"),
          // Its Tailscale address doesn't answer: the tunnel instead.
          linked("quiet", "Quiet", "https://edit-4.linker.example", "https://quiet.tail1.ts.net:8443"),
        ];
      },
      tailscale: async (): Promise<Computer[]> => [
        { name: "Office PC", url: "https://office.tail1.ts.net:8443", version: "2" },
        { name: "Den", url: "https://den.tail1.ts.net:8443", version: "3" },
      ],
      probe: async (url) => (url.includes("edit-2") || url.includes("edit-4") ? { name: "", version: "1" } : null),
      sessionOn: async () => null,
      now: () => clock,
    });
    expect((await machines.list()).machines).toEqual([
      { id: "this", name: "Studio", version: "2026.11.23", via: "this", online: true, base: "" },
      { id: "both", name: "Office PC", version: "2", via: "tailscale", online: true, base: "/machines/both" },
      { id: "tunnel", name: "Laptop", version: "1", via: "linker", online: true, base: "/machines/tunnel" },
      { id: "away", name: "Away", version: "", via: "linker", online: false, base: "/machines/away" },
      { id: "quiet", name: "Quiet", version: "1", via: "linker", online: true, base: "/machines/quiet" },
      { id: "tailscale-den.tail1.ts.net", name: "Den", version: "3", via: "tailscale", online: true, base: "/machines/tailscale-den.tail1.ts.net" },
    ]);
    // Kept for 30 seconds, unless a fresh look is asked for.
    await machines.list();
    expect(asked).toEqual([false]);
    await machines.list(true);
    expect(asked).toEqual([false, true]);
    clock = 30_000;
    await machines.list();
    expect(asked).toEqual([false, true, false]);
  });
});

describe("passing requests on", () => {
  /** The other computer: lets in its own session, or a request without one (as over Tailscale). */
  let valid = "s1";
  const seen: { method: string; url: string; headers: http.IncomingHttpHeaders; body: string }[] = [];
  const upgrades: { url: string; headers: http.IncomingHttpHeaders }[] = [];
  const closedThere: Promise<unknown>[] = [];
  const allowed = (headers: http.IncomingHttpHeaders) => headers.authorization === undefined || headers.authorization === `Bearer ${valid}`;
  const other = http.createServer((req, res) => {
    let body = "";
    req.on("data", (chunk) => (body += chunk));
    req.on("end", () => {
      seen.push({ method: req.method ?? "", url: req.url ?? "", headers: req.headers, body });
      if (!allowed(req.headers)) res.writeHead(401).end();
      else if (req.url === "/go") res.writeHead(302, { location: "/there" }).end();
      else if (req.url === "/api/echo") res.writeHead(200, { "content-type": "application/json" }).end(JSON.stringify({ body }));
      else res.writeHead(206, { "content-range": "bytes 0-1/10", "set-cookie": "linker_edit_session=x; Path=/" }).end("01");
    });
  });
  const otherSockets = new WebSocketServer({ noServer: true });
  other.on("upgrade", (req, socket, head) => {
    upgrades.push({ url: req.url ?? "", headers: req.headers });
    if (!allowed(req.headers)) {
      socket.end("HTTP/1.1 401 Unauthorized\r\nConnection: close\r\n\r\n");
      return;
    }
    otherSockets.handleUpgrade(req, socket, head, (ws) => {
      closedThere.push(once(ws, "close"));
      ws.on("message", (data) => ws.send(`echo ${String(data)}`));
      ws.send("hello");
    });
  });

  let signedIn = true;
  let issued = 0;
  const askedAt: string[] = [];
  let port: number;
  let proxy: http.Server;

  beforeAll(async () => {
    const otherPort = await listen(other);
    const dead = http.createServer();
    const deadPort = await listen(dead);
    dead.close();
    const machines = createMachines({
      name: "Studio",
      version: "1",
      linked: async () => [
        linked("office", "Office", `http://127.0.0.1:${otherPort}`),
        // Its Tailscale address stopped answering; its tunnel still does.
        linked("both", "Both", `http://localhost:${otherPort}`, `http://127.0.0.1:${deadPort}`),
        linked("away", "Away", `http://localhost:${deadPort}`),
      ],
      tailscale: async () => [
        { name: "Both", url: `http://127.0.0.1:${deadPort}`, version: "1" },
        { name: "Den", url: `http://localhost:${otherPort}`, version: "1" },
      ],
      probe: async () => ({ name: "", version: "1" }),
      sessionOn: async (url) => {
        askedAt.push(url);
        return signedIn ? { session: `s${++issued}`, expiresAt: "2099-01-01T00:00:00.000Z" } : null;
      },
    });
    proxy = http.createServer((req, res) => void machines.proxy(req, res));
    proxy.on("upgrade", (req, socket, head) => void machines.upgrade(req, socket, head));
    port = await listen(proxy);
  });

  afterAll(() => {
    for (const server of [proxy, other]) {
      server.closeAllConnections();
      server.close();
    }
  });

  function request(method: string, urlPath: string, headers: http.OutgoingHttpHeaders = {}, body?: string) {
    return new Promise<{ status: number; body: string; headers: http.IncomingHttpHeaders }>((resolve, reject) => {
      http
        .request({ host: "127.0.0.1", port, path: urlPath, method, headers }, (res) => {
          let text = "";
          res.on("data", (chunk) => (text += chunk));
          res.on("end", () => resolve({ status: res.statusCode ?? 0, body: text, headers: res.headers }));
        })
        .on("error", reject)
        .end(body);
    });
  }

  it("signs in with this computer's session, never the page's origin, cookies or token", async () => {
    const page = { origin: `http://127.0.0.1:${port}`, referer: `http://127.0.0.1:${port}/`, cookie: "a=b", authorization: "Bearer page" };
    const clip = await request("GET", "/machines/office/clip.mp4?t=1", { ...page, range: "bytes=0-1" });
    expect(clip).toMatchObject({ status: 206, body: "01", headers: { "content-range": "bytes 0-1/10" } });
    expect(clip.headers["set-cookie"]).toBeUndefined();
    const there = seen.at(-1)!;
    expect(there.url).toBe("/clip.mp4?t=1");
    expect(there.headers).toMatchObject({ authorization: "Bearer s1", range: "bytes=0-1" });
    expect(there.headers).not.toHaveProperty("origin");
    expect(there.headers).not.toHaveProperty("referer");
    expect(there.headers).not.toHaveProperty("cookie");
    // A redirect stays on that computer.
    expect((await request("GET", "/machines/office/go")).headers.location).toBe("/machines/office/there");
    expect(askedAt).toHaveLength(1);
  });

  it("gets a new session when the other computer signed this one out, and sends the request again", async () => {
    valid = `s${issued + 1}`;
    const echoed = await request("POST", "/machines/office/api/echo", { "content-type": "application/json" }, '{"a":1}');
    expect(echoed).toMatchObject({ status: 200, body: JSON.stringify({ body: '{"a":1}' }) });
    expect(seen.slice(-2).map((entry) => [entry.headers.authorization, entry.body])).toEqual([
      [`Bearer s${issued - 1}`, '{"a":1}'],
      [`Bearer ${valid}`, '{"a":1}'],
    ]);
  });

  it("goes over Tailscale without a session, and through the tunnel when Tailscale stops answering", async () => {
    expect((await request("GET", "/machines/tailscale-localhost/clip")).status).toBe(206);
    expect(seen.at(-1)!.headers).not.toHaveProperty("authorization");
    valid = `s${issued + 1}`;
    expect((await request("GET", "/machines/both/clip")).status).toBe(206);
    expect(seen.at(-1)!.headers.authorization).toBe(`Bearer ${valid}`);
    expect(askedAt.at(-1)).toMatch(/^http:\/\/localhost:/);
  });

  it("says in words why a computer can't be reached", async () => {
    const unknown = await request("GET", "/machines/nope/api/hello");
    expect(unknown.status).toBe(404);
    signedIn = false;
    const signedOut = await request("GET", "/machines/away/api/hello");
    expect(signedOut).toMatchObject({ status: 401, body: JSON.stringify({ error: "Sign in with Linker on this computer to reach Away." }) });
    signedIn = true;
    const off = await request("GET", "/machines/away/api/hello");
    expect(off.status).toBe(502);
    expect(JSON.parse(off.body).error).toContain("Couldn't reach Away");
  });

  it("joins the window's socket to the other computer's, with a new session when it needs one", async () => {
    valid = `s${issued + 1}`;
    const page = new WebSocket(`ws://127.0.0.1:${port}/machines/office/ws?meta=patch`, ["linker-edit"], { origin: `http://127.0.0.1:${port}` });
    const [hello] = (await once(page, "message")) as [Buffer];
    expect(String(hello)).toBe("hello");
    expect(page.protocol).toBe("linker-edit");
    const there = upgrades.at(-1)!;
    expect(there.url).toBe("/ws?meta=patch");
    expect(there.headers.authorization).toBe(`Bearer ${valid}`);
    expect(there.headers).not.toHaveProperty("sec-websocket-protocol");
    expect(there.headers).not.toHaveProperty("origin");
    expect(upgrades.at(-2)!.headers.authorization).not.toBe(`Bearer ${valid}`);
    page.send("hi");
    const [echo] = (await once(page, "message")) as [Buffer];
    expect(String(echo)).toBe("echo hi");
    page.close();
    await closedThere.at(-1);
  });

  it("closes the window's socket when the other computer closes its own", async () => {
    const page = new WebSocket(`ws://127.0.0.1:${port}/machines/tailscale-localhost/ws`);
    await once(page, "message");
    // Over Tailscale the other computer lets its owner's pages in by their origin: its own address.
    expect(upgrades.at(-1)!.headers.origin).toMatch(/^http:\/\/localhost:/);
    const closed = once(page, "close");
    for (const client of otherSockets.clients) client.close(4000, "bye");
    const [code] = (await closed) as [number];
    expect(code).toBe(4000);
  });
});
